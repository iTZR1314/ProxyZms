//! 锁屏动作。按平台 cfg 分发,对外只暴露统一的异步 [`lock`] 函数。
//!
//! - **macOS**:`pmset displaysleepnow`(显示器休眠 → 系统要求密码;前提是已开启
//!   "唤醒立即要求密码")。不需要 Accessibility 授权,不会真登出。
//! - **Windows**:`rundll32 user32.dll,LockWorkStation`(等价 Win+L)。
//!
//! 实现风格沿用 ProxyZms 的 [`crate::mihomo::process`]:`std::process::Command`
//! 包在 `tokio::task::spawn_blocking` 里 —— 不引入额外 tokio feature。
//!
//! 这一层故意做得极薄:不判断**什么时候**该锁(那是 Monitor 的事),不持有任何状态。

use std::io;

#[cfg(target_os = "macos")]
pub async fn lock() -> io::Result<()> {
    let status = tokio::task::spawn_blocking(|| {
        std::process::Command::new("pmset")
            .arg("displaysleepnow")
            .status()
    })
    .await
    .map_err(|e| io::Error::other(format!("spawn_blocking 失败: {e}")))??;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "pmset displaysleepnow 退出码 {status}"
        )))
    }
}

#[cfg(target_os = "windows")]
pub async fn lock() -> io::Result<()> {
    let status = tokio::task::spawn_blocking(|| {
        std::process::Command::new("rundll32.exe")
            .arg("user32.dll,LockWorkStation")
            .status()
    })
    .await
    .map_err(|e| io::Error::other(format!("spawn_blocking 失败: {e}")))??;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "rundll32 LockWorkStation 退出码 {status}"
        )))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub async fn lock() -> io::Result<()> {
    Err(io::Error::other(
        "当前平台暂不支持自动锁屏(请在 src/ble_lock/locker.rs 添加支持)",
    ))
}
