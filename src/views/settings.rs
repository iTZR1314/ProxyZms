use crate::autostart;
use crate::bootstrap;
use crate::config::AppConfig;
use crate::diag;
#[cfg(target_os = "macos")]
use crate::helper;
use crate::mihomo::kernel;
use dioxus::prelude::*;

#[cfg(target_os = "macos")]
fn helper_may_own_tun() -> bool {
    helper::is_installed()
}

#[cfg(not(target_os = "macos"))]
fn helper_may_own_tun() -> bool {
    false
}

/// 设置页。FFI 内核后大幅变轻 —— mihomo 二进制 / controller secret / 重下载
/// 这些概念都不复存在,留下的只是订阅源 + 应用行为偏好 + 数据目录覆盖。
#[component]
pub fn Settings() -> Element {
    let mut config = use_context::<Signal<AppConfig>>();
    let mut share_port_input = use_signal(|| config.read().share_port.to_string());
    let lifecycle = use_context::<crate::Lifecycle>();
    let lifecycle_for_update = lifecycle.clone();
    let lifecycle_for_restart = lifecycle.clone();
    let mut tun_state = use_context::<crate::TunState>().0;
    let mut autostart_enabled = use_signal(autostart::is_enabled);
    let mut autostart_error = use_signal(|| None::<String>);
    let autostart_supported = autostart::is_supported();
    let mut saved = use_signal(|| false);
    let mut sub_status = use_signal(|| None::<String>);
    let mut updating = use_signal(|| false);
    let mut restart_status = use_signal(|| None::<String>);
    let mut restarting = use_signal(|| false);
    let mut applying_share = use_signal(|| false);
    let mut share_status = use_signal(|| None::<String>);

    // 下载订阅 → 写 config.yaml → apply_config(新内核直接接管,无需重启进程)
    let update_sub = move |_| {
        if updating() {
            return;
        }
        updating.set(true);
        sub_status.set(None);
        let lifecycle = lifecycle_for_update.clone();
        spawn(async move {
            let (url, work_dir) = {
                let c = config.read();
                (c.subscription_url.clone(), c.work_dir.clone())
            };
            match bootstrap::write_subscription(&url, &work_dir).await {
                Ok(()) => {
                    let yaml = match bootstrap::read_config(&work_dir) {
                        Ok(y) => y,
                        Err(e) => {
                            sub_status.set(Some(e));
                            updating.set(false);
                            return;
                        }
                    };
                    // apply_config 会重建 listener。macOS helper 持有独立 utun fd,
                    // 必须先关 TUN 释放系统路由,否则旧路由会指向已关闭的 Go listener。
                    let cfg_snapshot = config.read().clone();
                    let cleanup_tun = tun_state() || helper_may_own_tun();
                    if cleanup_tun {
                        sub_status.set(Some("正在关闭 TUN 并应用订阅…".to_string()));
                        let disable_result = lifecycle.0.set_tun(false, &cfg_snapshot).await;
                        let core_running = kernel().is_running().await.unwrap_or(false);
                        if let Err(e) = disable_result {
                            // 内核已停止时允许后面重新 init;helper 清理失败或活内核拒绝关闭则中止。
                            if e.contains("TUN helper 清理失败") || core_running {
                                sub_status.set(Some(format!("关闭 TUN 失败，订阅未应用:{e}")));
                                updating.set(false);
                                return;
                            }
                            eprintln!("[zms] core already stopped before subscription apply: {e}");
                        }
                    }
                    tun_state.set(false);
                    {
                        let mut cfg = config.write();
                        cfg.tun_enable = false;
                        let _ = cfg.save();
                    }
                    let home = bootstrap::effective_data_dir(&work_dir);
                    if !kernel().is_running().await.unwrap_or(false) {
                        if let Err(e) = kernel().init(&home).await {
                            sub_status.set(Some(format!("内核初始化失败:{e}")));
                            updating.set(false);
                            return;
                        }
                    }
                    let selected = config.read().selected_map.clone();
                    match kernel().apply_config(&yaml, &selected).await {
                        Ok(warning) => {
                            let preferences = config.read().clone();
                            match kernel()
                                .set_lan_share(preferences.share_lan, preferences.share_port)
                                .await
                            {
                                Ok(()) => {
                                    let suffix = warning
                                        .filter(|w| !w.is_empty())
                                        .map(|w| format!("(应用走默认兜底:{w})"))
                                        .unwrap_or_default();
                                    sub_status.set(Some(format!(
                                        "已更新订阅并应用{suffix};TUN 已关闭,可重新开启"
                                    )));
                                }
                                Err(e) => sub_status
                                    .set(Some(format!("订阅已应用,但恢复局域网共享设置失败:{e}"))),
                            }
                        }
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
        let lifecycle = lifecycle_for_restart.clone();
        spawn(async move {
            let work_dir = config.read().work_dir.clone();
            let cfg_snapshot = config.read().clone();
            let cleanup_tun = tun_state() || helper_may_own_tun();
            if cleanup_tun {
                if let Err(e) = lifecycle.0.set_tun(false, &cfg_snapshot).await {
                    let core_running = kernel().is_running().await.unwrap_or(false);
                    if e.contains("TUN helper 清理失败") || core_running {
                        restart_status.set(Some(format!("关闭 TUN 失败，未重启内核:{e}")));
                        restarting.set(false);
                        return;
                    }
                    eprintln!("[zms] core already stopped before kernel restart: {e}");
                }
            }
            tun_state.set(false);
            {
                let mut cfg = config.write();
                cfg.tun_enable = false;
                let _ = cfg.save();
            }
            let _ = kernel().shutdown().await;
            restart_status.set(Some("重新初始化…".to_string()));
            let home = bootstrap::effective_data_dir(&work_dir);
            if let Err(e) = kernel().init(&home).await {
                restart_status.set(Some(format!("初始化失败:{e}")));
                restarting.set(false);
                return;
            }
            let yaml = match bootstrap::read_config(&work_dir) {
                Ok(y) => y,
                Err(e) => {
                    restart_status.set(Some(e));
                    restarting.set(false);
                    return;
                }
            };
            let selected = config.read().selected_map.clone();
            match kernel().apply_config(&yaml, &selected).await {
                Ok(_) => {
                    let preferences = config.read().clone();
                    match kernel()
                        .set_lan_share(preferences.share_lan, preferences.share_port)
                        .await
                    {
                        Ok(()) => restart_status.set(Some("已重启内核".to_string())),
                        Err(e) => restart_status
                            .set(Some(format!("内核已启动,但恢复局域网共享设置失败:{e}"))),
                    }
                }
                Err(e) => restart_status.set(Some(format!("应用配置失败:{e}"))),
            }
            restarting.set(false);
        });
    };

    let save = move |_| {
        let _ = config.read().save();
        saved.set(true);
    };

    let apply_share = move |_| {
        if applying_share() {
            return;
        }
        let Some(port) = share_port_input()
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
        else {
            share_status.set(Some("端口必须是 1 到 65535 之间的整数".to_string()));
            return;
        };
        let mut snapshot = config.read().clone();
        snapshot.share_port = port;
        applying_share.set(true);
        share_status.set(Some("正在应用 Mihomo 共享设置…".to_string()));
        spawn(async move {
            match kernel()
                .set_lan_share(snapshot.share_lan, snapshot.share_port)
                .await
            {
                Ok(()) => {
                    let _ = config.read().save();
                    saved.set(true);
                    share_status.set(Some(if snapshot.share_lan {
                        format!("已开放 mixed-port {}", snapshot.share_port)
                    } else {
                        format!(
                            "局域网访问已关闭，本机 mixed-port 为 {}",
                            snapshot.share_port
                        )
                    }));
                }
                Err(e) => share_status.set(Some(format!("应用共享设置失败:{e}"))),
            }
            applying_share.set(false);
        });
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

            // Mihomo 原生 LAN 共享:allow-lan + mixed-port,不额外创建代理服务。
            section {
                div { class: "text-[11px] uppercase tracking-[0.2em] text-[var(--accent)] border-b border-black pb-2 mb-4", "03 / 局域网共享" }
                div { class: "space-y-4",
                    label { class: "flex items-start gap-3 cursor-pointer",
                        input {
                            r#type: "checkbox",
                            class: "mt-1 w-4 h-4 accent-[var(--accent)] cursor-pointer",
                            checked: config().share_lan,
                            onchange: move |evt| {
                                config.write().share_lan = evt.value().parse::<bool>().unwrap_or(false);
                                saved.set(false);
                            },
                        }
                        div { class: "flex-1",
                            div { class: "text-sm font-medium", "允许局域网设备使用代理" }
                            div { class: "mt-1 text-xs text-neutral-500 leading-relaxed",
                                "使用 Mihomo 原生 allow-lan。开启后，同一局域网内的设备可连接本机 LAN IP 与下方端口。"
                            }
                        }
                    }
                    label { class: "block max-w-xs",
                        span { class: "block text-[11px] uppercase tracking-[0.15em] text-neutral-500 mb-2", "Mihomo mixed-port" }
                        input {
                            r#type: "number",
                            min: "1",
                            max: "65535",
                            step: "1",
                            class: "w-full px-0 py-2 bg-transparent border-0 border-b border-black rounded-none outline-none text-base focus:border-[var(--accent)] transition-colors",
                            value: share_port_input(),
                            oninput: move |evt| {
                                let value = evt.value();
                                share_port_input.set(value.clone());
                                if let Ok(port) = value.parse::<u16>() {
                                    if port > 0 {
                                        config.write().share_port = port;
                                        saved.set(false);
                                    }
                                }
                            },
                        }
                    }
                    div { class: "text-xs text-neutral-500 leading-relaxed",
                        "局域网客户端填写“本机 LAN IP:端口”。开启 Mihomo 原生 allow-lan 后，订阅 YAML 中配置的其他入站端口也可能对局域网开放；如需保护，请在 Mihomo 配置中设置 authentication，并只在可信网络使用。"
                    }
                    div { class: "flex items-center gap-4 pt-1",
                        button {
                            class: "px-6 py-2 border border-[var(--accent)] text-[var(--accent)] text-sm uppercase tracking-[0.15em] hover:bg-[var(--accent)] hover:text-white disabled:opacity-40 transition-colors",
                            disabled: applying_share() || restarting() || updating(),
                            onclick: apply_share,
                            if applying_share() { "应用中…" } else { "应用共享设置" }
                        }
                        if let Some(s) = share_status() {
                            span { class: "text-xs text-neutral-600", "{s}" }
                        }
                    }
                }
            }

            // Helper 组件仅在 macOS 构建出内容,Windows 编译不包含其控制逻辑。
            MacosHelperSettings {}

            // 系统区块:开机启动
            section {
                div { class: "text-[11px] uppercase tracking-[0.2em] text-[var(--accent)] border-b border-black pb-2 mb-4", "05 / 系统" }
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
                            div { class: "text-sm font-medium", "开机启动施展魔法" }
                            div { class: "mt-1 text-xs text-neutral-500 leading-relaxed",
                                if autostart_supported {
                                    "登录系统后静默拉起并自动开启 TUN（不显示主界面，仅驻留托盘）。macOS 写入 LaunchAgent；Windows 写入注册表 Run 项。"
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

            DiagnosticsSettings {}

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
fn MacosHelperSettings() -> Element {
    #[cfg(target_os = "macos")]
    {
        let mut helper_status = use_signal(|| None::<String>);
        let mut helper_busy = use_signal(|| false);
        let mut helper_installed = use_signal(helper::is_installed);

        let install_helper = move |_| {
            if helper_busy() {
                return;
            }
            helper_busy.set(true);
            helper_status.set(Some("等待授权…".to_string()));
            spawn(async move {
                let res = tokio::task::spawn_blocking(helper::install).await;
                match res {
                    Ok(Ok(())) => {
                        helper_status.set(Some("已安装(launchd 常驻)".to_string()));
                        helper_installed.set(true);
                    }
                    Ok(Err(e)) => helper_status.set(Some(e)),
                    Err(_) => helper_status.set(Some("安装任务异常".to_string())),
                }
                helper_busy.set(false);
            });
        };

        let uninstall_helper = move |_| {
            if helper_busy() {
                return;
            }
            helper_busy.set(true);
            helper_status.set(Some("等待授权…".to_string()));
            spawn(async move {
                let res = tokio::task::spawn_blocking(helper::uninstall).await;
                match res {
                    Ok(Ok(())) => {
                        helper_status.set(Some("已卸载".to_string()));
                        helper_installed.set(false);
                    }
                    Ok(Err(e)) => helper_status.set(Some(e)),
                    Err(_) => helper_status.set(Some("卸载任务异常".to_string())),
                }
                helper_busy.set(false);
            });
        };

        rsx! {
            section {
                div { class: "text-[11px] uppercase tracking-[0.2em] text-[var(--accent)] border-b border-black pb-2 mb-4", "04 / macOS Helper" }
                div { class: "space-y-4",
                    div { class: "text-xs text-neutral-600 leading-relaxed",
                        "TUN 模式在 macOS 上需要 root 权限建立 utun 虚拟网卡。本程序通过独立的 \
                         root helper 子进程 + 文件描述符传递(SCM_RIGHTS)实现,主 App 全程不提权。\
                         首次开启 TUN 时会弹一次密码框安装 helper,之后 launchd 常驻无需再授权。"
                    }
                    div { class: "flex items-center gap-4 pt-1",
                        if helper_installed() {
                            button {
                                class: "px-6 py-2 border border-neutral-400 text-neutral-600 text-sm uppercase tracking-[0.15em] hover:border-[var(--accent)] hover:text-[var(--accent)] disabled:opacity-40 transition-colors",
                                disabled: helper_busy(),
                                onclick: uninstall_helper,
                                if helper_busy() { "处理中…" } else { "卸载 Helper" }
                            }
                            span { class: "text-xs text-neutral-500", "已安装" }
                        } else {
                            button {
                                class: "px-6 py-2 border border-[var(--accent)] text-[var(--accent)] text-sm uppercase tracking-[0.15em] hover:bg-[var(--accent)] hover:text-white disabled:opacity-40 transition-colors",
                                disabled: helper_busy(),
                                onclick: install_helper,
                                if helper_busy() { "处理中…" } else { "安装 Helper" }
                            }
                            span { class: "text-xs text-neutral-500", "未安装(开 TUN 时会自动装)" }
                        }
                        if let Some(s) = helper_status() {
                            span { class: "text-xs text-neutral-500", "{s}" }
                        }
                    }
                }
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    rsx! {}
}

fn fmt_value(v: i64, kb: bool) -> String {
    if kb {
        format!("{:.1} MB", v as f64 / 1024.0)
    } else {
        v.to_string()
    }
}

fn fmt_delta(v: i64, kb: bool) -> String {
    if kb {
        format!("{:+.1} MB", v as f64 / 1024.0)
    } else {
        format!("{v:+}")
    }
}

/// 泄漏排查面板:先「记为基线」,做一轮操作后「刷新」,看相对基线的增量。
#[component]
fn DiagnosticsSettings() -> Element {
    let mut snap = use_signal(|| None::<Result<diag::Snapshot, String>>);
    let mut base = use_signal(diag::baseline);
    let mut loading = use_signal(|| false);

    // 每次进入设置页自动取一次(用户通常是在别的页面操作完才切过来)
    use_future(move || async move {
        snap.set(Some(diag::snapshot().await));
    });

    let refresh = move |_| {
        if loading() {
            return;
        }
        loading.set(true);
        spawn(async move {
            snap.set(Some(diag::snapshot().await));
            loading.set(false);
        });
    };

    let mark_base = move |_| {
        if let Some(Ok(s)) = snap() {
            diag::set_baseline(s.clone());
            base.set(Some(s));
        }
    };

    let table = match snap() {
        Some(Ok(cur)) => Some(Ok(diag::rows(&cur, base().as_ref()))),
        Some(Err(e)) => Some(Err(e)),
        None => None,
    };

    rsx! {
        section {
            div { class: "text-[11px] uppercase tracking-[0.2em] text-[var(--accent)] border-b border-black pb-2 mb-4", "06 / 诊断" }
            div { class: "space-y-4",
                div { class: "text-xs text-neutral-600 leading-relaxed",
                    "排查资源泄漏:关闭 TUN → 点「记为基线」→ 开关 TUN 10 次并停在关闭 → 等 5 秒 → 回到本页点「刷新」。\
                     看「相对基线」一列:某一项随操作次数成比例上涨(例如 10 次涨约 10 或 20)就是泄漏;\
                     内存 RSS 小幅波动、socket 随流量变化属正常。"
                }
                match table {
                    Some(Ok(rows)) => rsx! {
                        div { class: "text-sm",
                            div { class: "flex text-[11px] uppercase tracking-[0.15em] text-neutral-500 pb-1 border-b border-neutral-300",
                                div { class: "flex-1", "指标" }
                                div { class: "w-28 text-right", "当前" }
                                div { class: "w-28 text-right", "相对基线" }
                            }
                            for r in rows {
                                div { class: "flex py-1 border-b border-neutral-200 tabular-nums",
                                    div {
                                        class: if r.sub { "flex-1 pl-4 text-neutral-500" } else { "flex-1" },
                                        "{r.label}"
                                    }
                                    div { class: "w-28 text-right", "{fmt_value(r.cur, r.kb)}" }
                                    div {
                                        class: if r.delta.unwrap_or(0) > 0 { "w-28 text-right text-[var(--accent)]" } else { "w-28 text-right text-neutral-500" },
                                        match r.delta {
                                            Some(d) => rsx! { "{fmt_delta(d, r.kb)}" },
                                            None => rsx! { "—" },
                                        }
                                    }
                                }
                            }
                        }
                    },
                    Some(Err(e)) => rsx! {
                        div { class: "text-xs text-[var(--accent)]", "读取诊断数据失败:{e}" }
                    },
                    None => rsx! {
                        div { class: "text-xs text-neutral-500", "读取中…" }
                    },
                }
                div { class: "flex items-center gap-4 pt-1",
                    button {
                        class: "px-6 py-2 border border-[var(--accent)] text-[var(--accent)] text-sm uppercase tracking-[0.15em] hover:bg-[var(--accent)] hover:text-white disabled:opacity-40 transition-colors",
                        disabled: loading(),
                        onclick: refresh,
                        if loading() { "读取中…" } else { "刷新" }
                    }
                    button {
                        class: "px-6 py-2 border border-neutral-400 text-neutral-600 text-sm uppercase tracking-[0.15em] hover:border-[var(--accent)] hover:text-[var(--accent)] transition-colors",
                        onclick: mark_base,
                        "记为基线"
                    }
                    if base().is_some() {
                        span { class: "text-xs text-neutral-500", "已设置基线" }
                    }
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
