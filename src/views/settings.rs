use crate::autostart;
use crate::bootstrap;
use crate::config::AppConfig;
use crate::mihomo::kernel;
use dioxus::prelude::*;

/// 设置页。FFI 内核后大幅变轻 —— mihomo 二进制 / controller secret / 重下载
/// 这些概念都不复存在,留下的只是订阅源 + 应用行为偏好 + 数据目录覆盖。
#[component]
pub fn Settings() -> Element {
    let mut config = use_context::<Signal<AppConfig>>();
    let mut autostart_enabled = use_signal(autostart::is_enabled);
    let mut autostart_error = use_signal(|| None::<String>);
    let autostart_supported = autostart::is_supported();
    let mut saved = use_signal(|| false);
    let mut sub_status = use_signal(|| None::<String>);
    let mut updating = use_signal(|| false);
    let mut restart_status = use_signal(|| None::<String>);
    let mut restarting = use_signal(|| false);

    // 下载订阅 → 写 config.yaml → apply_config(新内核直接接管,无需重启进程)
    let update_sub = move |_| {
        if updating() {
            return;
        }
        updating.set(true);
        sub_status.set(None);
        spawn(async move {
            let url = config.read().subscription_url.clone();
            match bootstrap::write_subscription(&url).await {
                Ok(()) => {
                    let yaml = match bootstrap::read_config() {
                        Ok(y) => y,
                        Err(e) => {
                            sub_status.set(Some(e));
                            updating.set(false);
                            return;
                        }
                    };
                    let selected = config.read().selected_map.clone();
                    match kernel().apply_config(&yaml, &selected).await {
                        Ok(Some(w)) if !w.is_empty() => sub_status.set(Some(format!(
                            "已更新订阅(应用走默认兜底:{w})"
                        ))),
                        Ok(_) => sub_status.set(Some("已更新订阅并应用".to_string())),
                        Err(e) => sub_status.set(Some(format!("应用配置失败:{e}"))),
                    }
                }
                Err(e) => sub_status.set(Some(format!("下载失败:{e}"))),
            }
            updating.set(false);
        });
    };

    // 重启内核:shutdown → init → apply_config;相当于"刷新"按钮
    let restart_kernel = move |_| {
        if restarting() {
            return;
        }
        restarting.set(true);
        restart_status.set(Some("停止内核…".to_string()));
        spawn(async move {
            let _ = kernel().shutdown().await;
            restart_status.set(Some("重新初始化…".to_string()));
            if let Err(e) = kernel().init(&bootstrap::data_dir()).await {
                restart_status.set(Some(format!("初始化失败:{e}")));
                restarting.set(false);
                return;
            }
            let yaml = match bootstrap::read_config() {
                Ok(y) => y,
                Err(e) => {
                    restart_status.set(Some(e));
                    restarting.set(false);
                    return;
                }
            };
            let selected = config.read().selected_map.clone();
            match kernel().apply_config(&yaml, &selected).await {
                Ok(_) => restart_status.set(Some("已重启内核".to_string())),
                Err(e) => restart_status.set(Some(format!("应用配置失败:{e}"))),
            }
            restarting.set(false);
        });
    };

    let save = move |_| {
        let _ = config.read().save();
        saved.set(true);
    };

    rsx! {
        // 满高列布局 + 内容溢出时纵向滚动
        div { class: "h-full px-6 md:px-12 py-6 max-w-3xl mx-auto flex flex-col gap-6 overflow-y-auto",
            header { class: "border-b-2 border-black pb-4",
                div { class: "text-[11px] uppercase tracking-[0.25em] text-neutral-500", "Configuration" }
                h1 { class: "mt-3 text-4xl font-bold tracking-tighter leading-none", "设置" }
            }

            // 订阅区块
            section {
                div { class: "text-[11px] uppercase tracking-[0.2em] text-[var(--accent)] border-b border-black pb-2 mb-4", "01 / 订阅" }
                Field {
                    label: "订阅地址(节点配置 YAML)",
                    value: config().subscription_url,
                    placeholder: "https://example.com/sub.yaml",
                    oninput: move |v| {
                        config.write().subscription_url = v;
                        saved.set(false);
                    },
                }
                div { class: "mt-4 flex items-center gap-4",
                    button {
                        class: "px-6 py-2 bg-black text-white text-sm uppercase tracking-[0.15em] hover:bg-[var(--accent)] disabled:opacity-40 transition-colors",
                        disabled: updating(),
                        onclick: update_sub,
                        if updating() { "更新中…" } else { "更新订阅并应用" }
                    }
                    if let Some(s) = sub_status() {
                        span { class: "text-xs uppercase tracking-[0.12em] text-neutral-500", "{s}" }
                    }
                }
            }

            // 核心区块(FFI 进程内内核)
            section {
                div { class: "text-[11px] uppercase tracking-[0.2em] text-[var(--accent)] border-b border-black pb-2 mb-4", "02 / 核心" }
                div { class: "space-y-4",
                    Field {
                        label: "工作目录(内含 config.yaml / 订阅缓存,留空使用默认)",
                        value: config().work_dir,
                        placeholder: "自动管理",
                        oninput: move |v| {
                            config.write().work_dir = v;
                            saved.set(false);
                        },
                    }
                    Field {
                        label: "日志级别(debug / info / warning / error / silent)",
                        value: config().log_level,
                        placeholder: "info",
                        oninput: move |v| {
                            config.write().log_level = v;
                            saved.set(false);
                        },
                    }
                    div { class: "flex items-center gap-4 pt-1",
                        button {
                            class: "px-6 py-2 border border-[var(--accent)] text-[var(--accent)] text-sm uppercase tracking-[0.15em] hover:bg-[var(--accent)] hover:text-white disabled:opacity-40 transition-colors",
                            disabled: restarting(),
                            onclick: restart_kernel,
                            if restarting() { "处理中…" } else { "重启内核" }
                        }
                        if let Some(s) = restart_status() {
                            span { class: "text-xs text-neutral-600", "{s}" }
                        }
                    }
                }
            }

            // 系统区块:开机启动
            section {
                div { class: "text-[11px] uppercase tracking-[0.2em] text-[var(--accent)] border-b border-black pb-2 mb-4", "03 / 系统" }
                div { class: "space-y-4",
                    // 开机启动
                    label { class: "flex items-start gap-3 cursor-pointer",
                        input {
                            r#type: "checkbox",
                            class: "mt-1 w-4 h-4 accent-[var(--accent)] cursor-pointer disabled:cursor-not-allowed",
                            checked: autostart_enabled(),
                            disabled: !autostart_supported,
                            onchange: move |evt| {
                                let want = evt.value().parse::<bool>().unwrap_or(false);
                                match autostart::set_enabled(want) {
                                    Ok(()) => {
                                        autostart_enabled.set(autostart::is_enabled());
                                        autostart_error.set(None);
                                    }
                                    Err(e) => autostart_error.set(Some(e)),
                                }
                            },
                        }
                        div { class: "flex-1",
                            div { class: "text-sm font-medium", "开机启动 ProxyZms" }
                            div { class: "mt-1 text-xs text-neutral-500 leading-relaxed",
                                if autostart_supported {
                                    "登录系统后自动拉起。macOS 写入 LaunchAgent;Windows 写入注册表 Run 项。"
                                } else {
                                    "当前平台不支持此选项。"
                                }
                            }
                            if let Some(e) = autostart_error() {
                                div { class: "mt-1 text-xs text-[var(--accent)]", "{e}" }
                            }
                        }
                    }
                }
            }

            div { class: "flex items-center gap-4",
                button {
                    class: "px-8 py-2.5 bg-black text-white text-sm uppercase tracking-[0.15em] hover:bg-[var(--accent)] transition-colors",
                    onclick: save,
                    "保存"
                }
                if saved() {
                    span { class: "text-xs uppercase tracking-[0.12em] text-[var(--accent)]", "已保存" }
                }
            }
        }
    }
}

#[component]
fn Field(
    label: String,
    value: String,
    placeholder: String,
    oninput: EventHandler<String>,
) -> Element {
    rsx! {
        label { class: "block",
            span { class: "block text-[11px] uppercase tracking-[0.15em] text-neutral-500 mb-2", "{label}" }
            input {
                class: "w-full px-0 py-2 bg-transparent border-0 border-b border-black rounded-none outline-none text-base focus:border-[var(--accent)] transition-colors",
                value,
                placeholder,
                oninput: move |e| oninput.call(e.value()),
            }
        }
    }
}
