# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

ProxyZms (`proxy-zms`) is a desktop GUI that downloads, launches, and controls a bundled
[mihomo](https://github.com/MetaCubeX/mihomo) proxy kernel. It is a **Dioxus 0.7** desktop app
(Rust + RSX + Tailwind, rendered via a webview). The UI is in Chinese; the app talks to mihomo
over its External Controller REST API. Most code comments are Chinese — match that when editing.

## Commands

```bash
dx serve                              # dev (default platform = desktop); keeps console/logs in debug
dx serve --platform desktop           # explicit
dx bundle --release --platform macos  # package .app (macOS)
cargo build                           # plain compile check
cargo clippy                          # lint — clippy.toml forbids holding signal/GenerationalRef borrows across await
```

`dx` is the Dioxus CLI (`cargo install dioxus-cli`). Tailwind is automatic in Dioxus 0.7 (it reads
`tailwind.css` next to `Cargo.toml`); no separate watcher needed. There are no tests in this repo.

### Windows builds (cross-built from macOS)

Windows is built on a remote Windows host, not locally:
- `./upload-to-windows.sh` — rsync-style upload of source (excludes `target/`, `.git/`) to the Windows box over SSH/expect.
- `./upload-to-windows.sh bundle` — upload **and** run `build-windows.ps1` remotely → produces an NSIS `.exe` installer under `dist/bundle/`.
- Host/credentials are hard-coded near the top of `upload-to-windows.sh`.
- `build.rs` + `proxyzms.rc`/`proxyzms.manifest` embed a `requireAdministrator` manifest on Windows so the exe always runs elevated (TUN/Wintun needs admin).

## Architecture

### Process-ownership invariant (the core design constraint)

**If the main app is not running, the mihomo kernel must not be running either.** `src/mihomo/process.rs`
enforces this across every exit path, layered:
- Normal close / panic unwind → `Inner::Drop` kills the child.
- Ctrl-C / SIGTERM → handler in `main()` calls `process::kill_tracked()` (Drop does *not* run on `process::exit`).
- Crash / SIGKILL → can't clean up live, so the **next** `Controller::start()` runs `cleanup_previous()`, which kills the PID recorded in `mihomo.pid` (and on Unix `pkill -f <work_dir>` as a backstop).

`Controller` is `Clone` (an `Arc<Inner>`), shared through Dioxus context, and `Send` so async event
handlers can start/stop the kernel. Start/stop are synchronous and never hold a lock across `await`.

### Local control seizure (`src/bootstrap.rs`)

The app treats itself as the sole authority over mihomo's External Controller. On every start it
**strips** any `external-controller*` / `secret*` top-level keys a downloaded subscription YAML might
carry (`strip_seized_keys` / `SEIZED_PREFIXES`) and **re-injects** the local controller URL + secret
(`enforce_local_control`, `reassert_control`). This prevents a subscription from hijacking the control
channel. `bootstrap.rs` also owns: the managed data dir (`<config_dir>/proxy-zms/mihomo`), downloading
the per-platform mihomo binary (`mac.gz` / `windows.zip` from `r2.zhoumaosen.top`, streamed with
progress), and fetching the subscription as `config.yaml`.

### State flow

- `AppConfig` (`src/config.rs`) — persisted to `<config_dir>/proxy-zms/config.json`. Empty `mihomo_path`/`work_dir` mean "use the managed binary/dir from bootstrap." Provided via context as `Signal<AppConfig>`.
- `TunState` (`src/main.rs`) — a single shared `Signal<bool>` for TUN on/off, read by **both** the UI and the system tray icon so they never diverge. The **only** writer is a 2s polling loop in `App` that reads mihomo's `/configs`; toggles never optimistically update — they set the signal only after the API call succeeds.
- `Controller` — also context-provided.

### `src/mihomo/`

- `api.rs` — `ApiClient`, a thin REST client for the External Controller (`/version`, `/configs`, `/connections`, `/proxies`, `set_mode`, `set_tun`, `select_proxy`, `group_delay`). Bearer-auth only when secret is non-empty.
- `process.rs` — `Controller` (above), plus privilege handling: `is_elevated` / `elevate_binary`. macOS uses an AppleScript `with administrator privileges` prompt to `chmod u+s` the binary (setuid-root so it can create the TUN device); Windows is already elevated via the embedded manifest, so its `elevate_binary` is a no-op.
- `types.rs` — serde models for the API responses.

### UI (`src/main.rs` + `src/views/`)

- `main.rs` — `App` root (sets up context + tray + polling), `Router` with `Shell` layout, and a large `#[cfg(feature = "desktop")]` block wiring the system tray: icon swaps with TUN state, right-click menu (启动/停止/退出), single-instance enforcement via a loopback TCP port (`127.0.0.1:53682`) used as lock + "show window" IPC, and macOS Dock-icon visibility toggling (hide to menu-bar agent on window close). Tray menu events are registered on **both** `use_tray_menu_event_handler` and `use_muda_event_handler` (only one global handler wins and it's unspecified which) sharing `handle_menu_select`.
- `views/dashboard.rs` (Home/状态) — bootstrap/setup state machine (`Checking → Downloading → Ready / Failed`), auto-start, live up/down speed + IPv6-reachability probe.
- `views/proxies.rs` — `ProxyGroups` (selectors + latency test) and `TunControls`.
- `views/connections.rs`, `views/settings.rs` — connection list and settings editor.

### Assets are compile-time embedded

For a single-file executable, `main.rs` inlines CSS (`include_str!` of `main.css` + `tailwind.css`)
into the webview's `<head>` via `with_custom_head`, and embeds icons/logo as `include_bytes!`
(the sidebar logo becomes a base64 data URI). Do **not** rely on runtime asset-path resolution for
these — add new global styles to `assets/main.css` or the Tailwind input so they get inlined.

## Dioxus 0.7 notes

`AGENTS.md` is the authoritative cheat-sheet. Key points: `cx`/`Scope`/`use_state` are gone; state is
`use_signal`/`use_memo`/`use_resource`; props must be owned, `PartialEq + Clone`; components are
`#[component]` fns starting with a capital letter. Clippy is configured to reject holding a signal
read/write borrow across an `await` (see `clippy.toml`) — clone the needed values out of `config.read()`
into locals before awaiting, as the existing code does.