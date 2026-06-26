//! 后台监听循环 + supervisor。架构上分两层:
//!
//! - [`run_watch_session`]:一次完整的保护会话(BleState 进入 Watching 到 Locked
//!   或被外部 Idle 打断)。内部含 scan 生产者 + Monitor 消费者 + LockPending 冷静期 +
//!   真锁屏调用。
//! - [`supervisor`]:App 级永久 task,每 200 ms 巡视 BleState,看到 Watching 且
//!   `session_id` 是新的就启动一次 [`run_watch_session`]。
//!
//! 二者都通过 [`BleSession`] context 与 UI 通讯,没有自己的状态。

use std::time::{Duration, Instant};

use dioxus::prelude::*;
use tokio::sync::mpsc;

use super::config::BleLockConfig;
use super::device::DeviceMatcher;
use super::locker;
use super::monitor::{Monitor, MonitorConfig};
use super::scanner::Scanner;
use super::{BleSession, BleState};

/// 采集节拍。0.5 Hz = 每 2 秒一笔 RSSI。
pub const WATCH_TICK_INTERVAL: Duration = Duration::from_secs(2);
/// 每拍扫描接收窗口。必须 < WATCH_TICK_INTERVAL,留余量给 stop_scan + 调度延迟。
pub const WATCH_SCAN_WINDOW: Duration = Duration::from_millis(1500);
/// RSSI 滚动历史窗口长度。30 × 2 s = 60 秒。
pub const RSSI_HISTORY_LEN: usize = 30;
/// 锁屏前的冷静期。Monitor 判定 should_lock 后,UI 给用户撤回的时间窗口。
pub const LOCK_COOLDOWN: Duration = Duration::from_secs(3);

/// 跑一次完整保护会话。从 BleState=Watching 开始,结束时 BleState 已经
/// 切到 Locked、Idle 或 Watching(被取消锁屏后又继续)之一。
pub async fn run_watch_session(
    mut ble: BleSession,
    config: Signal<BleLockConfig>,
    target: String,
    my_session_id: u64,
) {
    // 1. 拉起 Scanner(可能失败:蓝牙关闭 / 未授权)
    let scanner = match Scanner::new().await {
        Ok(s) => s,
        Err(e) => {
            ble.error_msg.set(Some(format!("{e}")));
            ble.state.set(BleState::Idle);
            return;
        }
    };
    let matcher = DeviceMatcher::Name(target.clone());

    // 2. Monitor + 初始 config 快照(后续支持热同步)
    let mut cfg = { config.read().clone() };
    let mut monitor = Monitor::new(MonitorConfig {
        lock_rssi: cfg.lock_rssi,
        missing_limit: cfg.missing_limit,
    });

    // 3. 重置 UI 信号
    ble.current_rssi.set(None);
    ble.current_status.set(None);
    ble.missing_count.set(0);
    ble.rssi_history.set(Vec::new());
    ble.error_msg.set(None);

    // 4. 生产者:开独立 task 跑持续采集,通过 mpsc channel 推过来
    let (tx, mut rx) = mpsc::channel(4);
    spawn(scanner.drive_watch_stream(
        matcher,
        WATCH_TICK_INTERVAL,
        WATCH_SCAN_WINDOW,
        tx,
    ));

    // 5. 消费循环
    while let Some(scan_result) = rx.recv().await {
        // session 检测:用户停了保护或换了会话 → 立即退出,不污染新会话。
        if ble.session_id.cloned() != my_session_id {
            return;
        }

        let rssi_input: Option<i16> = match scan_result {
            Ok(Some(info)) => {
                ble.error_msg.set(None);
                info.rssi
            }
            Ok(None) => {
                ble.error_msg.set(None);
                None
            }
            Err(e) => {
                // 扫描错误:**不**喂 Monitor,只上报。否则系统错被错算成"丢失"。
                ble.error_msg.set(Some(format!("{e}")));
                continue;
            }
        };

        // 热同步配置:UI 拖了阈值滑块就立刻应用。
        let live_cfg = { config.read().clone() };
        if live_cfg.lock_rssi != cfg.lock_rssi || live_cfg.missing_limit != cfg.missing_limit {
            monitor.set_config(MonitorConfig {
                lock_rssi: live_cfg.lock_rssi,
                missing_limit: live_cfg.missing_limit,
            });
            cfg = live_cfg;
        }

        // ★ UI 唯一接触锁屏决策逻辑的两行 ★
        let status = monitor.update(rssi_input);
        let lock_now = monitor.should_lock();

        ble.current_rssi.set(rssi_input);
        ble.current_status.set(Some(status));
        ble.missing_count.set(monitor.missing_count());

        ble.rssi_history.with_mut(|h| {
            if h.len() >= RSSI_HISTORY_LEN {
                h.remove(0);
            }
            h.push(rssi_input);
        });

        if lock_now {
            // 进入冷静期。
            ble.state.set(BleState::LockPending);
            ble.lock_cancel_requested.set(false);

            let cooldown_start = Instant::now();
            let mut cancelled = false;
            loop {
                let elapsed = cooldown_start.elapsed();
                if elapsed >= LOCK_COOLDOWN {
                    break;
                }
                ble.cooldown_remaining_ms
                    .set(LOCK_COOLDOWN.saturating_sub(elapsed).as_millis() as u64);
                // 冷静期内用户点了"停止保护" —— session 已变。
                if ble.session_id.cloned() != my_session_id {
                    return;
                }
                if ble.lock_cancel_requested.cloned() {
                    cancelled = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            ble.cooldown_remaining_ms.set(0);

            if cancelled {
                // 用户取消 —— Monitor 完全重置,UI 回到 Watching。
                monitor.reset();
                ble.current_rssi.set(None);
                ble.current_status.set(None);
                ble.missing_count.set(0);
                ble.lock_cancel_requested.set(false);
                ble.state.set(BleState::Watching);
                continue;
            }

            // 冷静期过完没人取消 —— 真锁。
            ble.state.set(BleState::Locked);
            if let Err(e) = locker::lock().await {
                ble.error_msg.set(Some(format!("锁屏失败: {e}")));
            }
            return;
        }
    }
}

/// App 级永久 task:巡视 BleState,每次 Watching 用新 session_id 时启动一次会话。
///
/// 派给 `use_future`,与 ProxyZms 既有的 `/connections` 长连和 `/configs` 轮询并列。
pub async fn supervisor(mut ble: BleSession, config: Signal<BleLockConfig>) {
    let mut last_handled_session: u64 = 0;
    loop {
        let current = ble.state.cloned();
        if current == BleState::Watching {
            let sid = ble.session_id.cloned();
            if sid != last_handled_session {
                last_handled_session = sid;
                let target = { config.read().target.clone() };
                let Some(target) = target else {
                    ble.error_msg
                        .set(Some("未绑定设备,无法启动保护".to_string()));
                    ble.state.set(BleState::Idle);
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                };
                run_watch_session(ble, config, target, sid).await;
            } else {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        } else {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}
