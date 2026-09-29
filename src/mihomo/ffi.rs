//! Go c-archive 内核的 Rust FFI 封装。
//!
//! 模型:每个 `proxyzms_*` 导出返回 `*mut c_char` 的统一 JSON 信封
//! (`{"ok":bool,"data":T,"error":str}`)。Rust 在 `spawn_blocking` 内调用,
//! 读完后立刻 `proxyzms_free_string`,**裸指针永不跨 await**。
//!
//! 事件回调:Go 通过 C 跳板(`core/callback.c`)调回 Rust;回调里只做
//! `strdup` 到 mpsc,不做任何 await / 锁,保证 Go 统计循环不被 Rust 卡住。

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use super::ffi_types::{CoreEvent, DelayResponse, Envelope, Traffic};
use super::types::{Connections, Proxies};
use tokio::sync::mpsc::UnboundedSender;

// Go c-archive 导出(见 libproxyzms_core.h / bridge.go)。Rust 2021:
// extern 块不标 unsafe,unsafe 只标在调用点。
#[allow(dead_code)] // PR-2 事件流接线后逐个放开
extern "C" {
    fn proxyzms_set_event_callback(cb: EventCallback, user_data: *mut c_void);
    fn proxyzms_init(home_dir: *mut c_char) -> *mut c_char;
    fn proxyzms_apply_config(yaml: *mut c_char, selected_map_json: *mut c_char) -> *mut c_char;
    fn proxyzms_shutdown() -> *mut c_char;
    fn proxyzms_is_running() -> *mut c_char;
    fn proxyzms_set_mode(mode: *mut c_char) -> *mut c_char;
    fn proxyzms_get_mode() -> *mut c_char;
    fn proxyzms_set_tun(enable: c_int) -> *mut c_char;
    fn proxyzms_set_log_level(level: *mut c_char) -> *mut c_char;
    fn proxyzms_validate_config(yaml: *mut c_char) -> *mut c_char;
    fn proxyzms_get_proxies() -> *mut c_char;
    fn proxyzms_select_proxy(group: *mut c_char, name: *mut c_char) -> *mut c_char;
    fn proxyzms_test_delay(
        proxy_name: *mut c_char,
        test_url: *mut c_char,
        timeout_ms: c_int,
    ) -> *mut c_char;
    fn proxyzms_get_connections() -> *mut c_char;
    fn proxyzms_close_connection(id: *mut c_char) -> *mut c_char;
    fn proxyzms_get_traffic(total: c_int) -> *mut c_char;
    fn proxyzms_update_subscription(name: *mut c_char) -> *mut c_char;
    fn proxyzms_free_string(s: *mut c_char);
}

#[allow(dead_code)]
type EventCallback = Option<unsafe extern "C" fn(*const c_char, *mut c_void)>;

fn c_string(s: &str) -> Result<CString, String> {
    CString::new(s).map_err(|_| format!("字符串含 NUL:{s:?}"))
}

/// 调 `f`,把返回的 C 字符串读成 owned String 后立即释放。
unsafe fn read_json<F>(f: F) -> Result<String, String>
where
    F: FnOnce() -> *mut c_char,
{
    let ptr = f();
    if ptr.is_null() {
        return Err("内核返回空指针".to_string());
    }
    let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
    proxyzms_free_string(ptr);
    Ok(s)
}

fn parse_envelope<T>(json: &str) -> Result<T, String>
where
    T: Default + serde::de::DeserializeOwned,
{
    serde_json::from_str::<Envelope<T>>(json)
        .map_err(|e| format!("内核响应解析失败:{e}({json})"))?
        .into_result()
}

async fn blocking<F, T>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("内核调用失败:{e}"))?
}

/// 全局内核句柄。仅一个实例(进程=内核),通过 [`kernel`] 获取。
pub struct Kernel {
    #[allow(dead_code)] // PR-2 事件流接线后放开
    events: Arc<Mutex<Option<UnboundedSender<CoreEvent>>>>,
}

// FFI 导出均已确认内部线程安全(Go runtime + configMu);回调 ctx 只读。
unsafe impl Send for Kernel {}
unsafe impl Sync for Kernel {}

static KERNEL: OnceLock<Kernel> = OnceLock::new();

pub fn kernel() -> &'static Kernel {
    KERNEL.get_or_init(|| Kernel {
        events: Arc::new(Mutex::new(None)),
    })
}

#[allow(dead_code)]
unsafe extern "C" fn on_event(json: *const c_char, _user_data: *mut c_void) {
    if json.is_null() {
        return;
    }
    let s = CStr::from_ptr(json).to_string_lossy().into_owned();
    let Some(tx) = kernel().events.lock().ok().and_then(|g| g.clone()) else {
        return;
    };
    // 解析失败就丢,绝不能在这里 panic / 阻塞(Go 统计循环在等我们返回)
    if let Ok(evt) = serde_json::from_str::<CoreEventWire>(&s) {
        let _ = tx.send(evt.into());
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum CoreEventWire {
    Log { data: CoreEventWireLog },
}

#[derive(serde::Deserialize)]
struct CoreEventWireLog {
    level: String,
    payload: String,
}

impl From<CoreEventWire> for CoreEvent {
    fn from(w: CoreEventWire) -> Self {
        match w {
            CoreEventWire::Log { data } => CoreEvent::Log {
                level: data.level,
                payload: data.payload,
            },
        }
    }
}

impl Kernel {
    /// 注册内核事件流(log 等)。重复调用会替换旧 sender。
    #[allow(dead_code)] // PR-2 事件流接线后放开
    pub fn set_event_sender(&self, tx: UnboundedSender<CoreEvent>) {
        if let Ok(mut guard) = self.events.lock() {
            *guard = Some(tx);
        }
        unsafe {
            proxyzms_set_event_callback(Some(on_event), std::ptr::null_mut());
        }
    }

    /// `init(home_dir)`:仅设 `constant.SetHomeDir`,不启动任何 listener。
    pub async fn init(&self, home_dir: &Path) -> Result<(), String> {
        let home = c_string(&home_dir.to_string_lossy())?;
        blocking(move || unsafe { parse_envelope(&read_json(|| proxyzms_init(home.into_raw()))?) })
            .await
            .map(|_: bool| ())
    }

    /// `apply_config(yaml, selected_map)`:整个 YAML 文本推给内核接管
    /// (取代 ensure_config / reassert_control)。`selected_map` 为策略组 → 节点名。
    pub async fn apply_config(
        &self,
        yaml: &str,
        selected_map: &HashMap<String, String>,
    ) -> Result<Option<String>, String> {
        let yaml = c_string(yaml)?;
        let map = c_string(&serde_json::to_string(selected_map).map_err(|e| e.to_string())?)?;
        blocking(move || unsafe {
            // data 可能是 {"warning":..} 或 true(无 warning 时)
            let v: serde_json::Value =
                parse_envelope(&read_json(|| proxyzms_apply_config(yaml.into_raw(), map.into_raw()))?)?;
            Ok(v
                .get("warning")
                .and_then(|w| w.as_str())
                .filter(|w| !w.is_empty())
                .map(str::to_string))
        })
        .await
    }

    /// `shutdown`:停 listener + executor,内核进入未初始化状态(可再 `init`)。
    pub async fn shutdown(&self) -> Result<(), String> {
        blocking(|| unsafe { parse_envelope(&read_json(|| proxyzms_shutdown())?) })
            .await
            .map(|_: bool| ())
    }

    /// 内核是否处于 running(已 init + apply_config 过)。PR-2 lifecycle 用。
    #[allow(dead_code)]
    pub async fn is_running(&self) -> Result<bool, String> {
        blocking(|| unsafe { parse_envelope(&read_json(|| proxyzms_is_running())?) }).await
    }

    // ---- 运行时控制(与 REST 一一对应) ----

    pub async fn set_mode(&self, mode: &str) -> Result<(), String> {
        let mode = c_string(mode)?;
        blocking(move || unsafe { parse_envelope(&read_json(|| proxyzms_set_mode(mode.into_raw()))?) })
            .await
            .map(|_: bool| ())
    }

    pub async fn get_mode(&self) -> Result<String, String> {
        blocking(|| unsafe { parse_envelope(&read_json(|| proxyzms_get_mode())?) }).await
    }

    pub async fn set_tun(&self, enable: bool) -> Result<(), String> {
        blocking(move || unsafe {
            parse_envelope(&read_json(|| proxyzms_set_tun(if enable { 1 } else { 0 }))?)
        })
        .await
        .map(|_: bool| ())
    }

    pub async fn set_log_level(&self, level: &str) -> Result<(), String> {
        let level = c_string(level)?;
        blocking(move || unsafe {
            parse_envelope(&read_json(|| proxyzms_set_log_level(level.into_raw()))?)
        })
        .await
        .map(|_: bool| ())
    }

    /// 校验 YAML 能否被 mihomo 解析(不应用)。PR-2 settings "测试配置"按钮用。
    #[allow(dead_code)]
    pub async fn validate_config(&self, yaml: &str) -> Result<(), String> {
        let yaml = c_string(yaml)?;
        blocking(move || unsafe {
            parse_envelope(&read_json(|| proxyzms_validate_config(yaml.into_raw()))?)
        })
        .await
        .map(|_: bool| ())
    }

    pub async fn get_proxies(&self) -> Result<Proxies, String> {
        blocking(|| unsafe { parse_envelope(&read_json(|| proxyzms_get_proxies())?) }).await
    }

    /// 在 Selector 组里选节点;`name == ""` 由 Go 端 `ForceSet("")` 清空。
    pub async fn select_proxy(&self, group: &str, name: &str) -> Result<(), String> {
        let group = c_string(group)?;
        let name = c_string(name)?;
        blocking(move || unsafe {
            parse_envelope(&read_json(|| {
                proxyzms_select_proxy(group.into_raw(), name.into_raw())
            })?)
        })
        .await
        .map(|_: bool| ())
    }

    /// 单节点延迟测试(`url` 为空用 mihomo 默认;`timeout <= 0` 用 5000ms)。
    pub async fn test_delay(
        &self,
        proxy_name: &str,
        url: &str,
        timeout_ms: i32,
    ) -> Result<DelayResponse, String> {
        let name = c_string(proxy_name)?;
        let url = c_string(url)?;
        blocking(move || unsafe {
            parse_envelope(&read_json(|| {
                proxyzms_test_delay(name.into_raw(), url.into_raw(), timeout_ms)
            })?)
        })
        .await
    }

    pub async fn get_connections(&self) -> Result<Connections, String> {
        blocking(|| unsafe { parse_envelope(&read_json(|| proxyzms_get_connections())?) }).await
    }

    /// 关闭单条连接。PR-2 connections 页"关闭"按钮用。
    #[allow(dead_code)]
    pub async fn close_connection(&self, id: &str) -> Result<(), String> {
        let id = c_string(id)?;
        blocking(move || unsafe {
            parse_envelope(&read_json(|| proxyzms_close_connection(id.into_raw()))?)
        })
        .await
        .map(|_: bool| ())
    }

    /// 流量:`total = true` 走 `Total()`(累计),false 走 `Now()`(当前速率)。
    /// PR-2 用 `Now()` 直接驱动流量曲线(取代 connections 差分)。
    #[allow(dead_code)]
    pub async fn get_traffic(&self, total: bool) -> Result<Traffic, String> {
        blocking(move || unsafe {
            parse_envelope(&read_json(|| proxyzms_get_traffic(if total { 1 } else { 0 }))?)
        })
        .await
    }

    /// 触发 proxy/rule provider 立即更新(走 mihomo `Update()`,会上网拉新)。
    /// PR-2 用"订阅重载"按钮:不必下载整个 yaml,只让 provider 自己上网。
    #[allow(dead_code)]
    pub async fn update_subscription(&self, name: &str) -> Result<(), String> {
        let name = c_string(name)?;
        blocking(move || unsafe {
            parse_envelope(&read_json(|| proxyzms_update_subscription(name.into_raw()))?)
        })
        .await
        .map(|_: bool| ())
    }
}
