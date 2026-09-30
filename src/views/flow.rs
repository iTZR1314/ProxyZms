//! 流量页(Flow,首页):扁平 Swiss 排版,平铺呈现运行状态 / IPv6 / TUN 开关 /
//! 流量统计。
//!
//! 本页同时承载首启引导(下载订阅/启动内核)。FFI 内核不再下载二进制,
//! 状态机由「下载 ⇄ 安装」变为「解析 ⇒ 应用 config ⇒ 启动」一条直路。

use crate::bootstrap;
use crate::config::AppConfig;
use crate::format;
use crate::mihomo::kernel;
use crate::Telemetry;
use crate::views::TunControls;
use dioxus::prelude::*;
use std::time::Duration;

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
    Applying { progress: String },
    Ready,
    Failed(String),
}

#[component]
pub fn Flow() -> Element {
    let config = use_context::<Signal<AppConfig>>();
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

    // 引导流程:env=DIOXUS_NO_KERNEL ⇒ 跳过(供 UI 调试);否则初始化 → apply_config
    use_future(move || async move {
        // UI 调试:不拉起内核,跳过订阅下载 — 用来跑 dx serve 调样式
        if std::env::var_os("DIOXUS_NO_KERNEL").is_some() {
            setup.set(Setup::Ready);
            return;
        }

        // Flow 是路由页,每次从其他页面切回都会重新挂载。
        // 内核属于 App 进程级状态;已运行时不可再次 init/apply_config,
        // 否则会重复重建 listener/provider,尤其可能意外关闭刚启用的 TUN。
        if kernel().is_running().await.unwrap_or(false) {
            setup.set(Setup::Ready);
            return;
        }

        // 1) 订阅 config.yaml 不存在 / 设置了订阅 URL,先下载
        let sub = config.read().subscription_url.clone();
        setup.set(Setup::Applying {
            progress: if sub.trim().is_empty() {
                "应用内置默认配置…".to_string()
            } else {
                "下载订阅…".to_string()
            },
        });

        let work_dir = config.read().work_dir.clone();
        if !bootstrap::effective_config_path(&work_dir).exists() && !sub.trim().is_empty() {
            if let Err(e) = bootstrap::write_subscription(&sub, &work_dir).await {
                setup.set(Setup::Failed(format!("下载订阅失败:{e}")));
                return;
            }
        }
        let yaml = match bootstrap::read_config(&work_dir) {
            Ok(y) => y,
            Err(e) => {
                setup.set(Setup::Failed(e));
                return;
            }
        };

        // 2) init 内核 home 目录(mihomo 的 -d 等价;work_dir 覆盖生效)
        let home = bootstrap::effective_data_dir(&work_dir);
        setup.set(Setup::Applying {
            progress: "初始化内核…".to_string(),
        });
        if let Err(e) = kernel().init(&home).await {
            setup.set(Setup::Failed(format!("内核初始化失败:{e}")));
            return;
        }

        // 3) 应用 config(Go 端 UnmarshalRawConfig + ParseRawConfig + Listener 重建)
        let selected_map = config.read().selected_map.clone();
        setup.set(Setup::Applying {
            progress: "应用代理配置…".to_string(),
        });
        match kernel().apply_config(&yaml, &selected_map).await {
            Ok(Some(warning)) if !warning.is_empty() => {
                // mihomo 内部走了 default config 兜底,提示但不阻塞
                error.set(Some(format!("配置应用走默认兜底:{warning}")));
            }
            Ok(_) => {}
            Err(e) => {
                setup.set(Setup::Failed(format!("应用配置失败:{e}")));
                return;
            }
        }

        // 4) 恢复用户持久化的 mode / log_level / catalog(仅当下发不 chang rust TUN)
        let persisted = config.read().clone();
        if let Err(e) = kernel()
            .set_lan_share(persisted.share_lan, persisted.share_port)
            .await
        {
            error.set(Some(format!("应用局域网共享设置失败:{e}")));
        }
        if !persisted.mode.trim().is_empty() && persisted.mode != "rule" {
            let _ = kernel().set_mode(&persisted.mode).await;
        }
        let mut mode_sig = tele.mode;
        if !persisted.mode.trim().is_empty() {
            mode_sig.set(persisted.mode.clone());
        } else if let Ok(m) = kernel().get_mode().await {
            mode_sig.set(m);
        }
        if !persisted.log_level.trim().is_empty() {
            let _ = kernel().set_log_level(&persisted.log_level).await;
        }
        // TUN 不在启动时自动开启 — macOS 需 helper 授权,用户点 ON 时才走提权流程

        setup.set(Setup::Ready);
    });

    // FFI 内核是 spawn_blocking 调用,无 "start 进程" 概念;started 只作幂等锁
    use_effect(move || {
        if setup() == Setup::Ready && !started() {
            started.set(true);
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
        Setup::Applying { progress } => {
            return rsx! {
                SetupScreen {
                    eyebrow: "Bootstrap · Applying",
                    title: "正在启动内核",
                    body: rsx! {
                        p { class: "text-sm text-neutral-500", "{progress}" }
                    },
                }
            };
        }
        Setup::Failed(msg) => {
            return rsx! {
                SetupScreen {
                    eyebrow: "Error",
                    title: "内核启动失败",
                    body: rsx! {
                        p { class: "text-sm text-neutral-600 break-words max-w-md", "{msg}" }
                        button {
                            class: "mt-8 px-8 py-3 bg-black text-white text-sm uppercase tracking-[0.15em] hover:bg-[var(--accent)] transition-colors",
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
        // 满高列布局:垂直居中;不出滚动条(外层 main 在非连接页用 overflow-hidden)。
        div { class: "h-full px-6 md:px-12 py-6 max-w-4xl mx-auto flex flex-col justify-center gap-5",

            // —— 状态头:运行 / IPv6 / TUN ——
            header { class: "border-b-2 border-black pb-4 flex items-center justify-between gap-6",
                div {
                    div { class: "text-[11px] uppercase tracking-[0.25em] text-neutral-500", "Mihomo · Status" }
                    div { class: "mt-3 flex items-center gap-3",
                        span {
                            class: if online() { "w-3.5 h-3.5 shrink-0 bg-[var(--accent)]" } else { "w-3.5 h-3.5 shrink-0 border-2 border-black" },
                        }
                        h1 { class: "text-3xl font-bold tracking-tighter leading-none",
                            if online() { "RUNNING" } else { "OFFLINE" }
                        }
                    }
                    div { class: "mt-3",
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
                                span { class: "flex items-center gap-2 text-xs uppercase tracking-[0.15em] text-[var(--accent)]",
                                    span { class: "w-2 h-2 bg-[var(--accent)]" }
                                    "不支持 IPv6"
                                }
                            },
                        }
                    }
                }
                div { class: "shrink-0", TunControls {} }
            }

            if let Some(err) = error() {
                div { class: "border-l-4 border-[var(--accent)] pl-4 py-2 text-sm text-neutral-700", "{err}" }
            }

            // —— 实时流量条形图(.flow-chart 高度由 CSS 控制) ——
            {
                let hist = history();
                let max = hist.iter().copied().max().unwrap_or(0).max(1);
                let last = hist.len().saturating_sub(1);
                rsx! {
                    div {
                        div { class: "flex items-baseline justify-between pb-3",
                            div { class: "text-[11px] uppercase tracking-[0.2em] text-neutral-500", "实时流量 / Throughput" }
                            div { class: "flex gap-4 tabular-nums",
                                div { class: "text-sm font-bold tracking-tight text-[var(--accent)]",
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

            // —— 6 格统计:压缩 min-h + padding,字号 ——
            div { class: "grid grid-cols-2 sm:grid-cols-3 border-t border-l border-black",
                StatCell { label: "下载速度", value: format::speed(down_speed()), accent: true }
                StatCell { label: "上传速度", value: format::speed(up_speed()), accent: true }
                StatCell { label: "活动连接", value: conn_count.to_string(), accent: false }
                StatCell { label: "内存占用", value: format::bytes(memory), accent: false }
                StatCell { label: "下载总量", value: format::bytes(dl_total), accent: false }
                StatCell { label: "上传总量", value: format::bytes(ul_total), accent: false }
            }
        }
    }
}

#[component]
fn StatCell(label: String, value: String, accent: bool) -> Element {
    rsx! {
        // 压缩版:min-h-[72px] + py-3 + text-base,适配满高列布局
        div { class: "border-r border-b border-black px-3 py-3 min-h-[72px] flex flex-col justify-between",
            div { class: "text-[10px] uppercase tracking-[0.18em] text-neutral-500", "{label}" }
            div {
                class: if accent {
                    "mt-1 text-base font-bold tracking-tight tabular-nums leading-tight text-[var(--accent)]"
                } else {
                    "mt-1 text-base font-bold tracking-tight tabular-nums leading-tight"
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
