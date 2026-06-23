//! mihomo External Controller 的轻量 REST + WebSocket 客户端。
use super::types::{Configs, Connections, Proxies};
use futures_util::{Stream, StreamExt};
use reqwest::Client;
use std::time::Duration;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};

#[derive(Clone)]
pub struct ApiClient {
    client: Client,
    base: String,
    secret: String,
}

impl ApiClient {
    pub fn new(base: impl Into<String>, secret: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        Self {
            client,
            // 去掉末尾斜杠,避免拼接出 `//version`
            base: base.into().trim_end_matches('/').to_string(),
            secret: secret.into(),
        }
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.auth(self.client.get(format!("{}{path}", self.base)))
    }

    fn put(&self, path: &str) -> reqwest::RequestBuilder {
        self.auth(self.client.put(format!("{}{path}", self.base)))
    }

    fn patch(&self, path: &str) -> reqwest::RequestBuilder {
        self.auth(self.client.patch(format!("{}{path}", self.base)))
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.secret.is_empty() {
            req
        } else {
            req.bearer_auth(&self.secret)
        }
    }

    /// 订阅 `/connections` 的 WebSocket 推送流(mihomo 1 Hz 主动推快照)。
    /// 返回的 `Stream` 每帧产出一个解析好的 `Connections`,出错或对端关闭即终止 —
    /// 调用方负责重连(我们的 `main.rs::Poller A` 用 1s 延迟重连)。
    ///
    /// 不持久化 client 状态:连接里若 secret/URL 变化,要等当前流终止后下次 connect 才会生效。
    pub async fn subscribe_connections(
        &self,
    ) -> Result<impl Stream<Item = Result<Connections, String>>, String> {
        let ws_url = http_to_ws(&self.base) + "/connections";
        let mut req = ws_url
            .into_client_request()
            .map_err(|e| format!("WS 请求构造失败:{e}"))?;
        if !self.secret.is_empty() {
            let val = HeaderValue::from_str(&format!("Bearer {}", self.secret))
                .map_err(|e| format!("非法 secret:{e}"))?;
            req.headers_mut().insert("Authorization", val);
        }
        let (ws, _) = tokio_tungstenite::connect_async(req)
            .await
            .map_err(|e| format!("WS 连接失败:{e}"))?;
        // 用 std::future::ready 包裹同步结果 —— async 块本身不是 Unpin,
        // 会让外层 FilterMap 也不 Unpin,导致调用方无法 `.next().await`。
        Ok(ws.filter_map(|msg| {
            std::future::ready(match msg {
                Ok(Message::Text(t)) => {
                    Some(serde_json::from_str::<Connections>(&t).map_err(|e| e.to_string()))
                }
                Ok(Message::Binary(b)) => {
                    Some(serde_json::from_slice::<Connections>(&b).map_err(|e| e.to_string()))
                }
                // 控制帧 / ping / pong 一律忽略;tungstenite 已自动回 pong。
                Ok(_) => None,
                Err(e) => Some(Err(e.to_string())),
            })
        }))
    }

    /// 所有节点与策略组。
    pub async fn proxies(&self) -> reqwest::Result<Proxies> {
        self.get("/proxies").send().await?.json().await
    }

    /// 当前运行配置(我们只取 mode)。
    pub async fn configs(&self) -> reqwest::Result<Configs> {
        self.get("/configs").send().await?.json().await
    }

    /// 切换代理模式:rule / global / direct。
    pub async fn set_mode(&self, mode: &str) -> reqwest::Result<()> {
        self.patch("/configs")
            .json(&serde_json::json!({ "mode": mode }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// 开关 TUN 模式(macOS 上需内核以 root 权限运行才能真正生效)。
    pub async fn set_tun(&self, enable: bool) -> reqwest::Result<()> {
        self.patch("/configs")
            .json(&serde_json::json!({ "tun": { "enable": enable } }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// 在某个 Selector 策略组里选择节点。
    pub async fn select_proxy(&self, group: &str, name: &str) -> reqwest::Result<()> {
        let path = format!("/proxies/{}", urlencoding::encode(group));
        self.put(&path)
            .json(&serde_json::json!({ "name": name }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// 对一个策略组内所有成员发起延迟测速(结果写回各节点 history,随后重拉 /proxies 即可看到)。
    pub async fn group_delay(&self, group: &str) -> reqwest::Result<()> {
        let path = format!(
            "/group/{}/delay?url=https%3A%2F%2Fwww.gstatic.com%2Fgenerate_204&timeout=3000",
            urlencoding::encode(group)
        );
        self.get(&path).send().await?.error_for_status()?;
        Ok(())
    }
}

/// `http(s)://host:port` → `ws(s)://host:port`(末尾 `/` 已由 `ApiClient::new` 去掉)。
fn http_to_ws(base: &str) -> String {
    if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("ws://{base}")
    }
}
