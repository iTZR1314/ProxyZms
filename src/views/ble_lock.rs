//! 蓝牙锁屏 —— 一页搞定绑定 + 保护 + 阈值调节。
//!
//! 设计原则:跨页状态(`BleSession` + `BleLockConfig`)已经在 context 里,
//! 本页只是渲染入口。"绑定设备"与"启动保护"是同一个 feature 的两面,
//! 用 `BleState` + 本地 `ScanUiState` 共同决定哪些段显示。

use std::cmp::Ordering;
use std::time::Duration;

use dioxus::prelude::*;

use crate::ble_lock::{self, BleLockConfig, BleSession, BleState, DeviceInfo, Scanner};
use crate::views::ble_components::{RssiChart, ThresholdControls};

/// 设备发现扫描的窗口长度。比保护期的 1.5 s 长,争取多扫到几台设备。
const DISCOVERY_SCAN_WINDOW: Duration = Duration::from_secs(5);

/// 本页本地的扫描 UI 模式。**仅这一页关心**,不进 `BleSession` context ——
/// 跨页没意义(切到流量页时,"扫描中" 这个 UI 状态没必要保留)。
#[derive(Clone, PartialEq)]
enum ScanUiState {
    /// 不在扫描模式,不显示扫描区。
    Hidden,
    /// 正在扫描。
    Running,
    /// 扫描完成,显示结果表格让用户选。
    Results(Vec<DeviceInfo>),
}

#[component]
pub fn BleLock() -> Element {
    let mut config = use_context::<Signal<BleLockConfig>>();
    let mut ble = use_context::<BleSession>();

    let mut scan_ui = use_signal(|| ScanUiState::Hidden);
    let mut scan_error = use_signal(|| None::<String>);

    let cfg_snapshot = config();
    let state = ble.state.cloned();
    let is_protecting = state != BleState::Idle;

    // ─────────────────────────────────────────────────────────────────────
    // 状态机动作

    // 启动一次设备发现扫描。仅在 Idle 状态下允许 —— 保护进行中扫描会与
    // supervisor 的 Scanner 抢蓝牙适配器。
    let start_scan = move |_| {
        if is_protecting {
            return;
        }
        if matches!(scan_ui(), ScanUiState::Running) {
            return;
        }
        scan_error.set(None);
        scan_ui.set(ScanUiState::Running);

        spawn(async move {
            let outcome = async {
                let scanner = Scanner::new().await?;
                let mut list = scanner.scan_all(DISCOVERY_SCAN_WINDOW).await?;
                list.sort_by(|a, b| match (a.rssi, b.rssi) {
                    (Some(ra), Some(rb)) => rb.cmp(&ra),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                });
                Ok::<_, crate::ble_lock::ScannerError>(list)
            }
            .await;

            match outcome {
                Ok(list) => scan_ui.set(ScanUiState::Results(list)),
                Err(e) => {
                    scan_error.set(Some(format!("{e}")));
                    scan_ui.set(ScanUiState::Hidden);
                }
            }
        });
    };

    let cancel_scan = move |_| {
        scan_ui.set(ScanUiState::Hidden);
    };

    let bind_device = move |name: String| {
        config.write().target = Some(name);
        let _ = config.read().save();
        scan_ui.set(ScanUiState::Hidden);
    };

    let unbind = move |_| {
        if is_protecting {
            return;
        }
        config.write().target = None;
        let _ = config.read().save();
        scan_ui.set(ScanUiState::Hidden);
    };

    let start_protection = move |_| {
        ble.session_id.with_mut(|s| *s += 1);
        ble.error_msg.set(None);
        ble.state.set(BleState::Watching);
    };

    let stop_protection = move |_| {
        ble.session_id.with_mut(|s| *s += 1);
        ble.state.set(BleState::Idle);
        ble.cooldown_remaining_ms.set(0);
    };

    let cancel_lock = move |_| {
        if ble.state.cloned() == BleState::LockPending {
            ble.lock_cancel_requested.set(true);
        }
    };

    let reset_to_idle = move |_| {
        ble.session_id.with_mut(|s| *s += 1);
        ble.state.set(BleState::Idle);
        ble.cooldown_remaining_ms.set(0);
        ble.error_msg.set(None);
    };

    let test_lock = move |_| {
        spawn(async move {
            if let Err(e) = ble_lock::locker::lock().await {
                ble.error_msg.set(Some(format!("锁屏失败: {e}")));
            }
        });
    };

    let on_rssi_change = move |v: i16| {
        config.write().lock_rssi = v;
        let _ = config.read().save();
    };
    let on_limit_change = move |v: u32| {
        config.write().missing_limit = v;
        let _ = config.read().save();
    };

    let has_target = cfg_snapshot.target.is_some();

    // ─────────────────────────────────────────────────────────────────────
    // 渲染

    rsx! {
        div { class: "h-full px-6 md:px-12 py-6 max-w-3xl mx-auto flex flex-col gap-6 overflow-y-auto",

            // ── 顶部:标题 + 状态徽章 ──
            header { class: "border-b-2 border-black pb-4 flex items-baseline justify-between gap-4",
                div {
                    div { class: "text-[11px] uppercase tracking-[0.25em] text-neutral-500", "Bluetooth Lock" }
                    h1 { class: "mt-3 text-4xl font-bold tracking-tighter leading-none", "锁屏" }
                }
                div { class: "text-right",
                    div { class: "text-[10px] uppercase tracking-[0.25em] text-neutral-400", "状态" }
                    div {
                        class: "mt-1 text-lg font-bold tracking-tighter",
                        class: if state == BleState::Idle { "text-neutral-500" } else { "text-[#e3000f]" },
                        "{state.label()}"
                    }
                }
            }

            // ── 01 / 目标设备 ──
            section {
                div { class: "text-[11px] uppercase tracking-[0.2em] text-[#e3000f] border-b border-black pb-2 mb-4",
                    "01 / 目标设备"
                }

                if let Some(name) = cfg_snapshot.target.clone() {
                    div { class: "flex items-baseline justify-between gap-4 flex-wrap",
                        div {
                            div { class: "text-[11px] uppercase tracking-[0.2em] text-neutral-500", "已绑定" }
                            div { class: "mt-1 text-2xl font-bold tracking-tighter", "{name}" }
                        }
                        div { class: "flex gap-3",
                            button {
                                class: "px-4 py-1.5 border border-black text-black text-[11px] uppercase tracking-[0.15em] hover:bg-black hover:text-white disabled:opacity-40 transition-colors",
                                disabled: is_protecting || matches!(scan_ui(), ScanUiState::Running),
                                onclick: start_scan,
                                "更换设备"
                            }
                            button {
                                class: "px-4 py-1.5 border border-[#e3000f] text-[#e3000f] text-[11px] uppercase tracking-[0.15em] hover:bg-[#e3000f] hover:text-white disabled:opacity-40 transition-colors",
                                disabled: is_protecting,
                                onclick: unbind,
                                "解绑"
                            }
                        }
                    }
                    if is_protecting {
                        div { class: "mt-3 text-[11px] uppercase tracking-[0.15em] text-neutral-500",
                            "保护进行中,请先在下方停止保护后再换设备 / 解绑"
                        }
                    }
                } else {
                    div { class: "flex items-center justify-between gap-4",
                        div { class: "text-sm text-neutral-600", "尚未绑定设备" }
                        button {
                            class: "px-6 py-2 bg-black text-white text-sm uppercase tracking-[0.15em] hover:bg-[#e3000f] disabled:opacity-40 transition-colors",
                            disabled: matches!(scan_ui(), ScanUiState::Running),
                            onclick: start_scan,
                            "扫描设备"
                        }
                    }
                }
            }

            // ── 02 / 扫描(仅在 scan_ui != Hidden 时浮出) ──
            match scan_ui() {
                ScanUiState::Hidden => rsx! {},
                ScanUiState::Running => rsx! {
                    section {
                        div { class: "text-[11px] uppercase tracking-[0.2em] text-[#e3000f] border-b border-black pb-2 mb-4",
                            "02 / 扫描"
                        }
                        div { class: "py-4 text-sm text-neutral-600",
                            "正在扫描… (约 {DISCOVERY_SCAN_WINDOW.as_secs()} 秒)"
                        }
                    }
                },
                ScanUiState::Results(devices) => rsx! {
                    section {
                        div { class: "text-[11px] uppercase tracking-[0.2em] text-[#e3000f] border-b border-black pb-2 mb-4 flex items-baseline justify-between",
                            span { "02 / 扫描结果 · {devices.len()} 台" }
                            button {
                                class: "px-3 py-1 text-[10px] uppercase tracking-[0.15em] text-neutral-600 border border-neutral-300 hover:border-black hover:text-black transition-colors",
                                onclick: cancel_scan,
                                "关闭"
                            }
                        }
                        DeviceTable { devices, on_pick: bind_device }
                    }
                },
            }

            // 扫描错误条
            if let Some(e) = scan_error() {
                div { class: "px-4 py-3 border-l-4 border-[#e3000f] bg-neutral-50 text-sm text-neutral-700",
                    "扫描出错:{e}"
                }
            }

            // ── 03 / 控制(仅在已绑定时) ──
            if has_target {
                section {
                    div { class: "text-[11px] uppercase tracking-[0.2em] text-[#e3000f] border-b border-black pb-2 mb-4",
                        "03 / 控制"
                    }
                    div { class: "flex flex-wrap gap-3",
                        match state {
                            BleState::Idle => rsx! {
                                button {
                                    class: "px-8 py-2.5 bg-black text-white text-sm uppercase tracking-[0.15em] hover:bg-[#e3000f] transition-colors",
                                    onclick: start_protection,
                                    "开始保护"
                                }
                            },
                            BleState::Watching => rsx! {
                                button {
                                    class: "px-8 py-2.5 border border-black text-black text-sm uppercase tracking-[0.15em] hover:bg-black hover:text-white transition-colors",
                                    onclick: stop_protection,
                                    "停止保护"
                                }
                            },
                            BleState::LockPending => rsx! {
                                button {
                                    class: "px-8 py-2.5 bg-[#e3000f] text-white text-sm uppercase tracking-[0.15em] hover:bg-black transition-colors",
                                    onclick: cancel_lock,
                                    "取消锁屏 · 剩 {ble.cooldown_remaining_ms.cloned().div_ceil(1000)} 秒"
                                }
                                button {
                                    class: "px-6 py-2.5 border border-neutral-400 text-neutral-700 text-sm uppercase tracking-[0.15em] hover:bg-neutral-100 transition-colors",
                                    onclick: stop_protection,
                                    "停止保护"
                                }
                            },
                            BleState::Locked => rsx! {
                                button {
                                    class: "px-8 py-2.5 bg-black text-white text-sm uppercase tracking-[0.15em] hover:bg-[#e3000f] transition-colors",
                                    onclick: reset_to_idle,
                                    "重置"
                                }
                                span { class: "self-center text-[11px] uppercase tracking-[0.15em] text-neutral-500",
                                    "已锁屏 · 信号回归 {cfg_snapshot.rearm_limit} 拍后自动续保护,或手动重置"
                                }
                            },
                        }
                    }
                }
            }

            // ── 04 / 实时状态(仅在 Watching / LockPending / Locked 时) ──
            if has_target && state != BleState::Idle {
                section {
                    div { class: "text-[11px] uppercase tracking-[0.2em] text-[#e3000f] border-b border-black pb-2 mb-4",
                        "04 / 实时状态"
                    }
                    div { class: "grid grid-cols-3 gap-6 mb-5",
                        div {
                            div { class: "text-[10px] uppercase tracking-[0.2em] text-neutral-500", "RSSI" }
                            div { class: "mt-1 text-3xl font-bold tabular-nums tracking-tighter leading-none",
                                {
                                    ble.current_rssi
                                        .cloned()
                                        .map(|v| format!("{v}"))
                                        .unwrap_or_else(|| "—".into())
                                }
                                span { class: "ml-1 text-[10px] uppercase tracking-[0.2em] text-neutral-400 font-normal",
                                    "dBm"
                                }
                            }
                        }
                        div {
                            div { class: "text-[10px] uppercase tracking-[0.2em] text-neutral-500", "状态" }
                            div { class: "mt-1 text-3xl font-bold tracking-tighter leading-none",
                                {
                                    ble.current_status
                                        .cloned()
                                        .map(|s| s.label().to_string())
                                        .unwrap_or_else(|| "—".into())
                                }
                            }
                        }
                        div {
                            div { class: "text-[10px] uppercase tracking-[0.2em] text-neutral-500", "丢失计数" }
                            div { class: "mt-1 text-3xl font-bold tabular-nums tracking-tighter leading-none",
                                "{ble.missing_count.cloned()}"
                                span { class: "ml-1 text-[10px] uppercase tracking-[0.2em] text-neutral-400 font-normal",
                                    "/ {cfg_snapshot.missing_limit}"
                                }
                            }
                        }
                    }
                    div {
                        div { class: "flex items-baseline justify-between text-[10px] uppercase tracking-[0.2em] text-neutral-500 mb-2",
                            span { "RSSI · 过去 60 秒" }
                            span { class: "font-mono", "阈值 {cfg_snapshot.lock_rssi} dBm" }
                        }
                        RssiChart {
                            history: ble.rssi_history.cloned(),
                            threshold: cfg_snapshot.lock_rssi,
                        }
                    }
                }
            }

            // ── 05 / 阈值微调(仅在 Idle 时显示 —— 避免保护中误改阈值导致行为飘) ──
            if has_target && state == BleState::Idle {
                section {
                    div { class: "text-[11px] uppercase tracking-[0.2em] text-[#e3000f] border-b border-black pb-2 mb-4",
                        "05 / 阈值微调"
                    }
                    ThresholdControls {
                        lock_rssi: cfg_snapshot.lock_rssi,
                        missing_limit: cfg_snapshot.missing_limit,
                        on_rssi_change,
                        on_limit_change,
                    }
                    div { class: "mt-3 text-[10px] uppercase tracking-[0.2em] text-neutral-400",
                        "改动自动落盘 · 开始保护后此段隐藏,防止运行中误改阈值"
                    }
                }
            }

            // ── 全局错误浮条(BleSession.error_msg) ──
            if let Some(e) = ble.error_msg.cloned() {
                div { class: "px-4 py-3 border-l-4 border-[#e3000f] bg-neutral-50 text-sm text-neutral-700",
                    "{e}"
                }
            }

            // ── 底部诊断:测试锁屏 ──
            div { class: "mt-auto pt-4 border-t border-neutral-200 flex items-center justify-between",
                span { class: "text-[10px] uppercase tracking-[0.2em] text-neutral-400", "诊断" }
                button {
                    class: "px-4 py-1.5 border border-neutral-300 text-neutral-700 text-[11px] uppercase tracking-[0.15em] hover:border-[#e3000f] hover:text-[#e3000f] transition-colors",
                    onclick: test_lock,
                    "立即锁屏(测试)"
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 扫描结果表格 —— 私有子组件

#[component]
fn DeviceTable(devices: Vec<DeviceInfo>, on_pick: EventHandler<String>) -> Element {
    if devices.is_empty() {
        return rsx! {
            div { class: "py-6 text-sm text-neutral-500",
                "扫描完成,但未发现任何 BLE 设备 —— 试试把目标设备靠近本机后重试。"
            }
        };
    }

    rsx! {
        div { class: "border border-black/15 divide-y divide-black/10",
            div { class: "grid grid-cols-[80px_1fr_auto] gap-3 px-3 py-2 text-[10px] uppercase tracking-[0.2em] text-neutral-500 bg-neutral-50",
                span { "RSSI" }
                span { "Name" }
                span { "" }
            }
            for device in devices.iter() {
                DeviceRow {
                    key: "{device.id}",
                    device: device.clone(),
                    on_pick,
                }
            }
        }
    }
}

#[component]
fn DeviceRow(device: DeviceInfo, on_pick: EventHandler<String>) -> Element {
    let rssi_display = device
        .rssi
        .map(|v| format!("{v} dBm"))
        .unwrap_or_else(|| "—".into());
    let name_display = device
        .local_name
        .as_deref()
        .unwrap_or("<匿名>")
        .to_string();
    let bindable_name = device.local_name.clone();

    rsx! {
        div { class: "grid grid-cols-[80px_1fr_auto] gap-3 px-3 py-2 items-center text-sm",
            span { class: "font-mono tabular-nums text-neutral-700", "{rssi_display}" }
            span { class: "truncate", "{name_display}" }
            if let Some(name) = bindable_name {
                button {
                    class: "px-3 py-1 bg-black text-white text-[11px] uppercase tracking-[0.15em] hover:bg-[#e3000f] transition-colors",
                    onclick: move |_| on_pick.call(name.clone()),
                    "绑定"
                }
            } else {
                span { class: "text-[10px] uppercase tracking-[0.15em] text-neutral-400",
                    "无名 不可绑"
                }
            }
        }
    }
}
