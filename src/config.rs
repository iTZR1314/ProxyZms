//! 应用配置:mihomo 路径 / 工作目录 / 控制器地址 / secret,持久化到磁盘。
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    /// mihomo 可执行文件路径
    pub mihomo_path: String,
    /// mihomo 工作目录(-d),内含 config.yaml
    pub work_dir: String,
    /// External Controller 地址
    pub controller_url: String,
    /// 控制器 secret(可为空)
    pub secret: String,
    /// 订阅(节点配置)URL,首启与"更新订阅"时下载为 config.yaml
    #[serde(default)]
    pub subscription_url: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            // 留空表示使用本程序托管的二进制(见 bootstrap)
            mihomo_path: String::new(),
            work_dir: String::new(),
            controller_url: "http://127.0.0.1:9091".to_string(),
            secret: String::new(),
            subscription_url: String::new(),
        }
    }
}

/// 配置文件路径:`<config_dir>/proxy-zms/config.json`
fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("proxy-zms").join("config.json"))
}

impl AppConfig {
    /// 从磁盘加载;不存在或解析失败则返回默认配置。
    pub fn load() -> Self {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
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
