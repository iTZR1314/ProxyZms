//! 流量页(Flow,首页):扁平 Swiss 排版,平铺呈现运行状态 / IPv6 / TUN 开关 /
//! 流量统计 / 代理节点选择。
//!
//! 本页同时承载首启引导(下载内核/订阅)与内核自动启动 —— 由原"状态页"合并而来。

use crate::bootstrap;
use crate::config::AppConfig;
use crate::format;
use crate::mihomo::Controller;
use crate::Telemetry;
use crate::views::{ProxyGroups, TunControls};
use dioxus::prelude::*;
use std::time::Duration;

/// 运行模式开关。
///
/// - `true` = **正常模式**:就绪后在后台真正拉起 mihomo 内核(发布行为)。
/// - `false` = **UI 调试模式**:只跑界面,**不**真的启动内核,避免与你手动/另一实例
///   已经跑着的内核抢占同一控制端口与控制权。调试 UI 时把它改成 `false` 即可。
const NORMAL_MODE: bool = true;

/// 检测当前网络是否拥有可路由的全局 IPv6 出口。
///
/// 用 UDP `connect` 触发系统路由选择——**不真正发包**,因此不受防火墙 / GFW 影响,
/// 也不会因为某个国外服务器被墙而误报。
fn check_ipv6() -> bool {
    use std::net::{Ipv6Addr, SocketAddr, UdpSocket};
    let Ok(sock) = UdpSocket::bind("[::]:0") else {
        return false;
    };
    if sock.connect("[2001:4860:4860::8888]:53").is_err() {
        return false;
    }
    match sock.local_addr() {
        Ok(SocketAddr::V6(addr)) => {
            let ip: Ipv6Addr = *addr.ip();
            let link_local = (ip.segments()[0] & 0xffc0) == 0xfe80;
            !ip.is_unspecified() && !ip.is_loopback() && !link_local
        }
        _ => false,
    }
}

/// 引导阶段状态。
#[derive(Clone, PartialEq)]
enum Setup {
    Checking,
    Downloading { done: u64, total: Option<u64> },
    Ready,
    Failed(String),
}

#[component]
pub fn Flow() -> Element {
    let mut config = use_context::<Signal<AppConfig>>();
    let controller = use_context::<Controller>();
    let tele = use_context::<Telemetry>();
    let online = tele.online;
    let connections = tele.connections;
    // 派生时序状态(瞬时速率、48 格滚动曲线)统一在 App() 里算并写入 Telemetry,
    // Flow 只读 —— 跨页切换不丢数据,曲线连续。
    let down_speed = tele.down_speed;
    let up_speed = tele.up_speed;
    let history = tele.history;

    let mut setup = use_signal(|| Setup::Checking);
    let mut started = use_signal(|| false);

    let mut error = use_signal(|| None::<String>);
    let mut ipv6 = use_signal(|| None::<bool>);

    // IPv6 支持检测:每 3 秒一次
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
        let current = config.read().mihomo_path.clone();
        if !current.trim().is_empty() && current != managed_bin {
            setup.set(Setup::Ready);
            return;
        }

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

        let (sub, ec_url, secret) = {
            let c = config.read();
            (
                c.subscription_url.clone(),
                c.controller_url.clone(),
                c.secret.clone(),
            )
        };
        let result = if !bootstrap::config_path().exists() {
            if sub.trim().is_empty() {
                bootstrap::ensure_config(&ec_url, &secret)
            } else {
                bootstrap::fetch_config(&sub, &ec_url, &secret).await
            }
        } else {
            bootstrap::reassert_control(&ec_url, &secret)
        };
        if let Err(e) = result {
            setup.set(Setup::Failed(e));
            return;
        }

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
            // UI 调试模式(NORMAL_MODE = false)下不拉起内核,避免与已运行的内核冲突。
            if NORMAL_MODE {
                if let Err(e) = controller_effect.start(&config.read()) {
                    error.set(Some(format!("启动失败:{e}")));
                }
            }
        }
    });

    let retry = move |_| setup.set(Setup::Checking);

    // 引导阶段:全屏引导界面(无地图)
    match setup() {
        Setup::Checking => {
            return rsx! {
                SetupScreen {
                    eyebrow: "Bootstrap",
                    title: "正在检查 mihomo",
                    body: rsx! { p { class: "text-sm text-neutral-500", "稍候片刻……" } },
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
                            div { class: "h-full bg-[#e3000f] transition-all", style: "width: {pct.unwrap_or(0)}%" }
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

    let snap = connections();
    let conn_count = snap.as_ref().map(|s| s.connections.len()).unwrap_or(0);
    let memory = snap.as_ref().map(|s| s.memory).unwrap_or(0);
    let dl_total = snap.as_ref().map(|s| s.download_total).unwrap_or(0);
    let ul_total = snap.as_ref().map(|s| s.upload_total).unwrap_or(0);

    rsx! {
        // 扁平排版:状态 / TUN / 统计 / 节点选择 全部平铺进内容区(无地图、无浮卡)。
        div { class: "px-6 md:px-12 py-10 max-w-4xl",

            // —— 状态头:左侧 = 运行状态 + IPv6,右侧 = TUN 开关(垂直居中) ——
            header { class: "border-b-2 border-black pb-6 flex items-center justify-between gap-6",
                div {
                    div { class: "text-[11px] uppercase tracking-[0.25em] text-neutral-500", "Mihomo · Status" }
                    div { class: "mt-3 flex items-center gap-3",
                        span {
                            class: if online() { "w-3.5 h-3.5 shrink-0 bg-[#e3000f]" } else { "w-3.5 h-3.5 shrink-0 border-2 border-black" },
                        }
                        h1 { class: "text-4xl font-bold tracking-tighter leading-none",
                            if online() { "RUNNING" } else { "OFFLINE" }
                        }
                    }
                    div { class: "mt-4",
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
                }
                // 右侧:TUN 开关(header 是 flex items-center,自动垂直居中)
                div { class: "shrink-0", TunControls {} }
            }

            if let Some(err) = error() {
                div { class: "mt-6 border-l-4 border-[#e3000f] pl-4 py-2 text-sm text-neutral-700", "{err}" }
            }

            // —— 实时流量条形图:48 格滚动,最后一格红色高亮 ——
            {
                let hist = history();
                let max = hist.iter().copied().max().unwrap_or(0).max(1);
                let last = hist.len().saturating_sub(1);
                rsx! {
                    div { class: "mt-8 border-t-2 border-black pt-5",
                        div { class: "flex items-baseline justify-between pb-4",
                            div { class: "text-[11px] uppercase tracking-[0.2em] text-neutral-500", "实时流量 / Throughput" }
                            div { class: "flex gap-4 tabular-nums",
                                div { class: "text-sm font-bold tracking-tight text-[#e3000f]",
                                    span { class: "text-[10px] text-neutral-500 mr-1", "↓" }
                                    "{format::speed(down_speed())}"
                                }
                                div { class: "text-sm font-bold tracking-tight",
                                    span { class: "text-[10px] text-neutral-500 mr-1", "↑" }
                                    "{format::speed(up_speed())}"
                                }
                            }
                        }
                        div { class: "flow-chart",
                            for (i, v) in hist.iter().enumerate() {
                                i {
                                    key: "{i}",
                                    class: if i == last { "flow-bar flow-bar-last" } else { "flow-bar" },
                                    style: "height: {(*v as f64 / max as f64 * 100.0)}%",
                                }
                            }
                        }
                    }
                }
            }

            // —— 统计网格:平铺,撑满内容宽度 ——
            div { class: "mt-8 grid grid-cols-2 sm:grid-cols-3 border-t border-l border-black",
                StatCell { label: "下载速度", value: format::speed(down_speed()), accent: true }
                StatCell { label: "上传速度", value: format::speed(up_speed()), accent: true }
                StatCell { label: "活动连接", value: conn_count.to_string(), accent: false }
                StatCell { label: "内存占用", value: format::bytes(memory), accent: false }
                StatCell { label: "下载总量", value: format::bytes(dl_total), accent: false }
                StatCell { label: "上传总量", value: format::bytes(ul_total), accent: false }
            }

            // —— 节点选择:直接平铺在下方 ——
            div { class: "mt-12", ProxyGroups {} }
        }
    }
}

#[component]
fn StatCell(label: String, value: String, accent: bool) -> Element {
    rsx! {
        // 固定最小高度 + 值底部对齐:数字变长换行也不撑动格子,布局始终稳定
        div { class: "border-r border-b border-black px-4 py-4 min-h-[96px] flex flex-col justify-between",
            div { class: "text-[10px] uppercase tracking-[0.18em] text-neutral-500", "{label}" }
            div {
                // tabular-nums:等宽数字,位数变化时宽度恒定,不左右抖动
                class: if accent {
                    "mt-2 text-lg font-bold tracking-tight tabular-nums leading-tight text-[#e3000f]"
                } else {
                    "mt-2 text-lg font-bold tracking-tight tabular-nums leading-tight"
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
