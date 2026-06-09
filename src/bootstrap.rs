//! 首次启动引导:创建工作目录、下载对应平台的 mihomo 二进制并解压、生成默认配置。
use futures_util::StreamExt;
use std::io::Cursor;
use std::path::{Path, PathBuf};

const MAC_URL: &str = "https://r2.zhoumaosen.top/mihomo/mac.gz";
const WIN_URL: &str = "https://r2.zhoumaosen.top/mihomo/windows.zip";

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

/// 二进制是否已就位。
pub fn is_installed() -> bool {
    binary_path().exists()
}

fn download_url() -> &'static str {
    if cfg!(windows) {
        WIN_URL
    } else {
        MAC_URL
    }
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

    let resp = reqwest::Client::new()
        .get(download_url())
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

    let bin = binary_path();
    if cfg!(windows) {
        extract_zip(&buf, &bin)?;
    } else {
        extract_gz(&buf, &bin)?;
    }
    set_executable(&bin)?;
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
