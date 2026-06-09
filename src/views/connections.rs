use crate::config::AppConfig;
use crate::format;
use crate::mihomo::types::Connections;
use crate::mihomo::ApiClient;
use dioxus::prelude::*;
use std::time::Duration;

#[component]
pub fn ConnectionsView() -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let mut data = use_signal(Connections::default);
    let mut online = use_signal(|| false);

    use_future(move || async move {
        let mut last_cfg: Option<AppConfig> = None;
        let mut client: Option<ApiClient> = None;
        loop {
            let cfg = config();
            if last_cfg.as_ref() != Some(&cfg) {
                client = Some(ApiClient::new(
                    cfg.controller_url.clone(),
                    cfg.secret.clone(),
                ));
                last_cfg = Some(cfg);
            }
            match client.as_ref().unwrap().connections().await {
                Ok(c) => {
                    online.set(true);
                    data.set(c);
                }
                Err(_) => online.set(false),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    let snap = data();

    rsx! {
        div { class: "px-6 md:px-12 py-10",
            header { class: "flex flex-wrap items-end justify-between gap-4 border-b-2 border-black pb-6",
                div {
                    div { class: "text-[11px] uppercase tracking-[0.25em] text-neutral-500", "Active · Connections" }
                    h1 { class: "mt-3 text-5xl font-bold tracking-tighter leading-none", "连接" }
                }
                div { class: "text-right",
                    div { class: "text-6xl font-bold tracking-tighter tabular-nums leading-none", "{snap.connections.len()}" }
                    div { class: "mt-1 text-[11px] uppercase tracking-[0.18em] text-neutral-500", "Total" }
                }
            }

            if !online() {
                div { class: "mt-6 border-l-4 border-[#e3000f] pl-4 py-2 text-sm text-neutral-700", "未连接到控制器。" }
            }

            // 表格直接在正常流里,纵向滚动交给外层 main(避免内层 overflow 容器吞滚轮)
            table { class: "mt-8 w-full text-sm border-collapse table-fixed",
                thead {
                    tr { class: "border-b-2 border-black text-left text-[11px] uppercase tracking-[0.15em] text-neutral-500",
                        th { class: "py-2 pr-4 font-medium", "目标" }
                        th { class: "py-2 pr-4 font-medium w-20", "类型" }
                        th { class: "py-2 pr-4 font-medium", "代理链" }
                        th { class: "py-2 pr-4 font-medium", "规则" }
                        th { class: "py-2 pl-4 font-medium text-right w-24", "上行" }
                        th { class: "py-2 pl-4 font-medium text-right w-24", "下行" }
                    }
                }
                tbody {
                    for c in snap.connections.iter() {
                        tr { key: "{c.id}", class: "border-b border-neutral-200 hover:bg-neutral-50 align-top",
                            td { class: "py-2.5 pr-4 font-medium break-all", "{c.target()}" }
                            td { class: "py-2.5 pr-4 text-neutral-500 tabular-nums", "{c.metadata.network}/{c.metadata.conn_type}" }
                            td { class: "py-2.5 pr-4 text-neutral-500 break-all", "{c.chains.join(\" → \")}" }
                            td { class: "py-2.5 pr-4 text-neutral-500 break-all", "{c.rule}" }
                            td { class: "py-2.5 pl-4 text-right tabular-nums", "{format::bytes(c.upload)}" }
                            td { class: "py-2.5 pl-4 text-right tabular-nums", "{format::bytes(c.download)}" }
                        }
                    }
                }
            }
        }
    }
}
