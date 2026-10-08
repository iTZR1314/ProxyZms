//! 退出清理:Ctrl-C / SIGTERM、托盘「退出」、事件循环销毁(macOS Cmd+Q 等)共用同一套流程。
//!
//! 做两件事,每一步都有时限,不会因为某一步卡住而收不了退出:
//! 1. 停 Go 内核(shutdown)
//! 2. macOS:让 root helper 释放它持有的 utun。helper 只在收到 `tun-off` 或自己被
//!    SIGTERM 时才关接口,感知不到主进程退出;不主动释放的话 utun9 和它的系统路由
//!    会留在原地,流量会被路由进一个没人读取的接口。
//!
//! 先停内核再放 helper,与 `KernelLifecycle::set_tun(false)` 的顺序一致:
//! 内核先关掉自己那份 fd,helper 再关它的,接口才会真正销毁。

use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

/// 每一步的最长等待。正常情况下两步都是毫秒级;
/// 超时只会出现在 Go 侧 configMu 被长操作(apply_config 等)占着的极端情形。
const STEP_DEADLINE: Duration = Duration::from_secs(2);

static DONE: Mutex<bool> = Mutex::new(false);

/// 清理内核与(macOS)helper 持有的 utun。幂等:重复 / 并发调用只有第一次真正执行,
/// 后到的调用会等它做完再返回(否则进程会在清理到一半时退出)。
pub fn cleanup() {
    once(&DONE, || {
        run_bounded("内核 shutdown", STEP_DEADLINE, || {
            if let Err(e) = crate::mihomo::kernel().shutdown_blocking() {
                eprintln!("[zms] 退出:内核 shutdown 失败:{e}");
            }
        });
        // 没装过 helper(socket 不存在)就没有可释放的东西,别连一次再报错
        #[cfg(target_os = "macos")]
        if std::path::Path::new(crate::helper::SOCKET_PATH).exists() {
            run_bounded("helper 释放 TUN", STEP_DEADLINE, || {
                if let Err(e) = crate::helper::release_tun() {
                    eprintln!("[zms] 退出:helper 释放 TUN 失败:{e}");
                }
            });
        }
    });
}

/// 在独立线程里跑 `f`,最多等 `deadline`。超时返回 false,线程被丢下,
/// 由随后的进程退出回收;`f` panic 时发送端随线程展开被 drop,同样立刻返回 false。
fn run_bounded<F>(name: &str, deadline: Duration, f: F) -> bool
where
    F: FnOnce() + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<()>();
    let spawned = std::thread::Builder::new()
        .name(format!("quit-{name}"))
        .spawn(move || {
            f();
            let _ = tx.send(());
        });
    if spawned.is_err() {
        eprintln!("[zms] 退出:无法为「{name}」创建线程,跳过");
        return false;
    }
    let finished = rx.recv_timeout(deadline).is_ok();
    if !finished {
        eprintln!("[zms] 退出:「{name}」未在 {deadline:?} 内完成,放弃等待");
    }
    finished
}

/// 持锁执行 `f`:只有第一个调用者会执行,其余调用者阻塞到它结束后直接返回。
fn once(done: &Mutex<bool>, f: impl FnOnce()) {
    let mut d = done.lock().unwrap_or_else(|e| e.into_inner());
    if *d {
        return;
    }
    f();
    *d = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    #[test]
    fn bounded_returns_true_when_step_finishes() {
        assert!(run_bounded("ok", Duration::from_secs(2), || {}));
    }

    // 退出时内核可能根本没起来(首次引导失败、DIOXUS_NO_KERNEL):
    // 同步 shutdown 必须经真实的 Go FFI 返回 Ok,而不是报错或卡住。
    #[test]
    fn shutdown_blocking_is_ok_when_kernel_never_started() {
        let finished = run_bounded("shutdown", Duration::from_secs(5), || {
            crate::mihomo::kernel()
                .shutdown_blocking()
                .expect("未初始化的内核 shutdown 应当是空操作");
        });
        assert!(finished, "shutdown_blocking 没有在时限内返回(或内部 panic)");
    }

    #[test]
    fn bounded_gives_up_on_hung_step() {
        let t = Instant::now();
        let finished = run_bounded("hang", Duration::from_millis(200), || {
            std::thread::sleep(Duration::from_secs(5));
        });
        assert!(!finished);
        assert!(t.elapsed() < Duration::from_secs(2), "不应等到步骤自己结束");
    }

    #[test]
    fn bounded_returns_promptly_when_step_panics() {
        let t = Instant::now();
        let finished = run_bounded("panic", Duration::from_secs(5), || panic!("boom"));
        assert!(!finished);
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "panic 应立刻返回,不必等满时限"
        );
    }

    #[test]
    fn once_runs_once_and_late_caller_waits_for_first() {
        let done = Arc::new(Mutex::new(false));
        let runs = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = mpsc::channel::<()>();

        let first = {
            let (done, runs) = (done.clone(), runs.clone());
            std::thread::spawn(move || {
                once(&done, || {
                    entered_tx.send(()).unwrap();
                    std::thread::sleep(Duration::from_millis(200));
                    runs.fetch_add(1, Ordering::SeqCst);
                })
            })
        };
        // 等 first 确实进入临界区,再发起第二个调用
        entered_rx.recv().unwrap();
        let t = Instant::now();
        once(&done, || {
            runs.fetch_add(1, Ordering::SeqCst);
        });

        assert!(
            t.elapsed() >= Duration::from_millis(100),
            "后到的调用必须等 first 做完才返回"
        );
        assert_eq!(runs.load(Ordering::SeqCst), 1, "清理只应执行一次");
        first.join().unwrap();
    }
}
