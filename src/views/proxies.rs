use crate::config::AppConfig;
use crate::mihomo::process;
use crate::mihomo::types::Proxy;
use crate::mihomo::{ApiClient, Controller};
use crate::node_notes;
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
    // 节点备注:直接读 config.yaml 里的注释(文件没变就走缓存,只 stat 一次)
    let notes = {
        let c = config.read();
        node_notes::load(&c.work_dir)
    };
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

            // ── 标签栏:每个策略组一颗 pill,wrap 进多行也只占顶部一小条。
            //    只有一个组时整条栏都是废话(点它也切不到别处),直接不画 ──
            if groups.len() > 1 {
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
                    // 整组都没写注释时(换了订阅 / 别的机场),收起备注列,节点名占满整行;
                    // 注释被分成多段(`甲骨文 · 日本大阪 · IPv6`)才按列对齐,否则整条塞一列
                    let has_notes = group.all.iter().any(|m| notes.contains_key(m.as_str()));
                    let columnar = group
                        .all
                        .iter()
                        .filter_map(|m| notes.get(m.as_str()))
                        .any(|n| n.parts() >= 2);
                    // 标签栏被省掉时,面板自己补上与标题线之间的呼吸位
                    let panel_gap = if groups.len() > 1 { "mt-4" } else { "mt-6" };
                    rsx! {
                        div { class: "{panel_gap} border border-black flex-1 min-h-0 flex flex-col",
                            // 组头:固定不滚
                            div { class: "flex items-center justify-between gap-3 px-4 py-3 border-b border-neutral-200 shrink-0",
                                div { class: "flex items-baseline gap-3 min-w-0",
                                    span { class: "shrink-0 font-bold tracking-tight", "{group.name}" }
                                    span { class: "shrink-0 text-xs uppercase tracking-[0.12em] text-neutral-400", "{group.proxy_type}" }
                                    span { class: "truncate text-sm text-[var(--accent)]", "→ {group.now}" }
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
                            // 节点列表:flex-1 + min-h-0 + 内部静默滚动(.no-scrollbar)
                            // 一行一个节点,左侧红点标记当前选中,右侧延迟右对齐(tabular-nums 对齐数位)
                            div { class: "flex-1 min-h-0 overflow-y-auto no-scrollbar",
                                div {
                                    for member in group.all.iter() {
                                        {
                                            let row_active = *member == group.now;
                                            let delay = map.get(member).and_then(|p| p.last_delay());
                                            // 备注来自 config.yaml 的注释;没写注释的节点这里全是空串
                                            let note = notes.get(member.as_str()).cloned().unwrap_or_default();
                                            let (n0, n1, n2) = (
                                                note.col(0).to_string(),
                                                note.col(1).to_string(),
                                                note.col(2).to_string(),
                                            );
                                            let n_full = note.text();
                                            // 窗口窄时几列都会截断,悬停用 title 补全整行信息
                                            let row_title = if n_full.is_empty() {
                                                member.clone()
                                            } else {
                                                format!("{member} · {n_full}")
                                            };
                                            let note_cls = if row_active {
                                                "text-[11px] text-neutral-400"
                                            } else {
                                                "text-[11px] text-neutral-500"
                                            };
                                            let g = gname.clone();
                                            let m = member.clone();
                                            rsx! {
                                                button {
                                                    key: "{member}",
                                                    title: "{row_title}",
                                                    class: if row_active {
                                                        "w-full flex items-center gap-3 px-4 py-2.5 text-sm text-left bg-black text-white border-b border-neutral-200"
                                                    } else {
                                                        "w-full flex items-center gap-3 px-4 py-2.5 text-sm text-left text-neutral-700 border-b border-neutral-200 hover:bg-neutral-50 transition-colors"
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
                                                    // 选中标记:未选中也占位,保证节点名左边界对齐
                                                    span {
                                                        class: if row_active {
                                                            "w-1.5 h-1.5 shrink-0 bg-[var(--accent)]"
                                                        } else {
                                                            "w-1.5 h-1.5 shrink-0"
                                                        }
                                                    }
                                                    span {
                                                        class: if has_notes {
                                                            "w-40 shrink-0 truncate"
                                                        } else {
                                                            "flex-1 min-w-0 truncate"
                                                        },
                                                        "{member}"
                                                    }
                                                    // 备注:分段的按「定宽 + 定宽 + 剩余」三列对齐成表格,
                                                    // 没分段的整条占一列;没写注释的节点这几格是空白,不影响对齐
                                                    if has_notes && columnar {
                                                        span { class: "w-20 shrink-0 truncate {note_cls}", "{n0}" }
                                                        span { class: "w-24 shrink-0 truncate {note_cls}", "{n1}" }
                                                        span { class: "flex-1 min-w-0 truncate {note_cls}", "{n2}" }
                                                    } else if has_notes {
                                                        span { class: "flex-1 min-w-0 truncate {note_cls}", "{n_full}" }
                                                    }
                                                    span {
                                                        class: if row_active {
                                                            "shrink-0 min-w-[3.25rem] text-right text-[11px] tabular-nums text-neutral-300"
                                                        } else {
                                                            "shrink-0 min-w-[3.25rem] text-right text-[11px] tabular-nums text-neutral-400"
                                                        },
                                                        if let Some(d) = delay {
                                                            "{d} ms"
                                                        } else {
                                                            "—"
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
                    "px-4 py-1.5 text-sm bg-[var(--accent)] text-white border border-[var(--accent)] inline-flex items-center justify-center min-w-[3.5rem]"
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
                    class: "px-4 py-1.5 text-sm border border-[var(--accent)] text-[var(--accent)] hover:bg-[var(--accent)] hover:text-white transition-colors",
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

