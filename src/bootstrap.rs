//! 首次启动引导:数据目录解析 + 订阅 config.yaml 下载/读取。
//!
//! FFI 内核后,**不再有** mihomo 二进制下载 / wintun.dll / external-controller
//! 接管逻辑 —— 内核链入 exe,控制面是进程内函数调用。本模块只剩两件事:
//! - 数据目录(`<config_dir>/proxy-zms/mihomo`)与 config.yaml 路径解析
//! - 首次启动/更新订阅时把订阅 yaml 写盘(后续由 `Kernel::apply_config` 接管)
use std::path::PathBuf;

/// 受本程序托管的 mihomo 工作目录:`<config_dir>/proxy-zms/mihomo`。
/// 与 v0.2.x 相同,老用户升级后 config.yaml / 订阅缓存目录无缝延续。
///
/// ⚠️ 用于内核 home 时必须走 [`effective_data_dir`],让 UI 里的
/// `work_dir` 覆盖生效;本函数只返回默认值。
pub fn data_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("proxy-zms")
        .join("mihomo")
}

/// 用户覆盖优先的工作目录:空/空白 → [`data_dir`] 默认值。
pub fn effective_data_dir(override_dir: &str) -> PathBuf {
    if override_dir.trim().is_empty() {
        data_dir()
    } else {
        PathBuf::from(override_dir)
    }
}

/// config.yaml 的完整路径(默认值;override 走 [`effective_config_path`])。
pub fn config_path() -> PathBuf {
    data_dir().join("config.yaml")
}

/// 与 [`effective_data_dir`] 配对的 config.yaml 路径。
pub fn effective_config_path(override_dir: &str) -> PathBuf {
    effective_data_dir(override_dir).join("config.yaml")
}

/// 无订阅 URL 时的最小兜底配置(仅本地 mixed-port;
/// external-controller / secret 由 Go bridge 在启动时忽略,无需再注入)。
const DEFAULT_BASE: &str = "\
mixed-port: 7890
allow-lan: false
mode: rule
log-level: info
";

/// config.yaml 不存在时写一份兜底;已存在则不动(尊重用户手改/既有订阅)。
pub fn ensure_config(override_dir: &str) -> Result<(), String> {
    let dir = effective_data_dir(override_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = effective_config_path(override_dir);
    if !path.exists() {
        std::fs::write(&path, DEFAULT_BASE).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 只下载订阅文本(不做任何改写/接管;控制权在 Go bridge,不会被订阅劫持)。
pub async fn fetch_config(url: &str) -> Result<String, String> {
    reqwest::Client::new()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("请求失败:{e}"))?
        .error_for_status()
        .map_err(|e| format!("订阅返回错误:{e}"))?
        .text()
        .await
        .map_err(|e| format!("读取订阅失败:{e}"))
}

/// 下载订阅并落盘为 config.yaml(`subscription_url` 为空时退化为 ensure_config)。
pub async fn write_subscription(url: &str, override_dir: &str) -> Result<(), String> {
    let text = if url.trim().is_empty() {
        DEFAULT_BASE.to_string()
    } else {
        fetch_config(url).await?
    };
    let dir = effective_data_dir(override_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(effective_config_path(override_dir), text).map_err(|e| e.to_string())
}

/// 读回当前 config.yaml 文本(喂给 `Kernel::apply_config` 的 source)。
pub fn read_config(override_dir: &str) -> Result<String, String> {
    let path = effective_config_path(override_dir);
    if !path.exists() {
        ensure_config(override_dir)?;
    }
    std::fs::read_to_string(&path).map_err(|e| format!("读 config.yaml 失败:{e}"))
}
