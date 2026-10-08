//! FFI 桥专属的轻量类型:Go 端 bridge.go JSON 信封与事件推送。
//!
//! 与 REST 形态一一对应的结构体(Connections / Proxy / …)在 `types.rs` 中,
//! Go `snapshot`/`MarshalJSON` 输出的字段名与其反序列化要求保持一致。

use serde::Deserialize;
use std::collections::HashMap;

/// Go 统一响应信封:`{"ok":bool,"data":T | "error":str}`。
#[derive(Debug, Deserialize)]
pub struct Envelope<T> {
    pub ok: bool,
    #[serde(default)]
    pub data: Option<T>,
    #[serde(default)]
    pub error: Option<String>,
}

impl<T> Envelope<T> {
    /// 拆信封:ok=true 时取 data(无 data 语义上用 Default),false 时把 Go 错误文本上抛。
    pub fn into_result(self) -> Result<T, String>
    where
        T: Default,
    {
        if self.ok {
            Ok(self.data.unwrap_or_default())
        } else {
            Err(self
                .error
                .unwrap_or_else(|| "内核未知错误".to_string()))
        }
    }
}

/// `proxyzms_apply_config` 返回的 data(无 warning 时为 `true` 布尔,解析到 Option 即可)。
#[allow(dead_code)] // PR-2 接 settings 配置校验后启用
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ApplyResult {
    #[serde(default)]
    pub warning: Option<String>,
}

/// `proxyzms_get_diag` 的 data:Go 运行时内存 / 协程快照。
/// Go 侧还输出 heap_alloc / sys / num_gc,UI 暂不展示,serde 默认忽略未知字段。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct GoDiag {
    #[serde(default)]
    pub heap_inuse: u64,
    #[serde(default)]
    pub goroutines: u64,
}

/// `proxyzms_get_traffic` 的 data(瞬时 `Now()` 或累计 `Total()`)。
#[allow(dead_code)] // PR-2 流量直读启用
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Traffic {
    #[serde(default)]
    pub up: u64,
    #[serde(default)]
    pub down: u64,
}

/// `proxyzms_test_delay` 的 data;`delay <= 0` 表示超时/失败。
#[allow(dead_code)] // PR-2 测速详情弹层启用
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DelayResponse {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub timeout_ms: i64,
    #[serde(default)]
    pub delay: i32,
    #[serde(default)]
    pub error: Option<String>,
}

/// Go `emitEvent` 推过来的事件(目前只有 log;delay/stats 由 Rust 轮询兜底)。
#[allow(dead_code)] // PR-2 事件流接线后启用
#[derive(Debug, Clone)]
pub enum CoreEvent {
    Log { level: String, payload: String },
}

#[allow(dead_code)] // PR-2 后移除或转用
pub type SelectedMap = HashMap<String, String>;
