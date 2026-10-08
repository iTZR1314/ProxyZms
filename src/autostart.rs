//! 系统级开机自启动。**OS 是单一真相**(plist / regkey 存在与否),
//! 不进 [`crate::config::AppConfig`] —— 避免"配置说 on,文件却被手删"的不一致。
//!
//! - **macOS**:写 `~/Library/LaunchAgents/<bundle_id>.plist`,`RunAtLoad=true`;
//!   下次登录由 launchd 自动加载。关闭 = 删文件。
//!   ⚠️ 光有 plist 不够:launchd 的 disabled 覆盖表优先于 plist,所以开启时还要
//!   `launchctl enable`,`is_enabled` 也要同时看这张表,否则勾选框会显示"开"但实际不生效。
//! - **Windows**:`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`,
//!   `reg add/delete/query`。沿用 `mihomo/process` 的 spawn_blocking + Command 模式,
//!   不引 `winreg` 依赖。
//!   ⚠️ app 因 manifest 是 `requireAdministrator`,登录时会触发 UAC 弹窗
//!   —— 现阶段接受;真嫌烦可改 Task Scheduler `/rl highest`。
//! - **其它平台**:不支持,UI 应把开关 disabled。
//!
//! 自启动项都带 [`LAUNCH_ARG`],程序靠它分辨"登录时被系统拉起"和"用户手动打开":
//! 前者不显示主窗口(macOS 同时收起 Dock 图标,只留托盘)并自动开 TUN,后者行为不变。
//! 旧版写入的启动项不带该参数,重新勾选一次才会生效。

/// 自启动项附带的启动参数,见 [`launched_at_login`]。
pub const LAUNCH_ARG: &str = "--autostart";

#[allow(dead_code)]
const LAUNCH_AGENT_LABEL: &str = "top.zhoumaosen.proxyzms";
#[allow(dead_code)]
const WINDOWS_RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[allow(dead_code)]
const WINDOWS_RUN_VALUE: &str = "ProxyZms";

// ─────────────────────────────────────────────────────────────────────────────
// macOS

#[cfg(target_os = "macos")]
mod imp {
    use super::{LAUNCH_AGENT_LABEL, LAUNCH_ARG};
    use std::path::PathBuf;
    use std::process::Command;

    extern "C" {
        fn getuid() -> u32;
    }

    fn plist_path() -> Result<PathBuf, String> {
        let home = dirs::home_dir().ok_or_else(|| "无法定位用户主目录".to_string())?;
        Ok(home
            .join("Library/LaunchAgents")
            .join(format!("{LAUNCH_AGENT_LABEL}.plist")))
    }

    /// 本用户 GUI 域,如 `gui/501`。
    fn gui_domain() -> String {
        format!("gui/{}", unsafe { getuid() })
    }

    /// launchd 的 disabled 覆盖表(`launchctl disable`、系统设置 → 登录项里关掉开关都会写入)
    /// 优先级高于 plist:label 被标成 disabled 时,plist 再正确、`RunAtLoad` 再为 true 也不会被加载。
    fn is_disabled_in_launchd() -> bool {
        let Ok(out) = Command::new("launchctl")
            .args(["print-disabled", &gui_domain()])
            .output()
        else {
            return false;
        };
        let needle = format!("\"{LAUNCH_AGENT_LABEL}\" =>");
        String::from_utf8_lossy(&out.stdout).lines().any(|line| {
            // 新系统写 disabled/enabled,旧系统写 true/false
            line.trim()
                .strip_prefix(&needle)
                .is_some_and(|v| matches!(v.trim(), "disabled" | "true"))
        })
    }

    pub fn is_enabled() -> bool {
        plist_path().map(|p| p.exists()).unwrap_or(false) && !is_disabled_in_launchd()
    }

    pub fn set_enabled(enable: bool) -> Result<(), String> {
        let path = plist_path()?;
        if enable {
            let exe = std::env::current_exe()
                .map_err(|e| format!("无法获取当前可执行文件路径: {e}"))?;
            let exe_str = exe
                .to_str()
                .ok_or_else(|| "可执行文件路径含非 UTF-8 字符".to_string())?;
            let plist = render_plist(exe_str);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建 LaunchAgents 目录失败: {e}"))?;
            }
            std::fs::write(&path, plist).map_err(|e| format!("写入 plist 失败: {e}"))?;
            // 清掉 disabled 覆盖,否则 plist 写对了下次登录也不会被加载。
            // 只 enable、不 bootstrap:bootstrap 会因 RunAtLoad 立刻再拉起一份自己。
            let target = format!("{}/{LAUNCH_AGENT_LABEL}", gui_domain());
            let status = Command::new("launchctl")
                .args(["enable", &target])
                .status()
                .map_err(|e| format!("调用 launchctl enable 失败: {e}"))?;
            if !status.success() {
                return Err(format!("launchctl enable 退出码 {status}"));
            }
        } else if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("删除 plist 失败: {e}"))?;
        }
        Ok(())
    }

    fn render_plist(exe_path: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>{arg}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
            label = LAUNCH_AGENT_LABEL,
            exe = xml_escape(exe_path),
            arg = LAUNCH_ARG,
        )
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn plist_passes_launch_arg_after_escaped_exe() {
            let plist = render_plist("/Applications/A & B.app/Contents/MacOS/x");
            let exe = plist.find("A &amp; B").expect("exe 路径应转义后写入");
            let arg = plist.find(LAUNCH_ARG).expect("应带自启动参数");
            assert!(exe < arg, "参数必须排在可执行文件之后");
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Windows

#[cfg(target_os = "windows")]
mod imp {
    use super::{LAUNCH_ARG, WINDOWS_RUN_KEY, WINDOWS_RUN_VALUE};
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    /// 隐藏 reg.exe 控制台窗口
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn is_enabled() -> bool {
        Command::new("reg")
            .args([
                "query",
                WINDOWS_RUN_KEY,
                "/v",
                WINDOWS_RUN_VALUE,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub fn set_enabled(enable: bool) -> Result<(), String> {
        if enable {
            let exe = std::env::current_exe()
                .map_err(|e| format!("无法获取当前可执行文件路径: {e}"))?;
            let exe_str = exe
                .to_str()
                .ok_or_else(|| "可执行文件路径含非 UTF-8 字符".to_string())?;
            // reg add 用 /d 直接传值;reg.exe 会把字符串原样写入注册表,
            // 加 "" 包裹路径让 Windows 启动器把带空格的路径当成单一可执行文件;
            // 启动参数跟在引号外。
            let value = format!("\"{exe_str}\" {LAUNCH_ARG}");
            let status = Command::new("reg")
                .args([
                    "add",
                    WINDOWS_RUN_KEY,
                    "/v",
                    WINDOWS_RUN_VALUE,
                    "/t",
                    "REG_SZ",
                    "/d",
                    &value,
                    "/f",
                ])
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .map_err(|e| format!("调用 reg add 失败: {e}"))?;
            if !status.success() {
                return Err(format!("reg add 退出码 {status}"));
            }
        } else {
            // delete:不存在的 value 会返回非 0,视为成功(目标态一致)。
            let _ = Command::new("reg")
                .args([
                    "delete",
                    WINDOWS_RUN_KEY,
                    "/v",
                    WINDOWS_RUN_VALUE,
                    "/f",
                ])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 其它平台:不支持

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    pub fn is_enabled() -> bool {
        false
    }

    pub fn set_enabled(_enable: bool) -> Result<(), String> {
        Err("当前平台暂不支持开机自启动".to_string())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 公共 API:UI 只通过这两个函数交互

/// 当前是否已设为开机自启动。同步、廉价(读文件 / 查注册表)。
pub fn is_enabled() -> bool {
    imp::is_enabled()
}

/// 设置开机自启动状态。同步,失败返回 UI 可直接显示的中文错误。
pub fn set_enabled(enable: bool) -> Result<(), String> {
    imp::set_enabled(enable)
}

/// 本次是否由开机自启动拉起(命令行带 [`LAUNCH_ARG`])。
pub fn launched_at_login() -> bool {
    std::env::args().any(|a| a == LAUNCH_ARG)
}

/// 当前平台是否支持自启动开关。
pub fn is_supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}
