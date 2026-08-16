//! 系统深浅色探测 —— 只用来定窗口的**首帧底色**。
//!
//! 界面本身的深色由 CSS 的 `prefers-color-scheme` 负责(见 `assets/main.css`),
//! webview 会自己跟随系统实时切换,Rust 这边不参与、也不缓存状态。
//! 但窗口底色是 tao 建窗时定死的:深色系统下若仍传白色,启动瞬间会白闪一下,
//! 所以这里在建窗前探一次。探不出来一律当浅色(维持历史行为)。

/// 系统当前是否为深色外观。
#[cfg(target_os = "macos")]
pub fn system_is_dark() -> bool {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};

    // 浅色时 AppleInterfaceStyle 这个键根本不存在,深色时值为 "Dark" —— 有键即深色。
    unsafe {
        let defaults: *mut Object = msg_send![class!(NSUserDefaults), standardUserDefaults];
        if defaults.is_null() {
            return false;
        }
        let key: *mut Object = msg_send![
            class!(NSString),
            stringWithUTF8String: c"AppleInterfaceStyle".as_ptr()
        ];
        let style: *mut Object = msg_send![defaults, stringForKey: key];
        !style.is_null()
    }
}

/// 系统当前是否为深色外观。
#[cfg(windows)]
pub fn system_is_dark() -> bool {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    // 与 autostart 一样走 reg.exe,不为了读一个值引入 winreg;CREATE_NO_WINDOW 免得闪黑框
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

    let Ok(out) = Command::new("reg")
        .args(["query", KEY, "/v", "AppsUseLightTheme"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return false;
    };
    // 输出形如 `AppsUseLightTheme    REG_DWORD    0x0`,0 = 深色。键不存在时 stdout 为空
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next_back()
        .is_some_and(|v| v == "0x0")
}

/// 系统当前是否为深色外观。
#[cfg(not(any(target_os = "macos", windows)))]
pub fn system_is_dark() -> bool {
    false
}
