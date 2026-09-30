//! 内核生命周期:start/stop/重载 + 平台 TUN 切换。
//!
//! macOS 上 TUN 不在主进程建(需 root),改为把 helper 建的 utun fd 通过
//! `proxyzms_adopt_tun(fd)` 注入内核,后续 set_tun(true) 才能成功;fd 注入
//! 失败会显式 return Err,让 UI 保持 OFF 而不是给假成功。
//!
//! start 流程:init → apply_config(从 config.yaml 读 yaml 文本)
//! stop 流程:shutdown
//! 重载流程:settings.rs 的"更新订阅并应用"直接调 `Kernel::apply_config`,
//! 不 restart(订阅文本变了,内核 reconcile listener 就完事)

use crate::bootstrap;
use crate::config::AppConfig;
use crate::mihomo::kernel;
use std::path::PathBuf;

#[cfg(target_os = "macos")]
use crate::helper;

#[derive(Clone)]
pub struct KernelLifecycle {
    inner: std::sync::Arc<std::sync::Mutex<Inner>>,
}

struct Inner {
    running: bool,
    /// 当前生效的 work_dir(AppConfig 里非空则覆盖,否则 data_dir 默认)。
    /// 记录在这里是为了 stop/restart 时能告知调用方"是不是同一个 home"。
    home_dir: PathBuf,
}

impl Default for KernelLifecycle {
    fn default() -> Self {
        Self {
            inner: std::sync::Arc::new(std::sync::Mutex::new(Inner {
                running: false,
                home_dir: bootstrap::data_dir(),
            })),
        }
    }
}

impl KernelLifecycle {
    /// 初始化 + 应用 config。重复调用 = no-op(等同 process.rs 旧 start 语义)。
    /// 首次启动失败会滚到 running=false,调用方可拿 Err 显示错误。
    pub async fn start(&self, cfg: &AppConfig) -> Result<(), String> {
        {
            let inner = self.inner.lock().map_err(|_| "lifecycle lock poisoned")?;
            if inner.running {
                return Ok(());
            }
        }

        let home = bootstrap::effective_data_dir(&cfg.work_dir);
        kernel().init(&home).await?;

        // 读 yaml(没有就 ensure_config 生成内置兜底)
        let yaml = bootstrap::read_config(&cfg.work_dir)?;
        let selected = cfg.selected_map.clone();
        match kernel().apply_config(&yaml, &selected).await {
            Ok(Some(warning)) if !warning.is_empty() => {
                // 不阻塞启动,但提示用户在 UI 上可见
                eprintln!("[zms] config fallback: {warning}");
            }
            Ok(_) => {}
            Err(e) => {
                let _ = kernel().shutdown().await;
                return Err(e);
            }
        }
        kernel()
            .set_lan_share(cfg.share_lan, cfg.share_port)
            .await?;

        {
            let mut inner = self.inner.lock().map_err(|_| "lifecycle lock poisoned")?;
            inner.running = true;
            inner.home_dir = home;
        }
        Ok(())
    }

    /// 停止内核。空转幂等。
    pub async fn stop(&self) -> Result<(), String> {
        {
            let inner = self.inner.lock().map_err(|_| "lifecycle lock poisoned")?;
            if !inner.running {
                return Ok(());
            }
        }
        kernel().shutdown().await?;
        let mut inner = self.inner.lock().map_err(|_| "lifecycle lock poisoned")?;
        inner.running = false;
        Ok(())
    }

    /// 设置 TUN 开关。**macOS 需要 helper fd;Windows 直接内核**。
    /// 开启失败会把错误往外抛(不静默),UI 应保持 OFF。
    pub async fn set_tun(&self, enable: bool, cfg: &AppConfig) -> Result<(), String> {
        eprintln!("[zms] [LC] set_tun({})", if enable { "ON" } else { "OFF" });
        if !enable {
            let res = kernel().set_tun(false).await;
            #[cfg(target_os = "macos")]
            {
                // helper 持有另一份 fd;必须成功释放,否则旧系统路由会继续指向 utun。
                let helper_res = tokio::task::spawn_blocking(helper::release_tun)
                    .await
                    .map_err(|e| format!("helper release_tun 任务异常:{e}"))
                    .and_then(|result| result);
                return match (res, helper_res) {
                    (Ok(()), Ok(())) => Ok(()),
                    (Err(kernel), Ok(())) => Err(kernel),
                    (Ok(()), Err(helper)) => Err(format!("TUN helper 清理失败:{helper}")),
                    (Err(kernel), Err(helper)) => {
                        Err(format!("{kernel}; TUN helper 清理失败:{helper}"))
                    }
                };
            }
            #[cfg(not(target_os = "macos"))]
            return res;
        }

        #[cfg(target_os = "macos")]
        {
            // fd 走 helper → 内核注入。失败返回 Err,UI 保持 OFF,不误判成功。
            eprintln!("[zms] [LC] ensure_tun_fd...");
            let fd = helper::ensure_tun_fd(cfg).await?;
            eprintln!("[zms] [LC] got fd={}, calling adopt_tun_fd", fd);
            kernel().adopt_tun_fd(fd).await?;
            eprintln!("[zms] [LC] adopt_tun_fd OK, calling set_tun(true)");
        }

        kernel().set_tun(true).await
    }

    /// 当前运行态(UI 显示 RUNNING/OFFLINE 的依据,非 Signal,
    /// 由调用方在 Telemetry.online 里同步)。
    pub fn is_running(&self) -> bool {
        self.inner.lock().map(|i| i.running).unwrap_or(false)
    }
}
