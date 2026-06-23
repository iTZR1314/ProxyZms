//! 首次启动引导:创建工作目录、下载对应平台的 mihomo 二进制并解压、生成默认配置。
use futures_util::StreamExt;
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// mihomo 内核镜像。用 R2 而非 GitHub 官方源是为绕过 GFW —— 国内拉
/// `github.com` / `objects.githubusercontent.com` 时常抽风。代价:R2 无版本探测,
/// 始终是当前桶里那一份;要升级请在 R2 重新上传同名文件。
const MIHOMO_R2_BASE: &str = "https://r2.zhoumaosen.top/mihomo";
/// Windows TUN 模式所必需的 wintun 驱动(mihomo 不会自带,缺了 TUN 直接起不来)。
/// 镜像到 R2 同桶,与 mihomo 二进制走同一通道避免 GFW 抽风;0.14.1 是 wintun 项目
/// 最后一个发行版,自 2021 起未变,可放心固定;SHA-256 与官方源一字节不差。
#[cfg(windows)]
const WINTUN_URL: &str = "https://r2.zhoumaosen.top/mihomo/wintun-0.14.1.zip";

/// 受本程序托管的 mihomo 工作目录:`<config_dir>/proxy-zms/mihomo`
pub fn data_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("proxy-zms")
        .join("mihomo")
}

/// 托管二进制的完整路径(Windows 为 mihomo.exe)。
pub fn binary_path() -> PathBuf {
    let name = if cfg!(windows) { "mihomo.exe" } else { "mihomo" };
    data_dir().join(name)
}

/// 全部托管资源就位:mihomo 二进制 + Windows 下的 wintun.dll。
pub fn is_installed() -> bool {
    if !binary_path().exists() {
        return false;
    }
    #[cfg(windows)]
    if !wintun_path().exists() {
        return false;
    }
    true
}

/// Windows TUN 所需 wintun.dll 路径(放在 mihomo.exe 同目录,mihomo 会按该处加载)。
#[cfg(windows)]
pub fn wintun_path() -> PathBuf {
    data_dir().join("wintun.dll")
}

/// 当前构建目标对应的 R2 资源 URL(完整下载链接)。
/// R2 桶里目前只有这三件,其它架构请上传后再加分支。
fn asset_url() -> Result<String, String> {
    let filename = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows.zip"
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        "windowsarm.zip"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "mac.gz"
    } else {
        return Err(format!(
            "R2 镜像未上传该架构:{} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
    };
    Ok(format!("{MIHOMO_R2_BASE}/{filename}"))
}

/// 默认 config.yaml 的基础内容(不含控制器设置,控制器由本地强制注入)。
const DEFAULT_BASE: &str = "\
mixed-port: 7890
allow-lan: false
mode: rule
log-level: info
";

/// 订阅里所有会影响控制权的顶层键,一律剥离后由本地接管。
/// 前缀匹配可覆盖 external-controller / -tls / -unix / -cors 等变体。
const SEIZED_PREFIXES: &[&str] = &["external-controller", "secret"];

/// config.yaml 的完整路径。
pub fn config_path() -> PathBuf {
    data_dir().join("config.yaml")
}

/// 把面板的 controller URL 转成 mihomo 需要的 `host:port`(去掉 scheme)。
fn url_to_ec(url: &str) -> String {
    url.trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .to_string()
}

/// 剥离 YAML 中所有被本地接管的顶层键(含其下的缩进子项)。
fn strip_seized_keys(yaml: &str) -> String {
    let mut out = String::with_capacity(yaml.len());
    let mut skipping = false;
    for line in yaml.lines() {
        let is_top_key = !line.is_empty() && !line.starts_with([' ', '\t']);
        if is_top_key {
            let key = line.split(':').next().unwrap_or("").trim();
            skipping = SEIZED_PREFIXES.iter().any(|p| key.starts_with(p));
        }
        // 非顶层行(缩进/空行)沿用上一个顶层键的取舍,从而连同子项一起跳过
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// 用本地权威的控制器设置改写配置:剥离订阅自带的控制器键,注入面板的设置。
/// `ec_addr` 为 `host:port`,`secret` 为空表示无鉴权。
fn enforce_local_control(yaml: &str, ec_addr: &str, secret: &str) -> String {
    let mut header = format!("external-controller: {ec_addr}\n");
    // 显式写出 secret(空串)以确保覆盖、不继承订阅里的鉴权
    header.push_str(&format!("secret: \"{secret}\"\n"));
    format!("{header}{}", strip_seized_keys(yaml))
}

/// 工作目录无 config.yaml 时写入一份最小默认配置(由本地接管控制器)。
pub fn ensure_config(ec_url: &str, secret: &str) -> Result<(), String> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = config_path();
    if !path.exists() {
        let content = enforce_local_control(DEFAULT_BASE, &url_to_ec(ec_url), secret);
        std::fs::write(&path, content).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 对现有 config.yaml 重新强制本地控制(每次启动前调用,确保控制权不被订阅夺走)。
pub fn reassert_control(ec_url: &str, secret: &str) -> Result<(), String> {
    let path = config_path();
    if !path.exists() {
        return Ok(());
    }
    let current = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let content = enforce_local_control(&current, &url_to_ec(ec_url), secret);
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

/// 下载订阅 yaml 作为 config.yaml,并立即用本地控制器设置改写。
pub async fn fetch_config(url: &str, ec_url: &str, secret: &str) -> Result<(), String> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let text = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("请求失败:{e}"))?
        .error_for_status()
        .map_err(|e| format!("订阅返回错误:{e}"))?
        .text()
        .await
        .map_err(|e| format!("读取订阅失败:{e}"))?;

    let content = enforce_local_control(&text, &url_to_ec(ec_url), secret);
    std::fs::write(config_path(), content).map_err(|e| e.to_string())
}

/// 下载并解压 mihomo 二进制。`on_progress(已下载字节, 总字节)` 用于上报进度。
pub async fn download_binary<F: FnMut(u64, Option<u64>)>(
    mut on_progress: F,
) -> Result<PathBuf, String> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let bin = binary_path();
    // mihomo 已存在则跳过(节省升级流量);
    // settings.rs 的「重新下载内核」按钮会在调用前先 remove_file,所以强制刷新仍然有效。
    if !bin.exists() {
        let url = asset_url()?;
        let is_zip = url.ends_with(".zip");
        let resp = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("请求失败:{e}"))?
            .error_for_status()
            .map_err(|e| format!("下载失败:{e}"))?;

        let total = resp.content_length();
        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut last_report = 0u64;
        on_progress(0, total);
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("传输中断:{e}"))?;
            buf.extend_from_slice(&chunk);
            let done = buf.len() as u64;
            // 每 256KB 上报一次,避免过于频繁地触发重渲染
            if done - last_report >= 256 * 1024 {
                last_report = done;
                on_progress(done, total);
            }
        }
        on_progress(buf.len() as u64, total);

        if is_zip {
            extract_zip(&buf, &bin)?;
        } else {
            extract_gz(&buf, &bin)?;
        }
        set_executable(&bin)?;
    }

    // Windows 上额外保证 wintun.dll 就位(已存在则不动)。
    ensure_wintun().await?;
    Ok(bin)
}


/// 解压单文件 gzip 到目标路径。
fn extract_gz(data: &[u8], out: &Path) -> Result<(), String> {
    use flate2::read::GzDecoder;
    let mut decoder = GzDecoder::new(data);
    let mut file = std::fs::File::create(out).map_err(|e| e.to_string())?;
    std::io::copy(&mut decoder, &mut file).map_err(|e| e.to_string())?;
    Ok(())
}

/// 从 zip 中取出可执行文件写到目标路径。
fn extract_zip(data: &[u8], out: &Path) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(data)).map_err(|e| e.to_string())?;
    let count = zip.len();
    for i in 0..count {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if !entry.is_file() {
            continue;
        }
        // 取 .exe;若压缩包只有一个文件则直接用它
        if entry.name().to_lowercase().ends_with(".exe") || count == 1 {
            let mut file = std::fs::File::create(out).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err("压缩包中未找到可执行文件".into())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .map_err(|e| e.to_string())?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Windows 下若 wintun.dll 缺失则从 wintun.net 下载并解压对应架构的 DLL。
/// 非 Windows 平台为 no-op。
#[cfg(windows)]
async fn ensure_wintun() -> Result<(), String> {
    let out = wintun_path();
    if out.exists() {
        return Ok(());
    }
    let arch_dir = if cfg!(target_arch = "x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        return Err(format!(
            "wintun 不支持当前架构:{}",
            std::env::consts::ARCH
        ));
    };
    let bytes = reqwest::Client::new()
        .get(WINTUN_URL)
        .send()
        .await
        .map_err(|e| format!("wintun 下载失败:{e}"))?
        .error_for_status()
        .map_err(|e| format!("wintun 下载失败:{e}"))?
        .bytes()
        .await
        .map_err(|e| format!("wintun 下载失败:{e}"))?
        .to_vec();
    // 包内布局:wintun/bin/{amd64,arm64,x86,arm}/wintun.dll
    let want_suffix = format!("bin/{arch_dir}/wintun.dll");
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if !entry.is_file() {
            continue;
        }
        if entry.name().replace('\\', "/").ends_with(&want_suffix) {
            let mut file = std::fs::File::create(&out).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err(format!("wintun.zip 中未找到 {want_suffix}"))
}

#[cfg(not(windows))]
async fn ensure_wintun() -> Result<(), String> {
    Ok(())
}
