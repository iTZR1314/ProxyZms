//! macOS root helper(`proxyzms-helper`)的安装与 IPC 客户端。
//!
//! helper 是 `core/helper/` 下的 Go 程序,launchd 以 root 拉起,监听
//! `/var/run/proxyzms-helper.sock` 提供三个操作:
//! - `ping`    :探测存活
//! - `tun-on`  :建立 utun 接口并把 fd 通过 SCM_RIGHTS 传回
//! - `tun-off` :回收接口
//!
//! 主 App 通过本模块与它通信。首次使用时若 helper 未安装,
//! 会用 AppleScript 弹一次密码框完成:
//!   cp helper /Library/PrivilegedHelperTools/
//!   cp plist  /Library/LaunchDaemons/
//!   launchctl bootstrap system <plist>
//! 之后 launchd 常驻,开机自启,无需再次密码。
//!
//! helper 二进制在**编译期嵌入**主 exe(`include_bytes!`),`install()`
//! 时落盘。这样 .app 结构不变,仍是单文件分发。
use crate::config::AppConfig;
use std::io::Write;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// helper 的 unix socket 路径(必须与 `core/helper/main.go` 的默认值一致)。
pub const SOCKET_PATH: &str = "/var/run/proxyzms-helper.sock";
/// launchd 服务名。
pub const HELPER_LABEL: &str = "top.zhoumaosen.ProxyZms.Helper";
/// helper 安装目标(launchd 约定目录,只 root 可写)。
const INSTALL_BIN_DIR: &str = "/Library/PrivilegedHelperTools";
/// LaunchDaemon plist 安装目标。
const INSTALL_PLIST_DIR: &str = "/Library/LaunchDaemons";

/// 编译期嵌入的 helper 二进制。由 build.rs 先把 `core/helper` 编成
/// `<OUT_DIR>/proxyzms-helper` 再 `include_bytes!`;若 build.rs 没找到
/// (比如纯 cargo check 而 Go 不可用)会是一个 0 字节占位,
/// `is_installable()` 返回 false 让 UI 给出明确指引。
pub fn embedded_helper() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/proxyzms-helper"))
}

/// helper 二进制是否已嵌入(>1KB 视为有效;0 字节占位是构建期 Go 缺失的信号)。
pub fn is_installable() -> bool {
    embedded_helper().len() > 1024
}

fn install_bin_path() -> PathBuf {
    Path::new(INSTALL_BIN_DIR).join(HELPER_LABEL)
}

fn install_plist_path() -> PathBuf {
    Path::new(INSTALL_PLIST_DIR).join(format!("{HELPER_LABEL}.plist"))
}

/// helper 是否已安装且可达(socket 可连 + ping 通)。
pub fn is_installed() -> bool {
    matches!(ping(), Ok(()))
}

/// 探测 helper 存活。
pub fn ping() -> Result<(), String> {
    let resp = roundtrip(r#"{"op":"ping"}"#, false)?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp.error.unwrap_or_else(|| "ping 失败".into()))
    }
}

/// 请求 helper 建 utun 并通过 SCM_RIGHTS 拿 fd 回来。
/// `cfg` 目前未用(预留给 MTU / 接口名个性化),保持签名与未来一致。
pub async fn ensure_tun_fd(_cfg: &AppConfig) -> Result<i32, String> {
    if !is_installable() {
        return Err(
            "helper 二进制未嵌入(构建时 Go 缺失?)。请用完整 dx bundle / cargo build 重新打包"
                .into(),
        );
    }
    if !is_installed() {
        install()?;
    }
    // helper 可能刚 bootstrap 完还没 ready,给一次短重试
    let mut last_err = String::new();
    for _ in 0..5 {
        match request_tun_on() {
            Ok(fd) => return Ok(fd),
            Err(e) => {
                last_err = e;
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }
    Err(format!("helper 建 utun 失败:{last_err}"))
}

/// 关闭 TUN(helper 回收接口与路由)。失败不阻塞主流程。
pub fn release_tun() -> Result<(), String> {
    let resp = roundtrip(r#"{"op":"tun-off"}"#, false)?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp.error.unwrap_or_else(|| "tun-off 失败".into()))
    }
}

/// 发一行 JSON 请求,读一行 JSON 响应。
/// `expect_fd` 为 true 时额外通过 SCM_RIGHTS 收一个 fd。
fn roundtrip(req_json: &str, expect_fd: bool) -> Result<HelperResponse, String> {
    let mut stream = UnixStream::connect(SOCKET_PATH)
        .map_err(|e| format!("helper 不可达({SOCKET_PATH}):{e}"))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();

    stream
        .write_all(req_json.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .map_err(|e| format!("写请求失败:{e}"))?;

    let mut buf = vec![0u8; 4096];
    let fd = if expect_fd {
        Some(recv_fd(&stream, &mut buf)?)
    } else {
        let n = read_line(&stream, &mut buf)?;
        buf.truncate(n);
        None
    };
    parse_response(&buf, fd)
}

fn request_tun_on() -> Result<i32, String> {
    // 固定 utun9:避免 sing-tun 在未指定名字时不暴露 fd 的问题。
    // utun0-8 常被系统/其它 VPN 占,9 起步撞车率低;helper 端 tun.New
    // 遇到 EBUSY 会失败,主 App 会在下一轮 install/retry 中换名字(目前未实现轮换)。
    let resp = roundtrip(r#"{"op":"tun-on","name":"utun9","mtu":1500}"#, true)?;
    match (resp.ok, resp.fd) {
        (true, Some(fd)) => Ok(fd),
        (true, None) => Err("helper 未回传 fd".into()),
        (false, _) => Err(resp.error.unwrap_or_else(|| "tun-on 失败".into())),
    }
}

#[derive(serde::Deserialize)]
struct HelperResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    name: Option<String>,
    #[serde(skip)]
    fd: Option<i32>,
}

fn parse_response(buf: &[u8], fd: Option<i32>) -> Result<HelperResponse, String> {
    let text = std::str::from_utf8(buf).map_err(|_| "helper 响应非 UTF-8".to_string())?;
    let mut resp: HelperResponse =
        serde_json::from_str(text.trim()).map_err(|e| format!("helper 响应解析失败:{e}"))?;
    resp.fd = fd;
    Ok(resp)
}

/// 不用 expect_fd 时的普通行读取。
fn read_line(stream: &UnixStream, buf: &mut [u8]) -> Result<usize, String> {
    use std::io::Read;
    let mut s = stream;
    let n = s.read(buf).map_err(|e| format!("读响应失败:{e}"))?;
    if n == 0 {
        return Err("helper 关闭连接未回响应".into());
    }
    Ok(n)
}

/// 通过 SCM_RIGHTS 从 unix socket 收一个 fd;同时把数据字节读进 `buf`
/// (helper 端 sendmsg 的 payload 是 1 字节占位,JSON 响应随后用普通 read 到)。
fn recv_fd(stream: &UnixStream, buf: &mut [u8]) -> Result<i32, String> {
    use nix::sys::socket::{recvmsg, ControlMessageOwned, MsgFlags};
    use std::io::IoSliceMut;

    let raw: RawFd = stream.as_raw_fd();
    let mut iov = [IoSliceMut::new(buf)];
    // cmsg buffer 至少能容一个 cmsghdr + 一个 fd
    let mut cmsg = [0u8; 64];

    let msg = recvmsg::<()>(raw, &mut iov, Some(&mut cmsg), MsgFlags::empty())
        .map_err(|e| format!("recvmsg 失败:{e}"))?;

    for c in msg
        .cmsgs()
        .map_err(|e| format!("cmsg 解析失败:{e}"))?
    {
        if let ControlMessageOwned::ScmRights(fds) = c {
            if let Some(&fd) = fds.first() {
                return Ok(fd);
            }
        }
    }
    Err("未收到 SCM_RIGHTS fd".into())
}

/// 用 AppleScript 以管理员权限安装 helper:
/// 1. 把编译期嵌入的二进制先写到 /tmp(普通用户可写)
/// 2. osascript 弹密码框,把它拷到 /Library/PrivilegedHelperTools/,
///    写 LaunchDaemon plist,launchctl bootstrap
///
/// 阻塞直到用户输密码或取消;应在 spawn_blocking 中调用。
pub fn install() -> Result<(), String> {
    if !is_installable() {
        return Err("helper 二进制未嵌入,无法安装".into());
    }

    // 1) 落盘到 /tmp(对 osascript 的 cp 而言,源路径无需权限)
    let tmp = std::env::temp_dir().join(format!("{HELPER_LABEL}.bin"));
    std::fs::write(&tmp, embedded_helper()).map_err(|e| format!("写临时 helper 失败:{e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&tmp)
            .map_err(|e| e.to_string())?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmp, perms).map_err(|e| e.to_string())?;
    }

    // 2) 生成 plist 文本(也落 /tmp,osascript 再拷)
    let plist = launchd_plist();
    let tmp_plist = std::env::temp_dir().join(format!("{HELPER_LABEL}.plist"));
    std::fs::write(&tmp_plist, plist).map_err(|e| format!("写临时 plist 失败:{e}"))?;

    // 3) AppleScript 一次性完成全部特权操作
    let bin_dst = install_bin_path();
    let plist_dst = install_plist_path();
    let script = format!(
        concat!(
            "do shell script \"",
            "mkdir -p '{bin_dir}' '{plist_dir}' && ",
            "cp '{tmp_bin}' '{bin_dst}' && ",
            "chown root:wheel '{bin_dst}' && chmod 755 '{bin_dst}' && ",
            "cp '{tmp_plist}' '{plist_dst}' && ",
            "chown root:wheel '{plist_dst}' && chmod 644 '{plist_dst}' && ",
            "launchctl bootout system '{plist_dst}' 2>/dev/null; ",
            "launchctl bootstrap system '{plist_dst}'",
            "\" with administrator privileges"
        ),
        bin_dir = INSTALL_BIN_DIR,
        plist_dir = INSTALL_PLIST_DIR,
        tmp_bin = tmp.display(),
        bin_dst = bin_dst.display(),
        tmp_plist = tmp_plist.display(),
        plist_dst = plist_dst.display(),
    );

    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .status()
        .map_err(|e| format!("无法运行 osascript:{e}"))?;

    // 清理 /tmp 临时文件(安装成功失败都清)
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&tmp_plist);

    if status.success() {
        Ok(())
    } else {
        Err("授权被取消或安装失败".into())
    }
}

/// 卸载 helper:停 launchd 服务 + 删二进制和 plist。
/// 设置页"卸载 root helper"按钮用;同样要 AppleScript 提权。
pub fn uninstall() -> Result<(), String> {
    let bin_dst = install_bin_path();
    let plist_dst = install_plist_path();
    let script = format!(
        concat!(
            "do shell script \"",
            "launchctl bootout system '{plist_dst}' 2>/dev/null; ",
            "rm -f '{bin_dst}' '{plist_dst}' '{sock}'",
            "\" with administrator privileges"
        ),
        plist_dst = plist_dst.display(),
        bin_dst = bin_dst.display(),
        sock = SOCKET_PATH,
    );
    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .status()
        .map_err(|e| format!("无法运行 osascript:{e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("卸载失败或被取消".into())
    }
}

/// LaunchDaemon plist 内容。`RunAtLoad` 开机即起,`KeepAlive` 崩了重拉。
/// 日志走 /var/log 便于排错。
fn launchd_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{bin}</string>
        <string>-socket</string>
        <string>{sock}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/var/log/{label}.log</string>
    <key>StandardErrorPath</key>
    <string>/var/log/{label}.err</string>
</dict>
</plist>
"#,
        label = HELPER_LABEL,
        bin = install_bin_path().display(),
        sock = SOCKET_PATH,
    )
}
