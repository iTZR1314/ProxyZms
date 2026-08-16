# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

ProxyZms is a desktop GUI that downloads, launches, and controls a bundled
[mihomo](https://github.com/MetaCubeX/mihomo) proxy kernel. It is a **Dioxus 0.7** desktop app
(Rust + RSX + Tailwind, rendered via a system webview, shipped as a single-file executable).
The UI is in Chinese; the app talks to mihomo over its External Controller REST/WebSocket API.
Most code comments are Chinese — match that when editing.

**Naming is deliberately inconsistent — don't "fix" it.** Cargo `[package].name` is `VPN-JR`
(this is what `dx bundle` derives the macOS `.app` name and NSIS display name from), the window
title is `VPN JR`, the sidebar brand is 东莞锦荣纺织, the repo/release artifacts are `ProxyZms`,
and the on-disk data dir is `proxy-zms`. Changing the Cargo package name renames the shipped app.

`AGENTS.md` is a longer cheat-sheet with Dioxus 0.7 conventions; its BLE-lock sections
(`src/ble_lock/`, `BleSession`, `/ble-lock` route) describe code that was removed — ignore them.

## Commands

```bash
dx serve                                        # dev (default platform = desktop); keeps console logs
cargo build                                     # plain compile check
cargo clippy --all-targets -- -D warnings       # exactly what CI gates on; must be clean
dx bundle --release --platform macos --package-types dmg   # package macOS .dmg locally
```

`dx` is the Dioxus CLI, pinned to **0.7.10** to match the `dioxus` crate (`cargo install dioxus-cli`;
CI uses `cargo binstall dioxus-cli@0.7.10`). Tailwind v4 is automatic in Dioxus 0.7 — the 23-byte root
`tailwind.css` (`@import "tailwindcss";`) is the input, `dx` compiles it to `assets/tailwind.css`
(committed). No watcher, `package.json`, or `tailwind.config.js`.

**Almost no tests** — the only `#[test]`s are the comment-parser units in `src/node_notes.rs`
(`cargo test`, also run by CI). There is no `tests/` dir and nothing else is covered: verify with
clippy + build, then hand a runtime check to the user.

**Don't run `dx serve` yourself** — it opens a real window *and* spawns the real mihomo kernel,
which will fight the user's running instance over the control port. Ask the user to run it.
For UI-only iteration without the kernel, flip `const NORMAL_MODE: bool = false` in
`src/views/flow.rs` (revert before committing).

### Releases

Tagging `v*` triggers `.github/workflows/release.yml`, which builds on three **native** runners
(macos-14, windows-latest, windows-11-arm) and uploads to a GitHub Release. macOS is not built by
`dx` alone — the workflow post-processes the `.app`:

1. `./scripts/inject-macos-info-plist.sh <app>` — adds `NSBluetooth*UsageDescription`.
   **Must run before `codesign`**; signing first then editing Info.plist breaks the signature.
2. ad-hoc `codesign --force --deep --sign -` — without *some* signature the app is
   "已损坏" on Apple Silicon. This is not notarization; users still need right-click-open.
3. `hdiutil create` to build the dmg with an `/Applications` symlink.

`upload-to-windows.sh` / `build-windows.ps1` (remote Windows build box over SSH+expect) are
**gitignored** — they hold a plaintext SSH password and are a local-only legacy path. CI is the
real Windows build. If you see them locally, don't commit them or leak the credentials.

## Architecture

### Process-ownership invariant (the core design constraint)

**If the main app is not running, the mihomo kernel must not be running either.** `src/mihomo/process.rs`
enforces this across every exit path, layered:
- Normal close / panic unwind → `Inner::Drop` kills the child.
- Ctrl-C / SIGTERM → handler in `main()` calls `process::kill_tracked()` (Drop does *not* run on `process::exit`).
- Tray 退出 → `controller.stop()` + `kill_tracked()` before `exit`.
- Crash / SIGKILL → can't clean up live, so the **next** `Controller::start()` runs `cleanup_previous()`,
  which kills the PID recorded in `mihomo.pid` (and on Unix `pkill -f <work_dir>` as a backstop;
  on Windows `taskkill /F /T` so TUN helper children don't orphan).

`Controller` is `Clone` (an `Arc<Inner>`), shared through Dioxus context, and `Send` so async event
handlers can start/stop the kernel. Start/stop are synchronous and never hold a lock across `await`.

### Local control seizure (`src/bootstrap.rs`)

The app treats itself as the sole authority over mihomo's External Controller. On every start it
**strips** any `external-controller*` / `secret*` top-level keys a downloaded subscription YAML might
carry (`strip_seized_keys` / `SEIZED_PREFIXES`, prefix-matched so `-tls`/`-unix`/`-cors` variants and
their indented children go too) and **re-injects** the local controller URL + secret
(`enforce_local_control`, `reassert_control`). This prevents a subscription from hijacking the
control channel.

`bootstrap.rs` also owns the managed data dir (`<config_dir>/proxy-zms/mihomo`) and asset download.
Binaries come from an **R2 mirror** (`r2.zhoumaosen.top`), not GitHub, to dodge the GFW — which means
there is no version detection: whatever is in the bucket under `mac.gz` / `windows.zip` /
`windowsarm.zip` is what you get. Windows additionally needs `wintun.dll` (pinned 0.14.1, also
mirrored); `is_installed()` is false until both are present.

### State flow — one central poller, everything else reads context

`App` (`src/main.rs`) provides four things via context and owns **all** kernel polling. Views must
never start their own `/configs` / `/connections` / `/proxies` loops:

- `Signal<AppConfig>` (`src/config.rs`) — persisted to `<config_dir>/proxy-zms/config.json`.
  Empty `mihomo_path`/`work_dir` mean "use the managed binary/dir from bootstrap".
- `Controller` — the kernel process handle.
- `TunState(Signal<bool>)` — shared TUN on/off, read by **both** the UI and the tray icon so they
  never diverge. The only writer is the `/configs` loop; toggles never optimistically update — they
  set the signal only after the API call succeeds (this is why the button shows a spinner instead of
  flipping immediately).
- `Telemetry` — `online`, `configs`, `connections`, `proxies`, `down_speed`, `up_speed`, `history`,
  and `poke`.

Two loops feed `Telemetry`:

1. **`/connections` over WebSocket** (`ApiClient::subscribe_connections`) — mihomo pushes a snapshot
   at 1 Hz. On stream error the loop reconnects after 1s; `online` flips false but the last
   `connections` frame is kept so the UI doesn't snap to zero. URL/secret changes only take effect on
   the next reconnect.
2. **`/configs` + `/proxies` every 2s** — sleeps in 100ms slices and breaks early when `poke` changes,
   so an action (switch node / test latency / change mode) refreshes within ~100ms. Any handler that
   mutates kernel state should bump `poke` after the call succeeds.

Instantaneous up/down speed and the 48-slot rolling throughput history are **derived in `App`** from
successive `connections` frames, not in the view — so the chart stays continuous across page switches.

### `src/mihomo/`

- `api.rs` — `ApiClient`: REST (`/configs`, `/proxies`, `set_mode`, `set_tun`, `select_proxy`,
  `group_delay`) plus the `/connections` WebSocket subscription. Bearer-auth only when secret is
  non-empty. Note the `std::future::ready` wrapper in `subscribe_connections` — without it the
  returned stream isn't `Unpin` and callers can't `.next().await`.
- `process.rs` — `Controller` (above), plus privilege handling: `is_elevated` / `elevate_binary`.
  macOS uses an AppleScript `with administrator privileges` prompt to `chown root` + `chmod u+s`
  (setuid-root so mihomo can create the TUN device); Windows is already elevated via the embedded
  manifest, so its `elevate_binary` is a no-op and `is_elevated` probes `net session`.
- `types.rs` — `Deserialize`-only models. `connections` uses `null_to_default` because mihomo sends
  JSON `null` (not `[]`) when idle; `Proxies` uses a `BTreeMap` for stable render order.

### Autostart (`src/autostart.rs`)

**The OS is the single source of truth** — a macOS LaunchAgent plist / Windows `HKCU\...\Run` value —
deliberately *not* mirrored into `AppConfig`, so config can't claim "on" after the file was deleted
by hand. Windows shells out to `reg.exe` (with `CREATE_NO_WINDOW`) rather than pulling in `winreg`.

### UI (`src/main.rs` + `src/views/`)

Routes under `#[layout(Shell)]`: `/` → `Flow` (流量, also the first-run bootstrap screen),
`/nodes` → `Nodes` (节点), `/connections` → `ConnectionsView` (连接), `/settings` → `Settings` (设置).

- `views/flow.rs` — home page. Owns the bootstrap state machine (`Checking → Downloading →
  Ready / Failed`), auto-starts the kernel once on `Ready`, and probes IPv6 every 3s. The IPv6 check
  uses UDP `connect` to trigger route selection **without sending a packet**, so a blocked host
  doesn't produce a false negative.
- `views/proxies.rs` — `Nodes` (mode switch + selector-group tabs + latency test; hides the built-in
  `GLOBAL` group) and `TunControls` (TUN toggle + 授权 button). Each node row's 供应商/地区/协议
  note is **parsed out of the comments in `config.yaml`** (`src/node_notes.rs`) — mihomo's API has
  no such field, so the subscription's own YAML comments are the source of truth. mtime-cached, so
  the 2s re-render only costs a `stat`.
- `views/connections.rs`, `views/settings.rs` — connection table and settings editor.
- `main.rs` also carries a large `#[cfg(feature = "desktop")]` block: tray icon that swaps with TUN
  state, a right-click menu (启动/停止 · 节点切换 submenu rebuilt from `Telemetry.proxies` · 退出),
  single-instance enforcement via a loopback TCP port (`127.0.0.1:17653`) used as lock + "show window"
  IPC (release builds only), and macOS Dock-icon visibility toggling (hide to menu-bar agent on
  window close).

**Showing the main window has exactly one entry point — `show_main_window()`.** Three things can ask
for it: macOS `Event::Reopen` (clicking the Dock/Finder/Launchpad icon of an already-running app),
a tray-icon click, and the single-instance "show" IPC. Two macOS ordering constraints are baked in
and easy to re-break: tao's `set_focus()` returns early unless the window is already visible, so it
must come *after* `set_visible(true)`; and flipping the activation policy Accessory→Regular does not
make the app frontmost, so `activate_app()` (NSApp `activate`, falling back to
`activateIgnoringOtherApps:` pre-macOS 14) is required or the window surfaces behind other apps.

macOS never launches a second process for an already-running `.app` — LaunchServices sends reopen
instead. So the TCP single-instance path is effectively Windows-only in practice, and `Event::Reopen`
is the *only* way a Dock/Finder icon click reaches the app. The single-instance port must stay
**outside the ephemeral range** (49152–65535 on both macOS and Windows): a port inside it can be
claimed by an unrelated process, which the lock would read as "an instance is already running" and
exit silently — the app then never opens at all. The handshake (`HELLO`/`ACK`) exists so a squatter
on the port is treated as "not us" and startup continues instead of aborting.

Two tray gotchas: menu events are registered on **both** `use_tray_menu_event_handler` and
`use_muda_event_handler` (only one global handler wins and it's unspecified which) sharing
`handle_menu_select`; and the proxy submenu is only rebuilt when `tray_proxy_snapshot` actually
changes, otherwise every 2s poll would tear down and rebuild the menu the user is clicking.

### Assets are compile-time embedded

For a single-file executable, `main.rs` inlines CSS (`include_str!` of `assets/main.css` +
`assets/tailwind.css`) into the webview's `<head>` via `with_custom_head`, and embeds icons/logo as
`include_bytes!` (the sidebar logo becomes a base64 data URI). Do **not** rely on runtime asset-path
resolution — add new global styles to `assets/main.css` or the Tailwind input so they get inlined.
Hand-written classes that Tailwind can't express (`.flow-chart`, `.flow-bar`, `.no-scrollbar`) live
in `main.css`; brand red `#e3000f` is `--accent` there.

### Dark mode = one variable block, no `dark:` variants

The UI follows the system theme via a single `@media (prefers-color-scheme: dark)` block in
`main.css` that **redefines Tailwind's own color variables** (`--color-white`/`--color-black` swap,
`--color-neutral-*` mirrored). Tailwind v4 compiles `text-neutral-500` to
`color: var(--color-neutral-500)`, and unlayered declarations beat `@layer theme`, so overriding
the variables re-themes every view at once — there is not a single `dark:` class in `src/`.

This only holds because the design is strictly monochrome + one accent. **Never write a literal
color in a view** (`bg-[#e3000f]`, `text-red-500`, inline `style` colors): it won't switch. Use the
`neutral` scale, `black`/`white`, or `var(--accent)`. `.flow-bar`/scrollbar colors go through
`--bar`/`--bar-live`/`--scroll-*` tokens for the same reason.

Two Rust-side pieces support it: the `<head>` gets `<meta name="color-scheme" content="light dark">`
(native controls/scrollbars), and `src/theme.rs::system_is_dark()` picks the tao window's
`with_background_color` so a dark-mode launch doesn't flash white (macOS: `AppleInterfaceStyle` via
`NSUserDefaults`; Windows: `AppsUseLightTheme` via `reg.exe`). Live theme switching is the webview's
job — nothing in Rust tracks it.

## Dioxus 0.7 notes

`cx`/`Scope`/`use_state` are gone; state is `use_signal`/`use_future`/`use_effect`/`use_context`.
Props must be owned, `PartialEq + Clone` — top-level views take no props, helper components take
owned `String`/`bool`/`Element` plus `EventHandler<T>`, never signals. Components are `#[component]`
fns starting with a capital letter.

Errors are plain `Result<_, String>` with Chinese messages (no `anyhow`/`thiserror`).

Clippy is configured to reject holding a signal read/write borrow across an `await` (`clippy.toml`) —
clone the needed values out into locals **before** awaiting, as the existing code does:

```rust
let (url, secret) = { let c = config.read(); (c.controller_url.clone(), c.secret.clone()) };
ApiClient::new(url, secret).set_tun(target).await   // borrow already dropped
```
