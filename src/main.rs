// objc 0.2 的 msg_send!/class! 宏内部使用 cfg(cargo-clippy),会触发 unexpected_cfgs
#![allow(unexpected_cfgs)]
// Windows 发布构建:用 windows 子系统启动,避免主程序弹出黑色控制台窗口
// (debug 构建保留控制台,方便看日志)
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use dioxus::prelude::*;

mod bootstrap;
mod config;
mod format;
mod mihomo;
mod views;

use config::AppConfig;
use mihomo::Controller;
use views::{ConnectionsView, Flow, Settings};

/// 全局共享的 TUN 开关状态:UI(TunControls)与系统托盘共用同一信号,保证一致。
#[derive(Clone, Copy)]
pub struct TunState(pub Signal<bool>);

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(Shell)]
    #[route("/")]
    FlowPage {},
    #[route("/connections")]
    Connections {},
    #[route("/settings")]
    SettingsPage {},
}

// 系统托盘(macOS 菜单栏 / Windows 系统托盘)图标:随 TUN 状态切换
#[cfg(feature = "desktop")]
const ON_PNG: &[u8] = include_bytes!("../assets/on.png");
#[cfg(feature = "desktop")]
const OFF_PNG: &[u8] = include_bytes!("../assets/off.png");

/// 侧边栏 logo:编译期内嵌为 base64 data URI(单文件 exe,不外挂 assets)。
fn fmr_logo_uri() -> &'static str {
    use base64::Engine;
    use std::sync::OnceLock;
    static URI: OnceLock<String> = OnceLock::new();
    URI.get_or_init(|| {
        let b64 = base64::engine::general_purpose::STANDARD
            .encode(include_bytes!("../assets/fmr-logo.png"));
        format!("data:image/png;base64,{b64}")
    })
}

// ===== 单实例(仅发布版)=====
// 用固定 loopback 端口当"锁 + IPC":连得上=已有实例(通知其显示后退出);
// 连不上=本进程为主实例(绑定监听,收到请求则置位 SHOW_REQUESTED 让窗口显示)。
#[cfg(not(debug_assertions))]
static SHOW_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 返回 true=本进程为主实例应继续;false=已有实例(已通知其显示),应退出。
#[cfg(not(debug_assertions))]
fn acquire_single_instance() -> bool {
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::Ordering;
    const ADDR: &str = "127.0.0.1:53682";
    // 已有实例:通知它显示窗口,本进程退出
    if let Ok(mut stream) = TcpStream::connect(ADDR) {
        let _ = stream.write_all(b"show");
        return false;
    }
    // 主实例:占住端口,起线程接收"显示"请求
    if let Ok(listener) = TcpListener::bind(ADDR) {
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if conn.is_ok() {
                    SHOW_REQUESTED.store(true, Ordering::SeqCst);
                }
            }
        });
    }
    true
}

fn main() {
    // 单实例:已有实例则通知其显示并退出
    #[cfg(not(debug_assertions))]
    if !acquire_single_instance() {
        return;
    }

    // Ctrl-C / SIGTERM 时:先杀掉 mihomo 内核再退出(此路径不会触发 Drop)
    let _ = ctrlc::set_handler(|| {
        mihomo::process::kill_tracked();
        std::process::exit(0);
    });

    #[cfg(feature = "desktop")]
    {
        use dioxus::desktop::tao::dpi::LogicalSize;
        use dioxus::desktop::tao::window::Icon;
        use dioxus::desktop::{Config, WindowBuilder, WindowCloseBehaviour};

        // 把 fmr.png 解码成窗口图标(Windows 任务栏/标题栏、Linux 标题栏)
        let icon = {
            let bytes = include_bytes!("../assets/fmr.png");
            image::load_from_memory(bytes)
                .map(|img| img.into_rgba8())
                .ok()
                .and_then(|rgba| {
                    let (w, h) = rgba.dimensions();
                    Icon::from_rgba(rgba.into_raw(), w, h).ok()
                })
        };

        let window = WindowBuilder::new()
            .with_title("暴暴龙专属")
            .with_window_icon(icon)
            // 默认窗口宽高(逻辑像素),并设置最小尺寸
            .with_inner_size(LogicalSize::new(900.0, 825.0))
            .with_min_inner_size(LogicalSize::new(720.0, 480.0));

        // 把 CSS 内容直接内联进初始 HTML 的 <head>(编译期 include_str! 嵌入)。
        // 不依赖 asset 路径解析 —— 发布版/开发版表现一致;且渲染阻塞,无 FOUC。
        let custom_head = format!(
            "<style>{}</style><style>{}</style>",
            include_str!("../assets/main.css"),
            include_str!("../assets/tailwind.css"),
        );

        // macOS 不进入下方 with_menu 分支,故 mut 在 macOS 上"未使用",单独抑制
        #[cfg_attr(target_os = "macos", allow(unused_mut))]
        let mut config = Config::new()
            .with_window(window)
            // 关闭窗口时隐藏而非退出,程序留在后台(托盘)
            .with_close_behaviour(WindowCloseBehaviour::WindowHides)
            // 初始背景设为白色,避免首帧黑/透明闪一下
            .with_background_color((255, 255, 255, 255))
            .with_custom_head(custom_head);

        // 隐藏 Windows/Linux 上的默认菜单栏(Window / Edit / Help)。
        // macOS 保留:其 Edit 菜单提供 Cmd+C/V/X 等复制粘贴快捷键。
        #[cfg(not(target_os = "macos"))]
        {
            config = config.with_menu(None::<dioxus::desktop::muda::Menu>);
        }

        dioxus::LaunchBuilder::new().with_cfg(config).launch(App);
    }
    #[cfg(not(feature = "desktop"))]
    dioxus::launch(App);
}

/// 把 fmr.png 加工成 macOS 风格图标(圆角 squircle + 四周留白),返回 PNG 字节。
#[cfg(target_os = "macos")]
fn rounded_icon_png() -> Option<Vec<u8>> {
    use image::{imageops::FilterType, ExtendedColorType, ImageBuffer, ImageEncoder, Rgba, RgbaImage};
    let src = image::load_from_memory(include_bytes!("../assets/fmr.png"))
        .ok()?
        .to_rgba8();
    let canvas = 1024u32;
    let margin = 100u32; // macOS 图标网格的留白
    let content = canvas - margin * 2; // 824
    let radius = content as f32 * 0.2237; // 近似 Apple squircle 圆角半径
    let resized = image::imageops::resize(&src, content, content, FilterType::Lanczos3);
    let mut out: RgbaImage = ImageBuffer::from_pixel(canvas, canvas, Rgba([0, 0, 0, 0]));
    let half = content as f32 / 2.0;
    for y in 0..content {
        for x in 0..content {
            // 圆角矩形有符号距离场 → 边缘抗锯齿覆盖率
            let px = (x as f32 + 0.5) - half;
            let py = (y as f32 + 0.5) - half;
            let qx = px.abs() - half + radius;
            let qy = py.abs() - half + radius;
            let d = qx.max(qy).min(0.0)
                + (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt()
                - radius;
            let cov = (0.5 - d).clamp(0.0, 1.0);
            if cov > 0.0 {
                let mut p = *resized.get_pixel(x, y);
                p[3] = (p[3] as f32 * cov) as u8;
                out.put_pixel(x + margin, y + margin, p);
            }
        }
    }
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buf)
        .write_image(&out, canvas, canvas, ExtendedColorType::Rgba8)
        .ok()?;
    Some(buf)
}

/// macOS:运行时把(加了圆角的)图标设为 Dock 图标(dev 模式也生效,不依赖打包)。
#[cfg(target_os = "macos")]
fn set_dock_icon() {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    type Id = *mut Object;
    let Some(png) = rounded_icon_png() else {
        return;
    };
    unsafe {
        let data: Id = msg_send![class!(NSData),
            dataWithBytes: png.as_ptr() as *const std::os::raw::c_void
            length: png.len()];
        let image: Id = msg_send![class!(NSImage), alloc];
        let image: Id = msg_send![image, initWithData: data];
        if !image.is_null() {
            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, setApplicationIconImage: image];
        }
    }
}

/// 把 PNG 字节解码成托盘图标。
#[cfg(feature = "desktop")]
fn tray_icon_from_png(bytes: &[u8]) -> Option<dioxus::desktop::trayicon::Icon> {
    let rgba = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (w, h) = rgba.dimensions();
    dioxus::desktop::trayicon::Icon::from_rgba(rgba.into_raw(), w, h).ok()
}

/// macOS:切换程序坞图标可见性(Regular=显示,Accessory=隐藏成菜单栏代理)。
#[cfg(target_os = "macos")]
fn set_dock_visible(visible: bool) {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    // NSApplicationActivationPolicy: Regular = 0, Accessory = 1
    let policy: i64 = if visible { 0 } else { 1 };
    unsafe {
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, setActivationPolicy: policy];
    }
}

/// 处理托盘菜单项点击:启动/停止(切 TUN)/ 退出。
/// muda 与 tray 菜单事件是同一类型、同一全局 handler(后注册覆盖前者),
/// 故 tray + muda 两个 hook 都注册并共用此函数,保证一定收到。
#[cfg(feature = "desktop")]
fn handle_menu_select(
    id: &dioxus::desktop::muda::MenuId,
    toggle_id: &dioxus::desktop::muda::MenuId,
    quit_id: &dioxus::desktop::muda::MenuId,
    config: Signal<AppConfig>,
    mut tun_state: Signal<bool>,
    controller: &Controller,
) {
    eprintln!("[zms] menu select: id={id:?}");
    if id == toggle_id {
        let (url, secret) = {
            let c = config.read();
            (c.controller_url.clone(), c.secret.clone())
        };
        let target = !tun_state();
        spawn(async move {
            // 成功才落定共享状态(失败保持原状),不乐观更新,避免托盘/UI 图标跳变
            if mihomo::ApiClient::new(url, secret).set_tun(target).await.is_ok() {
                tun_state.set(target);
            }
        });
    } else if id == quit_id {
        eprintln!("[zms] 退出:停止内核并退出程序");
        controller.stop();
        mihomo::process::kill_tracked();
        std::process::exit(0);
    }
}

#[component]
fn App() -> Element {
    // 全局状态:配置(从磁盘加载)+ mihomo 进程控制器 + 共享 TUN 状态
    let config = use_context_provider(|| Signal::new(AppConfig::load()));
    use_context_provider(Controller::default);
    let mut tun_state = use_context_provider(|| TunState(Signal::new(false))).0;

    // 挂载后设置 Dock 图标(此时 NSApplication 已就绪)
    use_effect(|| {
        #[cfg(target_os = "macos")]
        set_dock_icon();
    });

    // 唯一的 TUN 状态源:轮询 /configs 写入共享信号(UI 与托盘都读它)
    use_future(move || async move {
        loop {
            let (url, secret) = {
                let c = config.read();
                (c.controller_url.clone(), c.secret.clone())
            };
            let on = match mihomo::ApiClient::new(url, secret).configs().await {
                Ok(c) => c.tun.enable,
                Err(e) => {
                    eprintln!("[zms] 轮询 /configs 失败: {e}");
                    false
                }
            };
            if tun_state() != on {
                eprintln!("[zms] TUN 状态轮询更新: {on}");
                tun_state.set(on);
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });

    // 系统托盘图标:随共享 TUN 状态切换 on/off
    #[cfg(feature = "desktop")]
    {
        use dioxus::desktop::muda::{Menu, MenuItem, PredefinedMenuItem};
        use dioxus::desktop::trayicon::{init_tray_icon, use_tray_icon};
        // 构建托盘右键菜单:启动/停止(切 TUN)+ 退出,仅一次
        let (toggle_item, quit_item) = use_hook(|| {
            let toggle = MenuItem::new("启动", true, None);
            let quit = MenuItem::new("退出", true, None);
            let menu = Menu::new();
            let _ = menu.append_items(&[&toggle, &PredefinedMenuItem::separator(), &quit]);
            init_tray_icon(menu, tray_icon_from_png(OFF_PNG));
            (toggle, quit)
        });
        let toggle_id = toggle_item.id().clone();
        let quit_id = quit_item.id().clone();
        let tray = use_tray_icon();

        // 状态变化时在主线程更新托盘图标 + 菜单项文字(非 Send 句柄,放 effect)
        let toggle_for_effect = toggle_item.clone();
        use_effect(move || {
            let on = tun_state();
            if let Some(t) = tray.as_ref() {
                let icon = if on {
                    tray_icon_from_png(ON_PNG)
                } else {
                    tray_icon_from_png(OFF_PNG)
                };
                let _ = t.set_icon(icon);
            }
            toggle_for_effect.set_text(if on { "停止" } else { "启动" });
        });

        // 关窗(WindowHides 已隐藏窗口)→ 额外隐藏程序坞图标
        use dioxus::desktop::tao::event::{Event, WindowEvent};
        use dioxus::desktop::trayicon::{MouseButton, MouseButtonState, TrayIconEvent};
        use dioxus::desktop::{
            use_muda_event_handler, use_tray_icon_event_handler, use_tray_menu_event_handler,
            use_window, use_wry_event_handler,
        };

        let win = use_window();
        use_wry_event_handler(move |event, _| {
            if let Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } = event
            {
                #[cfg(target_os = "macos")]
                set_dock_visible(false);
            }
        });

        // 点击托盘图标 → 重新显示窗口 + 恢复程序坞图标
        use_tray_icon_event_handler(move |event| {
            eprintln!("[zms] tray icon event: {event:?}");
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                #[cfg(target_os = "macos")]
                {
                    set_dock_visible(true);
                    set_dock_icon();
                }
                win.set_visible(true);
                win.set_focus();
            }
        });

        // 托盘右键菜单事件:tray + muda 两个 hook 都注册(全局 handler 只有一个生效,
        // 不确定是哪个,故都挂上,共用 handle_menu_select)
        let menu_ctrl = use_context::<Controller>();
        {
            let (tid, qid, ctrl) = (toggle_id.clone(), quit_id.clone(), menu_ctrl.clone());
            use_tray_menu_event_handler(move |e| {
                handle_menu_select(&e.id, &tid, &qid, config, tun_state, &ctrl)
            });
        }
        {
            let (tid, qid, ctrl) = (toggle_id.clone(), quit_id.clone(), menu_ctrl.clone());
            use_muda_event_handler(move |e| {
                handle_menu_select(&e.id, &tid, &qid, config, tun_state, &ctrl)
            });
        }

        // 单实例:另一个实例请求显示时,把本窗口拉到前台(仅发布版)
        #[cfg(not(debug_assertions))]
        {
            let win_show = use_window();
            use_future(move || {
                let win = win_show.clone();
                async move {
                    use std::sync::atomic::Ordering;
                    loop {
                        if SHOW_REQUESTED.swap(false, Ordering::SeqCst) {
                            #[cfg(target_os = "macos")]
                            set_dock_visible(true);
                            win.set_visible(true);
                            win.set_focus();
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    }
                }
            });
        }
    }

    rsx! {
        // CSS 已通过 with_custom_head 内联进 <head>;图标走 WindowBuilder,无需外链
        Router::<Route> {}
    }
}

/// 侧边栏 + 内容区的整体布局(Swiss 网格)。
#[component]
fn Shell() -> Element {
    rsx! {
        div { class: "flex h-screen bg-white text-neutral-900 overflow-hidden",
            // 导航栏:实心白底,右侧发丝分隔线。
            aside { class: "w-52 shrink-0 border-r border-black/15 bg-white flex flex-col",
                // 品牌区
                div { class: "px-6 py-8 border-b-2 border-black",
                    div { class: "text-2xl font-bold tracking-tighter leading-none", "Mihomo" }
                    div { class: "mt-2 text-[10px] uppercase tracking-[0.25em] text-neutral-500",
                        "Mihomo Controller"
                    }
                }
                // 编号导航
                nav { class: "flex-1 py-2",
                    NavItem { to: Route::FlowPage {}, index: "01", label: "流量" }
                    NavItem { to: Route::Connections {}, index: "02", label: "连接" }
                    NavItem { to: Route::SettingsPage {}, index: "03", label: "设置" }
                }
                // 页脚:点击图标跳转作者主页
                div { class: "mt-auto",
                    button {
                        class: "w-full flex flex-col items-center gap-2 px-6 py-5 hover:bg-neutral-50 transition-colors",
                        onclick: |_| {
                            let _ = webbrowser::open("https://zhoumaosen.top");
                        },
                        img { src: fmr_logo_uri(), class: "w-16 h-16", alt: "付满瑞印" }
                        div { class: "text-center leading-tight",
                            div { class: "text-xs uppercase tracking-[0.2em] text-neutral-600", "fumanrui" }
                            div { class: "text-[10px] uppercase tracking-[0.2em] text-neutral-400", "2026 v0.0.3" }
                        }
                    }
                }
            }
            main {
                class: "flex-1 min-w-0 overflow-y-auto overflow-x-hidden overscroll-none",
                Outlet::<Route> {}
            }
        }
    }
}

#[component]
fn NavItem(to: Route, index: String, label: String) -> Element {
    rsx! {
        Link {
            to,
            class: "flex items-baseline gap-4 px-6 py-3 border-l-4 border-transparent text-neutral-500 hover:text-black transition-colors",
            active_class: "border-[#e3000f] text-black",
            span { class: "text-[11px] tabular-nums text-neutral-400", "{index}" }
            span { class: "text-sm uppercase tracking-[0.15em]", "{label}" }
        }
    }
}

#[component]
fn FlowPage() -> Element {
    rsx! { Flow {} }
}

#[component]
fn Connections() -> Element {
    rsx! { ConnectionsView {} }
}

#[component]
fn SettingsPage() -> Element {
    rsx! { Settings {} }
}
