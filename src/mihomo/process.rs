//! 本地 mihomo 进程的启停管理。
//!
//! 目标:**主程序不在 → mihomo 内核也不在**。为此分层兜住各种退出路径:
//! - 正常关窗 / panic 栈展开:`Inner` 的 `Drop` 析构时 kill 子进程
//! - Ctrl-C / SIGTERM:`main` 里注册的信号处理器调用 [`kill_tracked`]
//! - 崩溃 / SIGKILL:无法当下挽救,由下次启动的 [`Controller::start`] 清理残留
//!
//! `Child` 句柄存放在 `Arc<Inner>` 中:既能在 UI 的 context 里共享,也是 `Send`,
//! 可在异步事件处理(如更新订阅后重启)里直接调用。启停为同步操作,不跨 await 持锁。
use crate::bootstrap;
use crate::config::AppConfig;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};

/// Windows:给子进程加 CREATE_NO_WINDOW,避免弹出黑色控制台窗口。
#[cfg(windows)]
fn no_window(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(not(windows))]
fn no_window(cmd: &mut Command) -> &mut Command {
    cmd
}

#[derive(Default)]
struct Inner {
    child: Mutex<Option<Child>>,
}

/// 最后一个 `Controller` 克隆被释放(如组件树拆除、launch 返回)时,
/// 析构会终止 mihomo 子进程,确保不留孤儿。
impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        remove_pidfile();
    }
}

#[derive(Clone, Default)]
pub struct Controller {
    inner: Arc<Inner>,
}

impl Controller {
    /// 进程是否仍在运行。会顺带回收已退出的句柄。
    pub fn is_running(&self) -> bool {
        let mut guard = self.inner.child.lock().unwrap();
        match guard.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(_)) => {
                    *guard = None; // 已退出
                    false
                }
                Ok(None) => true, // 仍在运行
                Err(_) => false,
            },
            None => false,
        }
    }

    /// 以 `mihomo -d <work_dir>` 方式启动;若已在运行则不重复启动。
    pub fn start(&self, cfg: &AppConfig) -> std::io::Result<()> {
        if self.is_running() {
            return Ok(());
        }
        // 先清理可能残留的旧实例(覆盖上次崩溃/SIGKILL 的遗留),确保单例
        cleanup_previous(&cfg.work_dir);

        let mut cmd = Command::new(&cfg.mihomo_path);
        if !cfg.work_dir.trim().is_empty() {
            cmd.arg("-d").arg(&cfg.work_dir);
        }
        no_window(&mut cmd);
        let child = cmd.spawn()?;
        write_pidfile(child.id());
        *self.inner.child.lock().unwrap() = Some(child);
        Ok(())
    }

    /// 终止进程并回收。
    pub fn stop(&self) {
        if let Some(mut child) = self.inner.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        remove_pidfile();
    }
}

/// 二进制是否已具备 root 权限(owner=root 且置了 setuid 位)。
/// 满足时,即使由普通用户 spawn,mihomo 也会以 root 运行,从而能创建 TUN 网卡。
#[cfg(unix)]
pub fn is_elevated(path: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path)
        .map(|m| m.uid() == 0 && (m.mode() & 0o4000 != 0))
        .unwrap_or(false)
}

/// Windows:检测当前进程是否以管理员运行(子进程 mihomo 会继承)。
/// 进程内权限不变,故用 net session 探测一次后缓存。
#[cfg(windows)]
pub fn is_elevated(_path: &str) -> bool {
    use std::sync::OnceLock;
    static ELEVATED: OnceLock<bool> = OnceLock::new();
    *ELEVATED.get_or_init(|| {
        // `net session` 仅管理员可成功,常用作提权探测
        let mut cmd = Command::new("net");
        cmd.arg("session")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        no_window(&mut cmd);
        cmd.status().map(|s| s.success()).unwrap_or(false)
    })
}

#[cfg(not(any(unix, windows)))]
pub fn is_elevated(_path: &str) -> bool {
    false
}

/// 通过 macOS 管理员授权弹窗,把 mihomo 二进制设为 setuid-root。
/// 阻塞直到用户在弹窗里输入密码或取消,故应在 `spawn_blocking` 中调用。
#[cfg(target_os = "macos")]
pub fn elevate_binary(path: &str) -> Result<(), String> {
    // AppleScript 的 do shell script ... with administrator privileges 会弹出原生密码框
    let script = format!(
        "do shell script \"chown root:wheel '{p}' && chmod u+s '{p}'\" with administrator privileges",
        p = path
    );
    let status = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()
        .map_err(|e| format!("无法运行 osascript:{e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("授权被取消或失败".to_string())
    }
}

/// Windows:exe 已内嵌 requireAdministrator 清单,双击即以管理员运行,
/// 故运行时无需提权。此函数不会被实际调用(`is_elevated` 恒为 true → 授权按钮隐藏)。
#[cfg(target_os = "windows")]
pub fn elevate_binary(_path: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn elevate_binary(_path: &str) -> Result<(), String> {
    Err("当前平台暂不支持一键提权".to_string())
}

/// 信号处理器(Ctrl-C / SIGTERM)用:根据 pidfile 终止 mihomo。
/// 此时进程即将 `exit`,`Drop` 不会运行,故需在这里显式清理。
pub fn kill_tracked() {
    if let Ok(content) = std::fs::read_to_string(pidfile_path()) {
        if let Ok(pid) = content.trim().parse::<u32>() {
            kill_pid(pid);
        }
    }
    remove_pidfile();
}

fn pidfile_path() -> std::path::PathBuf {
    bootstrap::data_dir().join("mihomo.pid")
}

fn write_pidfile(pid: u32) {
    let _ = std::fs::write(pidfile_path(), pid.to_string());
}

fn remove_pidfile() {
    let _ = std::fs::remove_file(pidfile_path());
}

/// 清理上一次会话残留的 mihomo 进程。
fn cleanup_previous(work_dir: &str) {
    // 1) pidfile 记录的上次 PID
    if let Ok(content) = std::fs::read_to_string(pidfile_path()) {
        if let Ok(pid) = content.trim().parse::<u32>() {
            kill_pid(pid);
        }
    }
    // 2) Unix 上按工作目录兜底扫尾,清掉所有用本目录启动的实例
    #[cfg(unix)]
    if !work_dir.trim().is_empty() {
        let _ = Command::new("pkill").arg("-f").arg(work_dir).status();
    }
    let _ = work_dir; // 非 unix 平台未使用
}

#[cfg(unix)]
fn kill_pid(pid: u32) {
    let _ = Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .status();
}

#[cfg(windows)]
fn kill_pid(pid: u32) {
    let mut cmd = Command::new("taskkill");
    // /T:连同 mihomo 派生的子进程一起 kill,避免 TUN 模式下的辅助进程成为孤儿
    cmd.args(["/F", "/T", "/PID", &pid.to_string()]);
    no_window(&mut cmd);
    let _ = cmd.status();
}
