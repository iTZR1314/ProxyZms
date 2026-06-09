//! mihomo External Controller 的轻量 REST 客户端。
use super::types::{Configs, Connections, Proxies, Version};
use reqwest::Client;
use std::time::Duration;

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

    /// 读取版本;成功即代表控制器在线。
    pub async fn version(&self) -> reqwest::Result<Version> {
        self.get("/version").send().await?.json().await
    }

    /// 累计流量、内存占用与当前连接快照。
    pub async fn connections(&self) -> reqwest::Result<Connections> {
        self.get("/connections").send().await?.json().await
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
