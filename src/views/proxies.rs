use crate::config::AppConfig;
use crate::mihomo::process;
use crate::mihomo::types::Proxy;
use crate::mihomo::{ApiClient, Controller};
use crate::Telemetry;
use dioxus::prelude::*;
use std::collections::{BTreeMap, HashSet};

/// 节点选择页:顶部模式切换 + 策略组标签栏 + 单组详情。
/// 整页高度恒定,不出外层滚动条;某组节点过多时,仅芯片区静默滚动(`.no-scrollbar`)。
#[component]
pub fn Nodes() -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let tele = use_context::<Telemetry>();
    let mut testing = use_signal(HashSet::<String>::new);
    // 当前激活的策略组名;为 None 或不再存在时回退到首个组
    let mut active = use_signal(|| None::<String>);

    let current_mode = tele
        .configs
        .read()
        .as_ref()
        .map(|c| c.mode.clone())
        .unwrap_or_default();
    let map: BTreeMap<String, Proxy> = match tele.proxies.read().as_ref() {
        Some(p) => p.proxies.clone(),
        None => BTreeMap::new(),
    };
    // 隐藏内置 GLOBAL 组(规则模式下无意义),只展示订阅里的可选 Selector
    let groups: Vec<Proxy> = map
        .values()
        .filter(|p| p.is_selector() && p.name != "GLOBAL")
        .cloned()
        .collect();

    // 选中组兜底:用户未选 / 选项已消失时,默认首个
    let active_name: Option<String> = {
        let cur = active.read().clone();
        match cur {
            Some(n) if groups.iter().any(|g| g.name == n) => Some(n),
            _ => groups.first().map(|g| g.name.clone()),
        }
    };
    let active_group: Option<Proxy> = active_name
        .as_ref()
        .and_then(|n| groups.iter().find(|g| &g.name == n).cloned());

    rsx! {
        div { class: "h-full px-6 md:px-12 py-6 max-w-4xl mx-auto flex flex-col",
            // ── 顶部:标题 + 模式切换 ──
            header { class: "border-b-2 border-black pb-4 flex flex-wrap items-end justify-between gap-4 shrink-0",
                div {
                    div { class: "text-[11px] uppercase tracking-[0.25em] text-neutral-500", "Mihomo · Nodes" }
                    h1 { class: "mt-3 text-4xl font-bold tracking-tighter leading-none", "节点" }
                }
                div { class: "flex items-center gap-2",
                    span { class: "text-[11px] uppercase tracking-[0.18em] text-neutral-500", "模式" }
                    for (val, label) in [("rule", "规则"), ("global", "全局")] {
                        {
                            let mode_active = current_mode == val;
                            rsx! {
                                button {
                                    key: "{val}",
                                    class: if mode_active {
                                        "px-3 py-1.5 text-sm bg-black text-white border border-black"
                                    } else {
                                        "px-3 py-1.5 text-sm border border-neutral-300 text-neutral-700 hover:border-black transition-colors"
                                    },
                                    onclick: move |_| {
                                        let (url, secret) = {
                                            let c = config.read();
                                            (c.controller_url.clone(), c.secret.clone())
                                        };
                                        spawn(async move {
                                            let _ = ApiClient::new(url, secret).set_mode(val).await;
                                            let mut poke = tele.poke;
                                            poke.set(poke() + 1);
                                        });
                                    },
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }

            // ── 空状态 ──
            if groups.is_empty() {
                div { class: "flex-1 flex items-center justify-center",
                    p { class: "text-sm text-neutral-500",
                        "暂无可选策略组(等待内核就绪,或订阅无 Selector 组)。"
                    }
                }
            }

            // ── 标签栏:每个策略组一颗 pill,wrap 进多行也只占顶部一小条 ──
            if !groups.is_empty() {
                div { class: "mt-6 flex flex-wrap gap-2 shrink-0",
                    for g in groups.iter() {
                        {
                            let name = g.name.clone();
                            let is_active = active_name.as_ref() == Some(&name);
                            rsx! {
                                button {
                                    key: "{name}",
                                    class: if is_active {
                                        "px-3 py-1.5 text-sm bg-black text-white border border-black"
                                    } else {
                                        "px-3 py-1.5 text-sm border border-neutral-300 text-neutral-600 hover:border-black transition-colors"
                                    },
                                    onclick: move |_| { active.set(Some(name.clone())); },
                                    "{g.name}"
                                }
                            }
                        }
                    }
                }
            }

            // ── 活动面板:组头(组名 + 当前节点 + 测速) + 芯片区(内部隐式滚动) ──
            if let Some(group) = active_group {
                {
                    let gname = group.name.clone();
                    let gname_test = gname.clone();
                    let is_testing = testing.read().contains(&gname);
                    rsx! {
                        div { class: "mt-4 border border-black flex-1 min-h-0 flex flex-col",
                            // 组头:固定不滚
                            div { class: "flex items-center justify-between gap-3 px-4 py-3 border-b border-neutral-200 shrink-0",
                                div { class: "flex items-baseline gap-3 min-w-0",
                                    span { class: "shrink-0 font-bold tracking-tight", "{group.name}" }
                                    span { class: "shrink-0 text-xs uppercase tracking-[0.12em] text-neutral-400", "{group.proxy_type}" }
                                    span { class: "truncate text-sm text-[#e3000f]", "→ {group.now}" }
                                }
                                button {
                                    class: "shrink-0 inline-flex items-center justify-center min-w-[3.25rem] px-3 py-1 text-[11px] uppercase tracking-[0.12em] border border-black hover:bg-black hover:text-white disabled:hover:bg-transparent disabled:hover:text-black transition-colors",
                                    disabled: is_testing,
                                    onclick: move |_| {
                                        let g = gname_test.clone();
                                        let (url, secret) = {
                                            let c = config.read();
                                            (c.controller_url.clone(), c.secret.clone())
                                        };
                                        testing.write().insert(g.clone());
                                        spawn(async move {
                                            let _ = ApiClient::new(url, secret).group_delay(&g).await;
                                            let mut poke = tele.poke;
                                            poke.set(poke() + 1);
                                            testing.write().remove(&g);
                                        });
                                    },
                                    if is_testing {
                                        div { class: "w-3 h-3 border border-black border-t-transparent rounded-full animate-spin" }
                                    } else {
                                        "测速"
                                    }
                                }
                            }
                            // 芯片区:flex-1 + min-h-0 + 内部静默滚动(.no-scrollbar)
                            div { class: "flex-1 min-h-0 overflow-y-auto no-scrollbar p-4",
                                div { class: "flex flex-wrap gap-2",
                                    for member in group.all.iter() {
                                        {
                                            let chip_active = *member == group.now;
                                            let delay = map.get(member).and_then(|p| p.last_delay());
                                            let g = gname.clone();
                                            let m = member.clone();
                                            rsx! {
                                                button {
                                                    key: "{member}",
                                                    title: "{member}",
                                                    class: if chip_active {
                                                        "max-w-full px-3 py-1.5 text-sm bg-black text-white border border-black flex items-center gap-2"
                                                    } else {
                                                        "max-w-full px-3 py-1.5 text-sm border border-neutral-300 text-neutral-700 hover:border-black transition-colors flex items-center gap-2"
                                                    },
                                                    onclick: move |_| {
                                                        let g = g.clone();
                                                        let m = m.clone();
                                                        let (url, secret) = {
                                                            let c = config.read();
                                                            (c.controller_url.clone(), c.secret.clone())
                                                        };
                                                        spawn(async move {
                                                            let _ = ApiClient::new(url, secret).select_proxy(&g, &m).await;
                                                            let mut poke = tele.poke;
                                                            poke.set(poke() + 1);
                                                        });
                                                    },
                                                    if chip_active {
                                                        span { class: "w-1.5 h-1.5 shrink-0 bg-[#e3000f]" }
                                                    }
                                                    span { class: "truncate max-w-[200px]", "{member}" }
                                                    if let Some(d) = delay {
                                                        span {
                                                            class: if chip_active {
                                                                "shrink-0 min-w-[3.25rem] text-right text-[11px] tabular-nums text-neutral-300"
                                                            } else {
                                                                "shrink-0 min-w-[3.25rem] text-right text-[11px] tabular-nums text-neutral-400"
                                                            },
                                                            "{d} ms"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// TUN 开关 + 授权按钮(放在状态头部)。与代理模式正交,自成一组。
/// TUN 状态读写共享的 `TunState` 信号 —— 与系统托盘完全一致。
#[component]
pub fn TunControls() -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let controller = use_context::<Controller>();
    // 共享 TUN 状态(与托盘同一信号)
    let mut tun_state = use_context::<crate::TunState>().0;
    let auth_status = use_signal(|| None::<String>);
    // 切换请求进行中:显示转圈,期间不被轮询的乐观/旧值干扰
    let mut tun_busy = use_signal(|| false);

    // 二进制/进程是否已提权(决定 TUN 能否真正生效)
    let elevated = process::is_elevated(&config.read().mihomo_path);
    let tun_on = tun_state();
    let busy = tun_busy();

    rsx! {
        div { class: "flex items-center gap-2",
            span { class: "text-[11px] uppercase tracking-[0.18em] text-neutral-500", "TUN" }
            // 开关:请求中转圈;确认成功后再落定(关=黑、开=红)。不再乐观更新,避免 ON/OFF 跳变
            button {
                class: if busy {
                    "px-4 py-1.5 text-sm bg-neutral-500 border border-neutral-500 text-white inline-flex items-center justify-center min-w-[3.5rem]"
                } else if tun_on {
                    "px-4 py-1.5 text-sm bg-[#e3000f] text-white border border-[#e3000f] inline-flex items-center justify-center min-w-[3.5rem]"
                } else {
                    "px-4 py-1.5 text-sm bg-black text-white border border-black hover:bg-neutral-800 transition-colors inline-flex items-center justify-center min-w-[3.5rem]"
                },
                disabled: busy,
                onclick: move |_| {
                    if tun_busy() {
                        return;
                    }
                    let (url, secret) = {
                        let c = config.read();
                        (c.controller_url.clone(), c.secret.clone())
                    };
                    let target = !tun_state();
                    tun_busy.set(true);
                    spawn(async move {
                        // 成功才落定状态(失败保持原状),全程不乐观更新
                        if ApiClient::new(url, secret).set_tun(target).await.is_ok() {
                            tun_state.set(target);
                        }
                        tun_busy.set(false);
                    });
                },
                if busy {
                    // 转圈圈:请求进行中
                    div { class: "w-3.5 h-3.5 border-2 border-white border-t-transparent rounded-full animate-spin" }
                } else if tun_on {
                    "ON"
                } else {
                    "OFF"
                }
            }
            // 未提权时:一键授权(setuid root / UAC)
            if !elevated {
                button {
                    class: "px-4 py-1.5 text-sm border border-[#e3000f] text-[#e3000f] hover:bg-[#e3000f] hover:text-white transition-colors",
                    onclick: move |_| {
                        let path = config.read().mihomo_path.clone();
                        let cfg = config.read().clone();
                        let controller = controller.clone();
                        let mut status = auth_status;
                        status.set(Some("等待授权…".to_string()));
                        spawn(async move {
                            let res = tokio::task::spawn_blocking(move || {
                                process::elevate_binary(&path)
                            })
                            .await;
                            match res {
                                Ok(Ok(())) => {
                                    controller.stop();
                                    match controller.start(&cfg) {
                                        Ok(()) => status.set(Some("已授权".to_string())),
                                        Err(e) => status.set(Some(format!("授权成功但重启失败:{e}"))),
                                    }
                                }
                                Ok(Err(e)) => status.set(Some(e)),
                                Err(_) => status.set(Some("授权任务异常".to_string())),
                            }
                        });
                    },
                    "授权"
                }
            }
            if let Some(s) = auth_status() {
                span { class: "text-xs text-neutral-500", "{s}" }
            }
        }
    }
}

