//! 与 mihomo External Controller API 对应的数据结构。
use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;

/// 把 JSON `null` 当作 `Default`(mihomo 在无连接时 `connections` 返回 null)。
fn null_to_default<'de, D, T>(de: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(de)?.unwrap_or_default())
}

/// `GET /connections` 的快照,含累计流量、内存与当前连接。
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Connections {
    #[serde(default)]
    pub download_total: u64,
    #[serde(default)]
    pub upload_total: u64,
    #[serde(default)]
    pub memory: u64,
    #[serde(default, deserialize_with = "null_to_default")]
    pub connections: Vec<Connection>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub rule: String,
    #[serde(default)]
    pub rule_payload: String,
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    #[serde(default)]
    pub network: String,
    #[serde(rename = "type", default)]
    pub conn_type: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub destination_ip: String,
    #[serde(default)]
    pub destination_port: String,
    #[serde(default)]
    pub source_ip: String,
    #[serde(default)]
    pub source_port: String,
}

impl Connection {
    /// 连接目标的可读标签:优先用域名,否则回退到目标 IP。
    pub fn target(&self) -> String {
        let host = if self.metadata.host.is_empty() {
            &self.metadata.destination_ip
        } else {
            &self.metadata.host
        };
        if self.metadata.destination_port.is_empty() {
            host.clone()
        } else {
            format!("{host}:{}", self.metadata.destination_port)
        }
    }
}

/// `GET /configs` 中我们关心的部分。
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct Configs {
    /// 代理模式:rule / global / direct
    #[serde(default)]
    pub mode: String,
    /// TUN(虚拟网卡)设置
    #[serde(default)]
    pub tun: TunConfig,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct TunConfig {
    #[serde(default)]
    pub enable: bool,
}

/// `GET /proxies`:所有节点与策略组(用 BTreeMap 保证渲染顺序稳定)。
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct Proxies {
    #[serde(default)]
    pub proxies: BTreeMap<String, Proxy>,
}

/// 一个节点或策略组。策略组额外带 `now`(当前选择)与 `all`(成员)。
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Proxy {
    pub name: String,
    #[serde(rename = "type")]
    pub proxy_type: String,
    #[serde(default)]
    pub now: String,
    #[serde(default)]
    pub all: Vec<String>,
    #[serde(default)]
    pub history: Vec<DelayHistory>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct DelayHistory {
    #[serde(default)]
    pub delay: u32,
}

impl Proxy {
    /// 仅 Selector 类型可由用户手动切换。
    pub fn is_selector(&self) -> bool {
        self.proxy_type == "Selector"
    }

    /// 最近一次测速延迟(毫秒);0 表示超时/无数据。
    pub fn last_delay(&self) -> Option<u32> {
        self.history.last().map(|h| h.delay).filter(|d| *d > 0)
    }
}
