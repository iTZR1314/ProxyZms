//! Bluetooth Low Energy 锁屏:扫描附近 BLE 设备 → 监测目标设备 RSSI → 离开范围自动锁屏。
//!
//! 三层契约严格分开,各层互不污染、可独立替换:
//! - [`Scanner`](scanner::Scanner) 只负责蓝牙扫描(产出 `Option<DeviceInfo>`);
//! - [`Monitor`](monitor::Monitor) 只负责 RSSI 累积判断(吃 `Option<i16>`,吐 `PhoneStatus` + `should_lock()`);
//! - [`locker::lock`] 只负责调用系统锁屏 API(平台分发)。
//!
//! 状态机([`BleState`])只描述"模式";实时数据(rssi / 计数 / 历史)放独立 [`BleSession`] 信号,
//! 任意页面通过 `use_context::<BleSession>()` 共享读取。

pub mod config;
pub mod device;
pub mod locker;
pub mod monitor;
pub mod runner;
pub mod scanner;

use dioxus::prelude::*;

pub use config::BleLockConfig;
pub use device::DeviceInfo;
pub use monitor::PhoneStatus;
pub use runner::{supervisor, try_autostart, RSSI_HISTORY_LEN};
pub use scanner::{Scanner, ScannerError};

/// BLE 锁屏的核心模式。绑定信息(`target`)从 [`BleLockConfig`] 读取,**不**在此重复。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BleState {
    /// 未在保护中(无论是否已绑定设备)。
    Idle,
    /// 保护进行中:后台任务正在按节奏扫描并喂 Monitor。
    Watching,
    /// Monitor 判定该锁屏,UI 显示倒计时给用户撤回机会。
    LockPending,
    /// 已经触发锁屏(dormant,需用户手动复位回 Idle 才能再次启用)。
    Locked,
}

impl BleState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "待机",
            Self::Watching => "保护中",
            Self::LockPending => "即将锁屏",
            Self::Locked => "已锁屏",
        }
    }
}

/// BLE 锁屏会话的共享状态。仿照 [`crate::Telemetry`]:Copy 结构,字段都是 [`Signal`],
/// 任意页面 / 任意组件通过 `use_context::<BleSession>()` 读写。
///
/// **设计要点**:`state` 只描述模式,实时数据(rssi / status / 计数 / 历史)放独立信号,
/// 避免每次 RSSI 更新都克隆整个 enum;UI 局部 reactivity 也更精准。
#[derive(Clone, Copy)]
pub struct BleSession {
    pub state: Signal<BleState>,

    pub current_rssi: Signal<Option<i16>>,
    pub current_status: Signal<Option<PhoneStatus>>,
    pub missing_count: Signal<u32>,
    pub rssi_history: Signal<Vec<Option<i16>>>,
    /// Monitor 的 armed 镜像:Watching 进入后是否已扫到过至少一次 Nearby。
    /// UI 用它显示"待定位手机"占位文案,避免误以为正在倒计时锁屏。
    pub armed: Signal<bool>,

    /// 会话号:每次进入 Watching 时 +1,后台任务用它做协作式取消。
    pub session_id: Signal<u64>,
    /// LockPending 期间的剩余毫秒(>0 = 倒计时中)。
    pub cooldown_remaining_ms: Signal<u64>,
    /// LockPending 期间用户按下取消时翻 true,任务下一拍自检退出。
    pub lock_cancel_requested: Signal<bool>,

    /// 最近的错误消息(扫描失败 / 锁屏失败等)。
    pub error_msg: Signal<Option<String>>,
}
