//! 绑定 / 锁屏页共用的小组件:[`ThresholdControls`] 滑块组 + [`RssiChart`] 实时曲线。
//!
//! 故意做成纯展示组件 —— 不读 context,只接 props,父组件负责把 [`BleLockConfig`]
//! 或 [`BleSession`] 的数据拍扁后传进来。这样未来想换数据源(比如批量 undo / 多设备)
//! 不影响这里。

use dioxus::prelude::*;

use crate::ble_lock::RSSI_HISTORY_LEN;

// ─────────────────────────────────────────────────────────────────────────────
// 阈值滑块组

/// 两条 range slider:`lock_rssi` 和 `missing_limit`。
/// 父组件用 `EventHandler` 接住每次拖动的新值,自己决定是否落盘。
#[component]
pub fn ThresholdControls(
    lock_rssi: i16,
    missing_limit: u32,
    on_rssi_change: EventHandler<i16>,
    on_limit_change: EventHandler<u32>,
) -> Element {
    rsx! {
        div { class: "space-y-5",

            // ── 信号阈值 ──
            div {
                div { class: "flex items-baseline justify-between gap-3 mb-1",
                    span { class: "text-[11px] uppercase tracking-[0.2em] text-neutral-500",
                        "信号阈值 lock_rssi"
                    }
                    span { class: "text-2xl font-bold tabular-nums tracking-tighter leading-none",
                        "{lock_rssi}"
                        span { class: "ml-1 text-[10px] uppercase tracking-[0.2em] text-neutral-400 font-normal",
                            "dBm"
                        }
                    }
                }
                input {
                    r#type: "range",
                    min: "-100",
                    max: "-30",
                    step: "1",
                    value: "{lock_rssi}",
                    class: "w-full accent-[#e3000f]",
                    oninput: move |e| {
                        if let Ok(v) = e.value().parse::<i16>() {
                            on_rssi_change.call(v);
                        }
                    },
                }
                div { class: "flex justify-between text-[10px] uppercase tracking-[0.2em] text-neutral-400 mt-1",
                    span { "-100 / 远 · 宽容" }
                    span { "-30 / 近 · 严格" }
                }
            }

            // ── 容忍丢失次数 ──
            div {
                div { class: "flex items-baseline justify-between gap-3 mb-1",
                    span { class: "text-[11px] uppercase tracking-[0.2em] text-neutral-500",
                        "容忍丢失次数 missing_limit"
                    }
                    span { class: "text-2xl font-bold tabular-nums tracking-tighter leading-none",
                        "{missing_limit}"
                    }
                }
                input {
                    r#type: "range",
                    min: "1",
                    max: "20",
                    step: "1",
                    value: "{missing_limit}",
                    class: "w-full accent-[#e3000f]",
                    oninput: move |e| {
                        if let Ok(v) = e.value().parse::<u32>() {
                            on_limit_change.call(v);
                        }
                    },
                }
                div { class: "flex justify-between text-[10px] uppercase tracking-[0.2em] text-neutral-400 mt-1",
                    span { "1 / 敏感 · 易锁屏" }
                    span { "20 / 宽容 · 难锁屏" }
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RSSI 滚动曲线图(SVG)

/// 设计要点:
/// - X 轴 = 时间,左旧右新;每个样本固定间距(0.5 Hz),无需画刻度。
/// - Y 轴 = RSSI,顶部 -30 dBm(强),底部 -100 dBm(弱)。
/// - 虚线 = `threshold`(`lock_rssi`),直观看到信号穿越阈值的时刻。
/// - 黑色路径 = 实测样本,跨 None 时断开(每个 Some 段独立 path 子段)。
/// - 红色底点 = None 样本,表示这一拍完全没扫到设备。
#[component]
pub fn RssiChart(history: Vec<Option<i16>>, threshold: i16) -> Element {
    const W: i32 = 600;
    const H: i32 = 100;
    const RSSI_TOP: f32 = -30.0;
    const RSSI_BOT: f32 = -100.0;

    let rssi_to_y = |rssi: i16| -> f32 {
        let clamped = (rssi as f32).clamp(RSSI_BOT, RSSI_TOP);
        let frac = (RSSI_TOP - clamped) / (RSSI_TOP - RSSI_BOT);
        frac * H as f32
    };
    let idx_to_x = |idx: usize| -> f32 {
        if RSSI_HISTORY_LEN <= 1 {
            0.0
        } else {
            (idx as f32 / (RSSI_HISTORY_LEN - 1) as f32) * W as f32
        }
    };

    // 构造路径 d:Some 段连续 L,遇 None 段下次 Some 用 M 起新段(视觉上断开)。
    let mut path_d = String::new();
    let mut last_was_some = false;
    for (i, sample) in history.iter().enumerate() {
        if let Some(rssi) = sample {
            let cmd = if last_was_some { 'L' } else { 'M' };
            path_d.push_str(&format!(
                "{cmd} {:.1} {:.1} ",
                idx_to_x(i),
                rssi_to_y(*rssi)
            ));
            last_was_some = true;
        } else {
            last_was_some = false;
        }
    }

    let threshold_y = rssi_to_y(threshold);

    let none_xs: Vec<f32> = history
        .iter()
        .enumerate()
        .filter_map(|(i, s)| if s.is_none() { Some(idx_to_x(i)) } else { None })
        .collect();
    let some_pts: Vec<(f32, f32)> = history
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.map(|r| (idx_to_x(i), rssi_to_y(r))))
        .collect();

    rsx! {
        svg {
            width: "100%",
            height: "{H}",
            view_box: "0 0 {W} {H}",
            style: "background: #fafafa; display: block;",

            // 阈值参考线(虚线)
            line {
                x1: "0",
                x2: "{W}",
                y1: "{threshold_y:.1}",
                y2: "{threshold_y:.1}",
                stroke: "#bbb",
                stroke_dasharray: "4 3",
                stroke_width: "1",
            }

            // 信号路径(黑)
            path {
                d: "{path_d}",
                stroke: "#111",
                stroke_width: "1.5",
                fill: "none",
            }

            // 实测样本点(黑)
            for (x , y) in some_pts {
                circle {
                    cx: "{x:.1}",
                    cy: "{y:.1}",
                    r: "2",
                    fill: "#111",
                }
            }

            // None 样本(红点贴底)
            for x in none_xs {
                circle {
                    cx: "{x:.1}",
                    cy: "{H - 3}",
                    r: "2.5",
                    fill: "#e3000f",
                }
            }
        }
    }
}
