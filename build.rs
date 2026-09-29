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

/// cgo 交叉编译时的 CC。本机编译 darwin 不需要设置;
/// Windows/arm64(GHA windows-11-arm)需 UCRT MinGW-w64 aarch64,
/// 否则退回 clang --target(镜像里有 LLVM 15,实测可用)。
fn go_cc(goos: &str, goarch: &str) -> Option<String> {
    if goos == "windows" && goarch == "arm64" {
        // msys2 ucrt64 添加进 PATH 后的标准名
        for name in ["aarch64-w64-mingw32-gcc", "clang"] {
            if which(name) {
                return Some(name.to_string());
            }
        }
        panic!(
            "windows/arm64 cgo 需要 aarch64-w64-mingw32-gcc(推荐,见 release.yml)或 clang --target=aarch64-windows-msvc"
        );
    }
    // darwin / windows-x64 / linux / 本机:Go 会自动选 clang/gcc
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
        // macOS:Go runtime 的 net/fsnotify 用到 CoreFoundation;Security 供 x509 系统根
        "darwin" => {
            println!("cargo:rustc-link-lib=framework=CoreFoundation");
            println!("cargo:rustc-link-lib=framework=Security");
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
}
