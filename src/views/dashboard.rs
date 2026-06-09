use crate::bootstrap;
use crate::config::AppConfig;
use crate::format;
use crate::mihomo::types::Connections;
use crate::mihomo::{ApiClient, Controller};
use crate::views::{ProxyGroups, TunControls};
use dioxus::prelude::*;
use std::time::Duration;

/// 检测当前网络是否拥有可路由的全局 IPv6 出口。
///
/// 用 UDP `connect` 触发系统路由选择——**不真正发包**,因此不受防火墙 / GFW 影响,
/// 也不会因为某个国外服务器被墙而误报。只要系统能为"访问公网 IPv6"挑出一个
/// 全局单播源地址,就认为网络支持 IPv6。
fn check_ipv6() -> bool {
    use std::net::{Ipv6Addr, SocketAddr, UdpSocket};
    let Ok(sock) = UdpSocket::bind("[::]:0") else {
        return false;
    };
    // 目标是公网 IPv6(Google DNS);connect 仅做路由选择,无 IPv6 路由时会立即报错
    if sock.connect("[2001:4860:4860::8888]:53").is_err() {
        return false;
    }
    match sock.local_addr() {
        Ok(SocketAddr::V6(addr)) => {
            let ip: Ipv6Addr = *addr.ip();
            // 链路本地 fe80::/10 不算(只能本地通信)
            let link_local = (ip.segments()[0] & 0xffc0) == 0xfe80;
            !ip.is_unspecified() && !ip.is_loopback() && !link_local
        }
        _ => false,
    }
}

/// 引导阶段状态。
#[derive(Clone, PartialEq)]
enum Setup {
    /// 正在检查二进制是否就位
    Checking,
    /// 下载中:已下载 / 总大小
    Downloading { done: u64, total: Option<u64> },
    /// 就绪(已安装或用户自定义了路径)
    Ready,
    /// 失败
    Failed(String),
}

#[component]
pub fn Dashboard() -> Element {
    let mut config = use_context::<Signal<AppConfig>>();
    let controller = use_context::<Controller>();

    let mut setup = use_signal(|| Setup::Checking);
    let mut started = use_signal(|| false); // 是否已自动启动过

    // 运行时监控状态
    let mut online = use_signal(|| false);
    let mut stats = use_signal(|| None::<Connections>);
    let mut down_speed = use_signal(|| 0u64);
    let mut up_speed = use_signal(|| 0u64);

    let mut error = use_signal(|| None::<String>);

    // 网络 IPv6 支持检测:每 3 秒探测一次(阻塞探测放到 blocking 线程)。
    // None = 首次检测中;Some(bool) = 最近一次结果。
    let mut ipv6 = use_signal(|| None::<bool>);
    use_future(move || async move {
        loop {
            let supported = tokio::task::spawn_blocking(check_ipv6)
                .await
                .unwrap_or(false);
            ipv6.set(Some(supported));
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });

    // 引导流程:首启检查 → 必要时下载二进制与订阅 → 写入托管路径 → 就绪
    use_future(move || async move {
        let managed_bin = bootstrap::binary_path().to_string_lossy().into_owned();
        // 用户指定了非托管的自定义路径:不接管,直接就绪
        let current = config.read().mihomo_path.clone();
        if !current.trim().is_empty() && current != managed_bin {
            setup.set(Setup::Ready);
            return;
        }

        // 1) 二进制
        if !bootstrap::is_installed() {
            setup.set(Setup::Downloading { done: 0, total: None });
            if let Err(e) = bootstrap::download_binary(move |done, total| {
                setup.set(Setup::Downloading { done, total });
            })
            .await
            {
                setup.set(Setup::Failed(e));
                return;
            }
        }

        // 2) 配置:本地掌握控制权。面板的 controller_url / secret 为唯一准绳。
        let (sub, ec_url, secret) = {
            let c = config.read();
            (
                c.subscription_url.clone(),
                c.controller_url.clone(),
                c.secret.clone(),
            )
        };
        let result = if !bootstrap::config_path().exists() {
            // 首次:拉订阅(并强制注入本地控制器),无订阅则写默认配置
            if sub.trim().is_empty() {
                bootstrap::ensure_config(&ec_url, &secret)
            } else {
                bootstrap::fetch_config(&sub, &ec_url, &secret).await
            }
        } else {
            // 已有配置:每次启动都重新强制本地控制,防止订阅夺权
            bootstrap::reassert_control(&ec_url, &secret)
        };
        if let Err(e) = result {
            setup.set(Setup::Failed(e));
            return;
        }

        // 3) 写回托管路径并持久化
        {
            let mut cfg = config.write();
            cfg.mihomo_path = managed_bin;
            cfg.work_dir = bootstrap::data_dir().to_string_lossy().into_owned();
        }
        let _ = config.read().save();
        setup.set(Setup::Ready);
    });

    // 就绪后自动启动一次(同步,避免跨 await 持有进程句柄)
    let controller_effect = controller.clone();
    use_effect(move || {
        if setup() == Setup::Ready && !started() {
            started.set(true);
            if let Err(e) = controller_effect.start(&config.read()) {
                error.set(Some(format!("启动失败:{e}")));
            }
        }
    });

    // 每秒轮询控制器:版本(在线判断)+ 连接(流量/内存/速率)
    use_future(move || async move {
        let mut prev: Option<(u64, u64)> = None;
        let mut last_cfg: Option<AppConfig> = None;
        let mut client: Option<ApiClient> = None;
        loop {
            let cfg = config();
            if last_cfg.as_ref() != Some(&cfg) {
                client = Some(ApiClient::new(
                    cfg.controller_url.clone(),
                    cfg.secret.clone(),
                ));
                last_cfg = Some(cfg);
                prev = None;
            }
            let api = client.as_ref().unwrap();

            // 版本请求成功 = 控制器在线
            online.set(api.version().await.is_ok());
            match api.connections().await {
                Ok(conn) => {
                    if let Some((pd, pu)) = prev {
                        down_speed.set(conn.download_total.saturating_sub(pd));
                        up_speed.set(conn.upload_total.saturating_sub(pu));
                    }
                    prev = Some((conn.download_total, conn.upload_total));
                    stats.set(Some(conn));
                }
                Err(_) => {
                    down_speed.set(0);
                    up_speed.set(0);
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    let retry = move |_| setup.set(Setup::Checking);

    // 下载/失败阶段:展示引导界面
    match setup() {
        Setup::Checking => {
            return rsx! {
                SetupScreen {
                    eyebrow: "Bootstrap",
                    title: "正在检查 mihomo",
                    body: rsx! {
                        p { class: "text-sm text-neutral-500", "稍候片刻……" }
                    },
                }
            };
        }
        Setup::Downloading { done, total } => {
            let pct = total
                .filter(|t| *t > 0)
                .map(|t| (done as f64 / t as f64 * 100.0).round() as u64);
            let detail = match total {
                Some(t) => format!("{} / {}", format::bytes(done), format::bytes(t)),
                None => format::bytes(done),
            };
            return rsx! {
                SetupScreen {
                    eyebrow: "First Run · Downloading",
                    title: "正在下载 mihomo",
                    body: rsx! {
                        div { class: "flex items-baseline gap-3",
                            span { class: "text-7xl font-bold tracking-tighter tabular-nums leading-none",
                                if let Some(p) = pct { "{p}" } else { "—" }
                            }
                            if pct.is_some() {
                                span { class: "text-2xl font-bold text-[#e3000f]", "%" }
                            }
                        }
                        div { class: "mt-6 h-1 bg-neutral-200",
                            div {
                                class: "h-full bg-[#e3000f] transition-all",
                                style: "width: {pct.unwrap_or(0)}%",
                            }
                        }
                        p { class: "mt-3 text-xs uppercase tracking-[0.15em] text-neutral-500 tabular-nums", "{detail}" }
                    },
                }
            };
        }
        Setup::Failed(msg) => {
            return rsx! {
                SetupScreen {
                    eyebrow: "Error",
                    title: "下载失败",
                    body: rsx! {
                        p { class: "text-sm text-neutral-600 break-words max-w-md", "{msg}" }
                        button {
                            class: "mt-8 px-8 py-3 bg-black text-white text-sm uppercase tracking-[0.15em] hover:bg-[#e3000f] transition-colors",
                            onclick: retry,
                            "重试"
                        }
                    },
                }
            };
        }
        Setup::Ready => {}
    }

    let snap = stats();
    let conn_count = snap.as_ref().map(|s| s.connections.len()).unwrap_or(0);
    let memory = snap.as_ref().map(|s| s.memory).unwrap_or(0);
    let dl_total = snap.as_ref().map(|s| s.download_total).unwrap_or(0);
    let ul_total = snap.as_ref().map(|s| s.upload_total).unwrap_or(0);

    rsx! {
        div { class: "px-6 md:px-12 py-10",
            // 头部:状态(mihomo 随主界面启停,无需手动按钮)
            header { class: "border-b-2 border-black pb-6",
                div { class: "text-[11px] uppercase tracking-[0.25em] text-neutral-500", "Mihomo · Status" }
                // RUNNING 与 TUN 控件同一行,垂直居中对齐
                div { class: "mt-3 flex flex-wrap items-center justify-between gap-4",
                    div { class: "flex items-center gap-4",
                        span {
                            class: if online() {
                                "w-3.5 h-3.5 shrink-0 bg-[#e3000f]"
                            } else {
                                "w-3.5 h-3.5 shrink-0 border-2 border-black"
                            },
                        }
                        h1 { class: "text-4xl md:text-5xl font-bold tracking-tighter leading-none",
                            if online() { "RUNNING" } else { "OFFLINE" }
                        }
                        // 用 IPv6 检测结果替代核心版本号
                        match ipv6() {
                            None => rsx! {
                                span { class: "flex items-center gap-2 text-xs uppercase tracking-[0.15em] text-neutral-400",
                                    span { class: "w-2 h-2 border border-neutral-400" }
                                    "IPv6 检测中"
                                }
                            },
                            Some(true) => rsx! {
                                span { class: "flex items-center gap-2 text-xs uppercase tracking-[0.15em] text-neutral-700",
                                    span { class: "w-2 h-2 bg-neutral-900" }
                                    "支持 IPv6"
                                }
                            },
                            Some(false) => rsx! {
                                span { class: "flex items-center gap-2 text-xs uppercase tracking-[0.15em] text-[#e3000f]",
                                    span { class: "w-2 h-2 bg-[#e3000f]" }
                                    "不支持 IPv6"
                                }
                            },
                        }
                    }
                    // TUN 开关 + 授权,与 RUNNING 垂直居中
                    TunControls {}
                }
            }

            if let Some(err) = error() {
                div { class: "mt-6 border-l-4 border-[#e3000f] pl-4 py-2 text-sm text-neutral-700", "{err}" }
            }

            // 指标:发丝线网格
            div { class: "mt-10 grid grid-cols-2 md:grid-cols-3 border-t border-l border-black",
                StatCell { label: "下载速度", value: format::speed(down_speed()), accent: true }
                StatCell { label: "上传速度", value: format::speed(up_speed()), accent: true }
                StatCell { label: "活动连接", value: conn_count.to_string(), accent: false }
                StatCell { label: "内存占用", value: format::bytes(memory), accent: false }
                StatCell { label: "下载总量", value: format::bytes(dl_total), accent: false }
                StatCell { label: "上传总量", value: format::bytes(ul_total), accent: false }
            }

            // 代理节点选择(仅在内核在线时显示,停止后整块隐藏)
            if online() {
                ProxyGroups {}
            }
        }
    }
}

#[component]
fn StatCell(label: String, value: String, accent: bool) -> Element {
    rsx! {
        div { class: "border-r border-b border-black px-5 py-6",
            div { class: "text-[11px] uppercase tracking-[0.18em] text-neutral-500", "{label}" }
            div {
                class: if accent {
                    "mt-3 text-3xl font-bold tracking-tighter tabular-nums text-[#e3000f]"
                } else {
                    "mt-3 text-3xl font-bold tracking-tighter tabular-nums"
                },
                "{value}"
            }
        }
    }
}

#[component]
fn SetupScreen(eyebrow: String, title: String, body: Element) -> Element {
    rsx! {
        div { class: "h-full flex items-center px-6 md:px-12",
            div { class: "max-w-lg w-full",
                div { class: "text-[11px] uppercase tracking-[0.25em] text-neutral-500", "{eyebrow}" }
                h1 { class: "mt-3 text-4xl font-bold tracking-tighter border-b-2 border-black pb-6", "{title}" }
                div { class: "mt-8", {body} }
            }
        }
    }
}
