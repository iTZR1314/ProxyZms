use crate::autostart;
use crate::ble_lock::BleLockConfig;
use crate::bootstrap;
use crate::config::AppConfig;
use crate::format;
use crate::mihomo::Controller;
use dioxus::prelude::*;

#[component]
pub fn Settings() -> Element {
    let mut config = use_context::<Signal<AppConfig>>();
    let mut ble_config = use_context::<Signal<BleLockConfig>>();
    let controller = use_context::<Controller>();
    let mut autostart_enabled = use_signal(autostart::is_enabled);
    let mut autostart_error = use_signal(|| None::<String>);
    let autostart_supported = autostart::is_supported();
    let mut saved = use_signal(|| false);
    let mut sub_status = use_signal(|| None::<String>);
    let mut updating = use_signal(|| false);
    let mut core_status = use_signal(|| None::<String>);
    let mut core_busy = use_signal(|| false);

    // 删除现有 mihomo 二进制并重新下载,然后重启内核
    let redownload_core = {
        let controller = controller.clone();
        move |_| {
            if core_busy() {
                return;
            }
            core_busy.set(true);
            core_status.set(Some("停止内核…".to_string()));
            let controller = controller.clone();
            let cfg = config.read().clone();
            spawn(async move {
                controller.stop();
                // 删除旧二进制(目录归属当前用户,即使旧文件是 root setuid 也能删)
                let _ = std::fs::remove_file(bootstrap::binary_path());
                core_status.set(Some("下载中…".to_string()));
                let result = bootstrap::download_binary(move |done, total| {
                    let text = match total {
                        Some(t) => format!("下载中 {} / {}", format::bytes(done), format::bytes(t)),
                        None => format!("下载中 {}", format::bytes(done)),
                    };
                    core_status.set(Some(text));
                })
                .await;
                match result {
                    Ok(_) => match controller.start(&cfg) {
                        Ok(()) => core_status.set(Some("已重新下载并重启内核(TUN 需重新授权)".to_string())),
                        Err(e) => core_status.set(Some(format!("下载完成但重启失败:{e}"))),
                    },
                    Err(e) => core_status.set(Some(format!("下载失败:{e}"))),
                }
                core_busy.set(false);
            });
        }
    };

    let save = move |_| {
        let _ = config.read().save();
        saved.set(true);
    };

    // 下载订阅 → 写 config.yaml → 同步控制器信息 → 重启 mihomo
    let update_sub = move |_| {
        if updating() {
            return;
        }
        updating.set(true);
        sub_status.set(Some("正在下载订阅...".to_string()));
        let controller = controller.clone();
        spawn(async move {
            // 面板设置为唯一准绳:订阅里的控制器/secret 会被强制覆盖
            let (url, ec_url, secret) = {
                let c = config.read();
                (
                    c.subscription_url.clone(),
                    c.controller_url.clone(),
                    c.secret.clone(),
                )
            };
            match bootstrap::fetch_config(&url, &ec_url, &secret).await {
                Ok(()) => {
                    // 重启使新配置生效
                    let cfg = config.read().clone();
                    controller.stop();
                    match controller.start(&cfg) {
                        Ok(()) => sub_status.set(Some("已更新订阅并重启 mihomo".to_string())),
                        Err(e) => sub_status.set(Some(format!("订阅已更新,但重启失败:{e}"))),
                    }
                }
                Err(e) => sub_status.set(Some(e)),
            }
            updating.set(false);
        });
    };

    rsx! {
        // 满高列布局 + 内容溢出时纵向滚动(03/系统 区块加入后内容可能超过窗口高度)
        div { class: "h-full px-6 md:px-12 py-6 max-w-3xl mx-auto flex flex-col gap-6 overflow-y-auto",
            header { class: "border-b-2 border-black pb-4",
                div { class: "text-[11px] uppercase tracking-[0.25em] text-neutral-500", "Configuration" }
                h1 { class: "mt-3 text-4xl font-bold tracking-tighter leading-none", "设置" }
            }

            // 订阅区块
            section {
                div { class: "text-[11px] uppercase tracking-[0.2em] text-[#e3000f] border-b border-black pb-2 mb-4", "01 / 订阅" }
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
                        class: "px-6 py-2 bg-black text-white text-sm uppercase tracking-[0.15em] hover:bg-[#e3000f] disabled:opacity-40 transition-colors",
                        disabled: updating(),
                        onclick: update_sub,
                        if updating() { "更新中…" } else { "更新订阅并重启" }
                    }
                    if let Some(s) = sub_status() {
                        span { class: "text-xs uppercase tracking-[0.12em] text-neutral-600", "{s}" }
                    }
                }
            }

            // 核心区块
            section {
                div { class: "text-[11px] uppercase tracking-[0.2em] text-[#e3000f] border-b border-black pb-2 mb-4", "02 / 核心" }
                div { class: "space-y-4",
                    Field {
                        label: "mihomo 可执行文件路径(留空使用内置下载)",
                        value: config().mihomo_path,
                        placeholder: "自动管理",
                        oninput: move |v| {
                            config.write().mihomo_path = v;
                            saved.set(false);
                        },
                    }
                    Field {
                        label: "工作目录 (-d,含 config.yaml)",
                        value: config().work_dir,
                        placeholder: "自动管理",
                        oninput: move |v| {
                            config.write().work_dir = v;
                            saved.set(false);
                        },
                    }
                    Field {
                        label: "Secret(可选)",
                        value: config().secret,
                        placeholder: "留空表示无鉴权",
                        oninput: move |v| {
                            config.write().secret = v;
                            saved.set(false);
                        },
                    }
                    div { class: "flex items-center gap-4 pt-1",
                        button {
                            class: "px-6 py-2 border border-[#e3000f] text-[#e3000f] text-sm uppercase tracking-[0.15em] hover:bg-[#e3000f] hover:text-white disabled:opacity-40 transition-colors",
                            disabled: core_busy(),
                            onclick: redownload_core,
                            if core_busy() { "处理中…" } else { "删除并重新下载核心" }
                        }
                        if let Some(s) = core_status() {
                            span { class: "text-xs text-neutral-600", "{s}" }
                        }
                    }
                }
            }

            // 系统区块:开机启动 + BLE 启动时自动启用保护
            section {
                div { class: "text-[11px] uppercase tracking-[0.2em] text-[#e3000f] border-b border-black pb-2 mb-4", "03 / 系统" }
                div { class: "space-y-4",
                    // 开机启动
                    label { class: "flex items-start gap-3 cursor-pointer",
                        input {
                            r#type: "checkbox",
                            class: "mt-1 w-4 h-4 accent-[#e3000f] cursor-pointer disabled:cursor-not-allowed",
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
                                div { class: "mt-1 text-xs text-[#e3000f]", "{e}" }
                            }
                        }
                    }

                    // BLE 启动时自动启用保护
                    label { class: "flex items-start gap-3 cursor-pointer",
                        input {
                            r#type: "checkbox",
                            class: "mt-1 w-4 h-4 accent-[#e3000f] cursor-pointer",
                            checked: ble_config().auto_protect_on_launch,
                            onchange: move |evt| {
                                let want = evt.value().parse::<bool>().unwrap_or(false);
                                ble_config.write().auto_protect_on_launch = want;
                                let _ = ble_config.read().save();
                            },
                        }
                        div { class: "flex-1",
                            div { class: "text-sm font-medium", "打开应用时自动启用蓝牙锁屏保护" }
                            div { class: "mt-1 text-xs text-neutral-500 leading-relaxed",
                                "需先在 04 / 蓝牙锁屏 页绑定设备。启动后做一次 3 秒 probe,扫到目标且 RSSI 达阈值则直接进入保护中。"
                            }
                        }
                    }
                }
            }

            div { class: "flex items-center gap-4",
                button {
                    class: "px-8 py-2.5 bg-black text-white text-sm uppercase tracking-[0.15em] hover:bg-[#e3000f] transition-colors",
                    onclick: save,
                    "保存"
                }
                if saved() {
                    span { class: "text-xs uppercase tracking-[0.12em] text-[#e3000f]", "已保存" }
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
                class: "w-full px-0 py-2 bg-transparent border-0 border-b border-black rounded-none outline-none text-base focus:border-[#e3000f] transition-colors",
                value,
                placeholder,
                oninput: move |e| oninput.call(e.value()),
            }
        }
    }
}
