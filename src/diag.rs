//! 设置页「诊断」面板的数据源:Go 运行时快照 + 进程级资源(fd / 僵尸子进程 / RSS)。
//!
//! 目的是让"有没有泄漏"变成可对比的数字:先「记为基线」,做一轮操作(比如开关 TUN
//! 10 次),再「刷新」看每项相对基线的增量。fd / 僵尸子进程只在 macOS 采集。

use crate::mihomo::ffi_types::GoDiag;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub go: GoDiag,
    pub os: Option<OsDiag>,
}

/// 进程级资源。内存单位 KB,其余为个数。
#[derive(Debug, Clone, Default)]
pub struct OsDiag {
    pub rss_kb: i64,
    pub fd_total: i64,
    pub fd_pipe: i64,
    pub fd_kqueue: i64,
    pub fd_socket: i64,
    pub fd_file: i64,
    /// 本进程已退出但没人 wait 的子进程(`ps` 里 STAT 为 Z)
    pub zombies: i64,
}

/// 一行展示数据。`kb` 为 true 时 `cur`/`delta` 的单位是 KB,渲染成 MB。
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub label: &'static str,
    pub sub: bool,
    pub kb: bool,
    pub cur: i64,
    pub delta: Option<i64>,
}

// 基线放进程级静态:设置页是路由页,切走再切回会重新挂载,信号状态会丢。
static BASELINE: Mutex<Option<Snapshot>> = Mutex::new(None);

pub fn baseline() -> Option<Snapshot> {
    BASELINE.lock().ok().and_then(|g| g.clone())
}

pub fn set_baseline(s: Snapshot) {
    if let Ok(mut g) = BASELINE.lock() {
        *g = Some(s);
    }
}

pub async fn snapshot() -> Result<Snapshot, String> {
    let go = crate::mihomo::kernel().get_diag().await?;
    let os = tokio::task::spawn_blocking(os_snapshot)
        .await
        .map_err(|e| format!("诊断任务异常:{e}"))?;
    Ok(Snapshot { go, os })
}

/// 把当前快照展开成表格行;有基线时附带增量。
pub fn rows(cur: &Snapshot, base: Option<&Snapshot>) -> Vec<Row> {
    let mut out = Vec::new();
    let mut push = |label, sub, kb, c: i64, b: Option<i64>| {
        out.push(Row {
            label,
            sub,
            kb,
            cur: c,
            delta: b.map(|b| c - b),
        });
    };
    let kb = |bytes: u64| (bytes / 1024) as i64;

    if let Some(o) = &cur.os {
        let b = base.and_then(|b| b.os.as_ref());
        push("进程内存 RSS", false, true, o.rss_kb, b.map(|b| b.rss_kb));
        push(
            "打开的 fd 总数",
            false,
            false,
            o.fd_total,
            b.map(|b| b.fd_total),
        );
        push("pipe", true, false, o.fd_pipe, b.map(|b| b.fd_pipe));
        push("kqueue", true, false, o.fd_kqueue, b.map(|b| b.fd_kqueue));
        push("socket", true, false, o.fd_socket, b.map(|b| b.fd_socket));
        push("文件", true, false, o.fd_file, b.map(|b| b.fd_file));
        push("僵尸子进程", false, false, o.zombies, b.map(|b| b.zombies));
    }
    let g = &cur.go;
    let bg = base.map(|b| &b.go);
    push(
        "Go 堆(使用中)",
        false,
        true,
        kb(g.heap_inuse),
        bg.map(|b| kb(b.heap_inuse)),
    );
    push(
        "Go goroutine 数",
        false,
        false,
        g.goroutines as i64,
        bg.map(|b| b.goroutines as i64),
    );
    out
}

#[cfg(target_os = "macos")]
fn os_snapshot() -> Option<OsDiag> {
    use std::os::raw::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ProcFdInfo {
        fd: i32,
        kind: u32,
    }
    const PROC_PIDLISTFDS: i32 = 1;
    const FDTYPE_VNODE: u32 = 1;
    const FDTYPE_SOCKET: u32 = 2;
    const FDTYPE_KQUEUE: u32 = 5;
    const FDTYPE_PIPE: u32 = 6;
    extern "C" {
        fn proc_pidinfo(
            pid: i32,
            flavor: i32,
            arg: u64,
            buffer: *mut c_void,
            buffersize: i32,
        ) -> i32;
    }

    let pid = std::process::id() as i32;
    let entry = std::mem::size_of::<ProcFdInfo>();

    // 先问需要多大;枚举期间 fd 可能增加,留 32 个余量
    let need = unsafe { proc_pidinfo(pid, PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if need <= 0 {
        return None;
    }
    let cap = need as usize / entry + 32;
    let mut buf = vec![ProcFdInfo { fd: 0, kind: 0 }; cap];
    let got = unsafe {
        proc_pidinfo(
            pid,
            PROC_PIDLISTFDS,
            0,
            buf.as_mut_ptr().cast(),
            (cap * entry) as i32,
        )
    };
    if got <= 0 {
        return None;
    }
    buf.truncate(got as usize / entry);

    let mut d = OsDiag {
        fd_total: buf.len() as i64,
        ..Default::default()
    };
    for e in &buf {
        match e.kind {
            FDTYPE_PIPE => d.fd_pipe += 1,
            FDTYPE_KQUEUE => d.fd_kqueue += 1,
            FDTYPE_SOCKET => d.fd_socket += 1,
            FDTYPE_VNODE => d.fd_file += 1,
            _ => {}
        }
    }

    // RSS 与僵尸子进程都从一次 `ps` 取(格式:pid ppid stat rss,无表头)
    let out = std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,stat=,rss="])
        .output()
        .ok()?;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut it = line.split_whitespace();
        let (Some(p), Some(pp), Some(stat), Some(rss)) =
            (it.next(), it.next(), it.next(), it.next())
        else {
            continue;
        };
        let (Ok(p), Ok(pp)) = (p.parse::<i32>(), pp.parse::<i32>()) else {
            continue;
        };
        if p == pid {
            d.rss_kb = rss.parse().unwrap_or(0);
        } else if pp == pid && stat.starts_with('Z') {
            d.zombies += 1;
        }
    }
    Some(d)
}

#[cfg(not(target_os = "macos"))]
fn os_snapshot() -> Option<OsDiag> {
    None
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    extern "C" {
        fn pipe(fds: *mut i32) -> i32;
        fn close(fd: i32) -> i32;
    }

    // 面板上的数字要可信:开 5 对 pipe,fd_pipe 恰好 +10;全部关掉后回到原值。
    #[test]
    fn counts_pipe_fds() {
        // 进程里第一次起 `ps` 会一次性建一个常驻 kqueue(实测 0 → 1 后不再增长),
        // 先预热,免得把它算进 before/during 的差里。
        let _ = os_snapshot();
        let before = os_snapshot().expect("proc_pidinfo 应当可用");
        let mut fds = Vec::new();
        for _ in 0..5 {
            let mut p = [0i32; 2];
            assert_eq!(unsafe { pipe(p.as_mut_ptr()) }, 0);
            fds.extend(p);
        }
        let during = os_snapshot().unwrap();
        for fd in fds {
            unsafe { close(fd) };
        }
        let after = os_snapshot().unwrap();

        let ctx = format!("before={before:?}\nduring={during:?}\nafter={after:?}");
        assert_eq!(during.fd_pipe - before.fd_pipe, 10, "{ctx}");
        assert_eq!(during.fd_total - before.fd_total, 10, "{ctx}");
        assert_eq!(after.fd_pipe, before.fd_pipe, "{ctx}");
        assert!(before.rss_kb > 0, "RSS 应当能从 ps 读到");
    }
}
