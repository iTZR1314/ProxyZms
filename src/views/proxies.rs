use crate::config::AppConfig;
use crate::mihomo::process;
use crate::mihomo::types::Proxy;
use crate::mihomo::{ApiClient, Controller};
use dioxus::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

/// 代理节点选择:列出所有 Selector 策略组,点选切换 / 按组测速。
#[component]
pub fn ProxyGroups() -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let refresh = use_signal(|| 0u32);
    // 正在测速的策略组名(用于显示转圈、禁用按钮)
    let testing = use_signal(HashSet::<String>::new);

    // 轮询:每 2s 重新拉取 /proxies。首次挂载时内核可能尚未就绪(此时返回空),
    // 靠这个循环在内核起来后自动重抓,页面自愈——否则会一直停在"暂无可选策略组"。
    use_future(move || async move {
        let mut r = refresh;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            r.set(r() + 1);
        }
    });

    let proxies = use_resource(move || async move {
        let _ = refresh(); // 依赖:变化即重新拉取
        let (url, secret) = {
            let c = config.read();
            (c.controller_url.clone(), c.secret.clone())
        };
        ApiClient::new(url, secret).proxies().await.ok()
    });

    let cfg_res = use_resource(move || async move {
        let _ = refresh();
        let (url, secret) = {
            let c = config.read();
            (c.controller_url.clone(), c.secret.clone())
        };
        ApiClient::new(url, secret).configs().await.ok()
    });
    let current_mode = match cfg_res() {
        Some(Some(c)) => c.mode,
        _ => String::new(),
    };

    let map: BTreeMap<String, Proxy> = match proxies() {
        Some(Some(p)) => p.proxies,
        _ => BTreeMap::new(),
    };
    // 隐藏内置的 GLOBAL 组(规则模式下无意义),只显示订阅里的可选组
    let groups: Vec<Proxy> = map
        .values()
        .filter(|p| p.is_selector() && p.name != "GLOBAL")
        .cloned()
        .collect();

    rsx! {
        section {
            div { class: "text-[11px] uppercase tracking-[0.2em] text-[#e3000f] border-b-2 border-black pb-2 mb-6",
                "代理节点 / Proxies"
            }

            // 模式与 TUN:两行对齐网格(代理模式与 TUN 正交,分开摆放)
            div { class: "mb-8 space-y-3",
                // 第一行:代理模式(规则 / 全局,互斥)
                div { class: "flex items-center gap-2",
                    span { class: "w-12 shrink-0 text-[11px] uppercase tracking-[0.18em] text-neutral-500", "模式" }
                    {
                        let modes = [("rule", "规则模式"), ("global", "全局模式")];
                        rsx! {
                            for (val, label) in modes {
                                {
                                    let active = current_mode == val;
                                    rsx! {
                                        button {
                                            key: "{val}",
                                            class: if active {
                                                "px-4 py-1.5 text-sm bg-black text-white border border-black"
                                            } else {
                                                "px-4 py-1.5 text-sm border border-neutral-300 text-neutral-700 hover:border-black transition-colors"
                                            },
                                            onclick: move |_| {
                                                let (url, secret) = {
                                                    let c = config.read();
                                                    (c.controller_url.clone(), c.secret.clone())
                                                };
                                                spawn(async move {
                                                    let _ = ApiClient::new(url, secret).set_mode(val).await;
                                                    let mut r = refresh;
                                                    r.set(r() + 1);
                                                });
                                            },
                                            "{label}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if groups.is_empty() {
                p { class: "text-sm text-neutral-500", "暂无可选策略组(等待内核就绪,或订阅无 Selector 组)。" }
            }

            div { class: "space-y-8",
                for group in groups.iter() {
                    {
                        let gname = group.name.clone();
                        let gname_test = gname.clone();
                        let is_testing = testing.read().contains(&gname);
                        rsx! {
                            div { key: "{group.name}", class: "border border-black",
                                // 组标题:名称 + 类型 + 当前选择 + 测速
                                div { class: "flex items-center justify-between gap-3 px-4 py-3 border-b border-neutral-200",
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
                                            let mut testing = testing;
                                            testing.write().insert(g.clone());
                                            spawn(async move {
                                                let _ = ApiClient::new(url, secret).group_delay(&g).await;
                                                let mut r = refresh;
                                                r.set(r() + 1);
                                                testing.write().remove(&g);
                                            });
                                        },
                                        if is_testing {
                                            // 转圈圈:测速进行中
                                            div { class: "w-3 h-3 border border-black border-t-transparent rounded-full animate-spin" }
                                        } else {
                                            "测速"
                                        }
                                    }
                                }
                                // 成员芯片
                                div { class: "p-4 flex flex-wrap gap-2",
                                    for member in group.all.iter() {
                                        {
                                            let active = *member == group.now;
                                            let delay = map.get(member).and_then(|p| p.last_delay());
                                            let g = gname.clone();
                                            let m = member.clone();
                                            rsx! {
                                                button {
                                                    key: "{member}",
                                                    title: "{member}",
                                                    class: if active {
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
                                                            let mut r = refresh;
                                                            r.set(r() + 1);
                                                        });
                                                    },
                                                    // 选中节点:红点标识
                                                    if active {
                                                        span { class: "w-1.5 h-1.5 shrink-0 bg-[#e3000f]" }
                                                    }
                                                    // 节点名:超长截断,避免撑宽芯片
                                                    span { class: "truncate max-w-[200px]", "{member}" }
                                                    // 延迟徽章:固定宽度右对齐,数字变化不让芯片重排
                                                    if let Some(d) = delay {
                                                        span {
                                                            class: if active {
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
