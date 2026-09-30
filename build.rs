//! 构建脚本:
//! 1. **Go 内核桥**(所有平台):`core/` 下的 mihomo c-archive 静态库,
//!    由 `go build -buildmode=c-archive` 产出到 OUT_DIR 后链入 Rust。
//! 2. **Windows**:只嵌入 requireAdministrator 清单,使双击即弹 UAC 以管理员运行
//!    (TUN/Wintun 需要管理员)。**ICON 与 VERSION 都不要在这里嵌** —— dx bundle
//!    自 0.7.10 起会生成 `.winres/resource.lib` 并嵌入它们,重复嵌会在链接期报
//!    `CVT1100: duplicate resource` / `LNK1123`。详见 proxyzms.rc 的注释。

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    #[cfg(windows)]
    {
        // proxyzms.rc 现在只声明 RT_MANIFEST(id 1, type 24)
        embed_resource::compile("proxyzms.rc", embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }

    build_go_core();
}

/// Rust target → Go target 映射(cgo 必须:内核大量 net/os 调用)。
fn go_target() -> (&'static str, &'static str) {
    let os = env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS unset");
    let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH unset");
    let goos = match os.as_str() {
        "macos" => "darwin",
        "windows" => "windows",
        "linux" => "linux",
        other => panic!("Go 内核不支持 GOOS={other}"),
    };
    let goarch = match arch.as_str() {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => panic!("Go 内核不支持 GOARCH={other}"),
    };
    (goos, goarch)
}

/// Windows c-archive 必须用 MSVC ABI 的 Clang 产物,否则 link.exe 会因 Go/MinGW
/// 的 `.pdata` unwind 格式不兼容而报 LNK1223。Go 的 CC 支持在命令后附 target 参数。
fn go_cc(goos: &str, goarch: &str) -> Option<String> {
    if goos == "windows" {
        let target = match goarch {
            "amd64" => "x86_64-pc-windows-msvc",
            "arm64" => "aarch64-pc-windows-msvc",
            _ => unreachable!("go_target 已校验 GOARCH"),
        };
        if which("clang") {
            return Some(format!("clang --target={target}"));
        }
        panic!("Windows cgo 构建需要 Clang (MSVC target: {target});请检查 runner 的 LLVM 安装");
    }
    // darwin / linux / 本机:Go 自动选择平台 C 编译器
    None
}

/// go 只认 `go version`(不认 --version);gcc/clang 用 --version。
fn which(name: &str) -> bool {
    let arg = if name == "go" { "version" } else { "--version" };
    Command::new(name)
        .arg(arg)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn build_go_core() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR unset"));
    let go = env::var("GO").unwrap_or_else(|_| "go".to_string());
    if !which(&go) {
        panic!("需要 go 1.24+, https://go.dev/dl/ ; 内核为 Go 编成 c-archive 链入");
    }

    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR unset");
    let core_dir = Path::new(&manifest).join("core");
    let lib = out_dir.join("libproxyzms_core.a");
    let (goos, goarch) = go_target();

    let mut cmd = Command::new(&go);
    cmd.current_dir(&core_dir)
        .args(["build", "-buildmode=c-archive"])
        // with_gvisor:mihomo 的 TUN 栈 gvisor 需要这个 build tag,
        // 否则tun.New 时报 "gVisor is not included in this build"
        .args(["-tags", "with_gvisor"])
        // -s -w 去符号/调试信息瘦身;-buildid= 避免 hash 随 Go 版本抖动弄垮 cargo 增量构建
        .args(["-ldflags", "-s -w -buildid="])
        .arg("-o")
        .arg(&lib)
        .arg(".")
        .env("CGO_ENABLED", "1")
        .env("GOOS", goos)
        .env("GOARCH", goarch);
    if let Some(cc) = go_cc(goos, goarch) {
        cmd.env("CC", cc);
    }
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("go build 无法启动:{e}"));
    if !status.success() {
        panic!("go build -buildmode=c-archive 失败({status})");
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=proxyzms_core");
    match goos {
        // macOS:Go runtime 的 net/fsnotify 用到 CoreFoundation;Security 供 x509 系统根;
        // resolv 供 Go DNS resolver(_res_9_ninit / _res_9_nclose 等)
        "darwin" => {
            println!("cargo:rustc-link-lib=framework=CoreFoundation");
            println!("cargo:rustc-link-lib=framework=Security");
            println!("cargo:rustc-link-lib=resolv");
        }
        "windows" => {
            println!("cargo:rustc-link-lib=ws2_32"); // Go net
            println!("cargo:rustc-link-lib=winmm"); // Go runtime timer
            println!("cargo:rustc-link-lib=ntdll"); // sing-tun / wintun helpers
        }
        "linux" => println!("cargo:rustc-link-lib=pthread"),
        _ => {}
    }

    // core/ 下任何 Go/h/c / go.mod / go.sum 变化都触发重新构建
    println!("cargo:rerun-if-changed=core");
    println!("cargo:rerun-if-env-changed=GO");
    println!("cargo:rerun-if-env-changed=GOFLAGS");
    println!("cargo:rerun-if-env-changed=GOPROXY");

    // macOS:同时构建 root helper(`core/helper`),主程序 include_bytes! 嵌入。
    // 若构建失败则写 0 字节占位,`helper::is_installable()` 返回 false,
    // UI 上明确指引而不是 panic(helper 仅在用户点 TUN ON 时才需要)。
    #[cfg(target_os = "macos")]
    build_helper(&out_dir, &go, goos, goarch);
}

/// macOS root helper 子进程(独立二进制,launchd 拉起)。
/// 构建失败不 panic —— helper 只在 macOS TUN 场景需要,允许开发机缺 Go 时
/// 仍然能编译主程序;运行时 `is_installable()` 会把它翻译成用户可读错误。
#[cfg(target_os = "macos")]
fn build_helper(out_dir: &Path, go: &str, goos: &str, goarch: &str) {
    let out = out_dir.join("proxyzms-helper");
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR unset");
    let helper_dir = Path::new(&manifest).join("core").join("helper");

    let status = Command::new(go)
        .current_dir(&helper_dir)
        .args(["build", "-ldflags", "-s -w -buildid="])
        .arg("-o")
        .arg(&out)
        .arg(".")
        .env("CGO_ENABLED", "1")
        .env("GOOS", goos)
        .env("GOARCH", goarch)
        .status();

    match status {
        Ok(s) if s.success() => {}
        _ => {
            // 失败:写占位,主程序能编译但 is_installable()=false
            eprintln!(
                "[build] proxyzms-helper 构建失败;macOS TUN 将不可用(可重跑 cargo build 重试)"
            );
            let _ = std::fs::write(&out, b"");
        }
    }
}
