//! 锁屏前的 best-effort 通知钩子。
//!
//! 目前唯一实现:Bark(iOS 推送)—— 给定完整 URL,GET 一次。
//! 设计原则与 [`crate::ble_lock::locker`] 一致 —— 这一层极薄:
//! - 不判断**什么时候**通知(那是 runner 的事);
//! - 不重试;
//! - 3 秒超时硬封顶,网络挂了也不会拖住锁屏主线;
//! - 任何错误(URL 解析失败 / 超时 / 4xx / 5xx)都返回 `Err(String)`,
//!   由调用方决定要不要写到 UI;**永远不阻止锁屏**。

use std::time::Duration;

/// 单次 GET 的硬超时。锁屏是主线,网络是配件 —— 不让配件拖死主线。
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(3);

/// 触发一次推送通知。空 URL = 跳过(等同未配置);非空 = GET 一次。
///
/// 返回:
/// - `Ok(())` —— URL 为空(跳过)或 HTTP 状态成功(2xx);
/// - `Err(msg)` —— 任意失败,文案可直接展示给用户。
pub async fn notify(url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Ok(());
    }

    // 为本次请求建临时 client(锁屏调用极低频,无需复用)。
    let client = reqwest::Client::builder()
        .timeout(NOTIFY_TIMEOUT)
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败: {e}"))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;

    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("推送返回 HTTP {}", status.as_u16()))
    }
}
