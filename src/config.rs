//! 应用配置:工作目录 / 订阅地址 / 代理模式 / 节点选中,持久化到磁盘。
//!
//! 内核 FFI 化后,控制器地址/secret/mihomo_path 均已删除 —— mihomo 是进程内
//! 静态库,不再需要外部控制平面。`work_dir` 仅作"自定义数据目录"的 UI 覆盖项。
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AppConfig {
    /// mihomo 工作目录(-d);留空 = 用 [`crate::bootstrap::data_dir`] 的托管路径
    #[serde(default)]
    pub work_dir: String,
    /// 订阅(节点配置)URL,首启与"更新订阅"时下载为 config.yaml
    #[serde(default)]
    pub subscription_url: String,
    /// 代理模式:rule / global / direct;启动后 apply_config 时下发给 tunnel
    #[serde(default)]
    pub mode: String,
    /// TUN 用户期望(实际是否生效仍读 TunState;持久化只为重启恢复)
    #[serde(default)]
    pub tun_enable: bool,
    /// TUN 栈(system/gvisor/mixed);macOS 建议 system,Windows 建议 wintun
    #[serde(default)]
    pub tun_stack: String,
    /// 日志级别(debug/info/warning/error/silent)
    #[serde(default)]
    pub log_level: String,
    /// 策略组 → 用户手选节点;apply_config 时回写 selector
    #[serde(default)]
    pub selected_map: HashMap<String, String>,
}

/// 配置文件路径:`<config_dir>/proxy-zms/config.json`
fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("proxy-zms").join("config.json"))
}

impl AppConfig {
    /// 从磁盘加载;不存在或解析失败则返回默认配置。
    ///
    /// 老版本残留的 mihomo_path / secret / controller_url 字段会被 serde 忽略
    /// (目标 struct 无对应 key 时丢弃;`serde(default)` 保证新字段缺失也能反序列化)。
    pub fn load() -> Self {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<Self>(&s).ok())
            .unwrap_or_default()
    }

    /// 保存到磁盘,返回是否成功。
    pub fn save(&self) -> std::io::Result<()> {
        let path = config_path()
            .ok_or_else(|| std::io::Error::other("无法定位配置目录"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
    }
}
