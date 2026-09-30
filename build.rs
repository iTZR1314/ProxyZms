//! 构建脚本:
//! 1. **Go 内核桥**(所有平台):`core/` 下的 mihomo c-archive 静态库,
//!    由 `go build -buildmode=c-archive` 产出到 OUT_DIR 后链入 Rust。
//! 2. **Windows**:只嵌入 requireAdministrator 清单,使双击即弹 UAC 以管理员运行
//!    (TUN/Wintun 需要管理员)。**ICON 与 VERSION 都不要在这里嵌** —— dx bundle
//!    自 0.7.10 起会生成 `.winres/resource.lib` 并嵌入它们,重复嵌会在链接期报
//!    `CVT1100: duplicate resource` / `LNK1123`。详见 proxyzms.rc 的注释。

use std::env;
use std::fs;
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

/// Windows arm64 的 Go runtime/cgo 会传 `-mthreads`;这是 MinGW 参数,Clang
/// 的 MSVC target 不接受。单独编译一个 host 可执行 wrapper,只在 arm64 过滤该 flag。
fn build_windows_cgo_wrapper(go: &str, core_dir: &Path, out_dir: &Path) -> PathBuf {
    let (host_goos, host_goarch) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "x86_64") => ("darwin", "amd64"),
        ("macos", "aarch64") => ("darwin", "arm64"),
        ("windows", "x86_64") => ("windows", "amd64"),
        ("windows", "aarch64") => ("windows", "arm64"),
        ("linux", "x86_64") => ("linux", "amd64"),
        ("linux", "aarch64") => ("linux", "arm64"),
        (os, arch) => panic!("不支持在 host {os}/{arch} 上编译 Windows cgo wrapper"),
    };
    let wrapper = out_dir.join("proxyzms-cgo-cc.exe");
    let status = Command::new(go)
        .current_dir(core_dir)
        .args(["build", "-o"])
        .arg(&wrapper)
        .arg("./cgo_cc")
        .env("GOOS", host_goos)
        .env("GOARCH", host_goarch)
        .env("CGO_ENABLED", "0")
        .status()
        .unwrap_or_else(|e| panic!("构建 Windows cgo wrapper 失败:{e}"));
    if !status.success() {
        panic!("构建 Windows cgo wrapper 失败({status})");
    }
    wrapper
}

/// Windows x64 MSVC-compatible linkers reject Go linker's SEH metadata in `go.o`
/// (LNK1223). Removing these sections affects native debugger stack unwinding
/// through Go frames only; Go panic/recover uses its own stack metadata.
fn strip_go_unwind_sections(objects: &[PathBuf]) -> Result<(), String> {
    let mut found = false;
    for object in objects {
        if object.file_name().and_then(|name| name.to_str()) != Some("go.o") {
            continue;
        }
        found = true;
        let output = Command::new("llvm-objcopy")
            .args(["--remove-section=.pdata", "--remove-section=.xdata"])
            .arg(object)
            .output()
            .map_err(|e| format!("启动 llvm-objcopy 失败:{e}"))?;
        if !output.status.success() {
            return Err(format!(
                "llvm-objcopy 处理 {} 失败: {}{}",
                object.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        println!("cargo:warning=Removed Go x64 SEH unwind sections from go.o");
    }
    if !found {
        return Err("Go c-archive 中找不到 go.o,无法处理 MSVC unwind sections".into());
    }
    Ok(())
}

/// 把 Go 生成的 GNU ar 文件重打包为 MSVC LIB,避免 MSVC linker 拒绝 LNK4003。
fn repack_as_msvc_lib(archive: &Path, out_dir: &Path, strip_unwind: bool) -> Result<(), String> {
    let extract_dir = out_dir.join("proxyzms_ar_extract");
    let _ = fs::remove_dir_all(&extract_dir);
    fs::create_dir_all(&extract_dir).map_err(|e| format!("创建 archive 临时目录失败:{e}"))?;

    let result = (|| {
        let output = Command::new("llvm-ar")
            .args(["x"])
            .arg(archive)
            .current_dir(&extract_dir)
            .output()
            .map_err(|e| format!("启动 llvm-ar 失败:{e}"))?;
        if !output.status.success() {
            return Err(format!(
                "llvm-ar 解包 Go c-archive 失败: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let mut objects = fs::read_dir(&extract_dir)
            .map_err(|e| format!("读取 archive 临时目录失败:{e}"))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect::<Vec<_>>();
        objects.sort();
        if objects.is_empty() {
            return Err("Go c-archive 解包后没有 object 文件".to_string());
        }
        println!("cargo:warning=Extracted Go c-archive with llvm-ar");
        if strip_unwind {
            strip_go_unwind_sections(&objects)?;
        }

        let repacked = out_dir.join("_proxyzms_core_repacked.lib");
        let _ = fs::remove_file(&repacked);
        let mut command = Command::new("llvm-lib");
        command
            .arg("/nologo")
            .arg(format!("/out:{}", repacked.display()));
        command.args(&objects);
        let output = command
            .output()
            .map_err(|e| format!("启动 llvm-lib 失败:{e}"))?;
        if !output.status.success() {
            return Err(format!(
                "llvm-lib 重打包 Go c-archive 失败: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        fs::rename(&repacked, archive).map_err(|e| format!("替换 MSVC archive 失败:{e}"))?;
        println!("cargo:warning=Repacked Go c-archive with llvm-lib for MSVC");
        Ok(())
    })();

    let _ = fs::remove_dir_all(&extract_dir);
    result
}

fn write_windows_build_diagnostic(out_dir: &Path, details: &str) {
    let path = out_dir.join("proxyzms-build-diagnostic.txt");
    let _ = fs::write(path, details);
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
    let cgo_wrapper = if goos == "windows" {
        Some(build_windows_cgo_wrapper(&go, &core_dir, &out_dir))
    } else {
        None
    };

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
    if let Some(wrapper) = cgo_wrapper {
        let target = if goarch == "arm64" {
            "aarch64-pc-windows-msvc"
        } else {
            "x86_64-pc-windows-msvc"
        };
        cmd.env("CC", format!("\"{}\"", wrapper.display()))
            .env("PROXYZMS_CGO_TARGET", target);
    }
    let output = cmd
        .output()
        .unwrap_or_else(|e| panic!("go build 无法启动:{e}"));
    if !output.status.success() {
        let details = format!(
            "go build -buildmode=c-archive failed ({})\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if goos == "windows" {
            write_windows_build_diagnostic(&out_dir, &details);
        }
        panic!("{details}");
    }

    if goos == "windows" {
        let strip_unwind = goarch == "amd64";
        println!("cargo:warning=Preparing Windows Go archive for MSVC linking");
        if let Err(e) = repack_as_msvc_lib(&lib, &out_dir, strip_unwind) {
            write_windows_build_diagnostic(&out_dir, &e);
            panic!("准备 Windows Go archive 供 MSVC 链接失败:{e}");
        }
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
