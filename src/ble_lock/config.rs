//! BLE 锁屏的持久化配置:绑定设备名 + 信号阈值。
//!
//! 文件路径与主配置同目录:`<config_dir>/proxy-zms/ble_lock.json`。
//! 与 [`crate::config::AppConfig`] 完全平行,互不影响。
//!
//! 设计原则与 `AppConfig` 一致:
//! - **读永不 panic** —— 文件缺失 / 解析失败一律退化为 [`Default`];
//! - **写显式返回 io::Result** —— 调用方决定是否在 UI 提示。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 应用的全部 BLE 持久化状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BleLockConfig {
    /// 当前绑定的设备名(BLE `local_name`)。`None` = 未绑定。
    #[serde(default)]
    pub target: Option<String>,
    /// RSSI 阈值(dBm,负值)。`rssi >= lock_rssi` 视为 Nearby。
    #[serde(default = "default_lock_rssi")]
    pub lock_rssi: i16,
    /// 连续"非 Nearby"次数上限,达到即触发 should_lock。
    #[serde(default = "default_missing_limit")]
    pub missing_limit: u32,
    /// Locked 状态下,连续"Nearby"次数达到该值即自动 re-arm 回 Watching。
    /// 默认 3 ≈ 6 秒稳定回归,抗抖动。
    #[serde(default = "default_rearm_limit")]
    pub rearm_limit: u32,
    /// 打开应用时,若已绑定设备且手机在阈值内,自动开启保护。
    /// 默认 true —— 用户的初衷一般就是"不想每次手动点启动"。
    #[serde(default = "default_auto_protect_on_launch")]
    pub auto_protect_on_launch: bool,
}

fn default_lock_rssi() -> i16 {
    -75
}
fn default_missing_limit() -> u32 {
    5
}
fn default_rearm_limit() -> u32 {
    3
}
fn default_auto_protect_on_launch() -> bool {
    true
}

impl Default for BleLockConfig {
    fn default() -> Self {
        Self {
            target: None,
            lock_rssi: default_lock_rssi(),
            missing_limit: default_missing_limit(),
            rearm_limit: default_rearm_limit(),
            auto_protect_on_launch: default_auto_protect_on_launch(),
        }
    }
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("proxy-zms").join("ble_lock.json"))
}

impl BleLockConfig {
    /// 从磁盘加载;不存在或解析失败则返回默认配置。
    pub fn load() -> Self {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// 保存到磁盘,返回是否成功。
    pub fn save(&self) -> std::io::Result<()> {
        let path = config_path().ok_or_else(|| std::io::Error::other("无法定位配置目录"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
    }
}
