# Repository Guidelines

## Project Overview

ProxyZms (`proxy-zms`, v0.0.4, GPL-3.0) is a **Dioxus 0.7 desktop GUI** (Rust + RSX + Tailwind, rendered in a system WebView, shipped as a single-file executable) that downloads, launches, and controls a bundled [mihomo](https://github.com/MetaCubeX/mihomo) proxy kernel over its External Controller REST API. The UI and most code comments are **Chinese** — match that style when editing. Default platform is `desktop`; Windows binaries are cross-built from macOS.

## Architecture & Data Flow

The app treats itself as the **sole owner** of one mihomo process and the **sole authority** over its control channel. Two invariants dominate the design:

- **Process-ownership** (`src/mihomo/process.rs`): *if the app isn't running, the kernel isn't either.* Enforced in 4 layers — normal close/panic → `Inner::Drop`; Ctrl-C/SIGTERM → handler calls `process::kill_tracked()` + `exit` (Drop won't run on `process::exit`); crash/SIGKILL → next `Controller::start()` runs `cleanup_previous()`, killing the PID in `mihomo.pid` (+ `pkill -f <work_dir>` on Unix).
- **Local-control seizure** (`src/bootstrap.rs`): every start **strips** any `external-controller*`/`secret*` top-level keys from the subscription YAML (`strip_seized_keys` / `SEIZED_PREFIXES`) and **re-injects** the local controller URL + secret (`enforce_local_control` / `reassert_control`), so a subscription can't hijack the channel.

State is shared through **Dioxus context** (no signal prop-drilling), provided once in `App` (`src/main.rs`):

- `Signal<AppConfig>` — persisted config (`use_context_provider(|| Signal::new(AppConfig::load()))`).
- `Controller` — `Clone` (`Arc<Inner>`) + `Send` kernel handle (`use_context_provider(Controller::default)`).
- `TunState(Signal<bool>)` — shared TUN on/off, read by **both** UI and tray. Its **only writer** is the 2s `/configs` polling loop in `App`; toggles never optimistically update — they set the signal only after the API call succeeds.

Data flow: `App` provides context + tray + polling → `Router::<Route>` renders `Shell` (sidebar + `Outlet`) → views read context, call `ApiClient`/`Controller`, and run their own `use_future` polling loops (throughput, IPv6 probe). Routes (`#[layout(Shell)]`): `/`→`FlowPage`, `/connections`→`Connections`, `/settings`→`SettingsPage`; these are thin wrappers rendering `Flow` / `ConnectionsView` / `Settings`.

## Key Directories

- `src/` — all Rust source.
  - `src/main.rs` — entry point, `App` root, `Route` enum, `Shell` layout, system tray, single-instance lock, 2s polling loop, macOS dock/icon shims, compile-time CSS/icon embedding.
  - `src/bootstrap.rs` — managed data dir (`<config_dir>/proxy-zms/mihomo`), per-platform binary download/extract, subscription fetch, control seizure.
  - `src/config.rs` — `AppConfig` (JSON-persisted).
  - `src/format.rs` — byte/speed humanizers.
  - `src/mihomo/` — `api.rs` (`ApiClient` REST client), `process.rs` (`Controller`, elevation), `types.rs` (Deserialize models).
  - `src/views/` — `flow.rs` (Home/状态 — **the real dashboard**), `proxies.rs` (`ProxyGroups`, `TunControls`), `connections.rs`, `settings.rs`; private modules with flat `pub use` re-exports in `mod.rs`.
- `assets/` — `main.css` (hand-written global CSS), `tailwind.css` (compiled output), icons/logos (most embedded at compile time).
- `.github/workflows/` — `ci.yml`, `release.yml`.

> ⚠️ `CLAUDE.md`/`README.md` mention `views/dashboard.rs`; it no longer exists — that behavior was merged into `views/flow.rs`. Treat such references as stale.

## Development Commands

```bash
cargo install dioxus-cli                                   # provides `dx` (pin to 0.7.1)
dx serve                                                   # dev (default platform=desktop; keeps console logs)
dx serve --platform desktop                               # explicit
cargo build                                                # compile check (cargo build --release in CI)
cargo clippy --all-targets -- -D warnings                 # lint — exactly what CI gates on; must be clean
dx bundle --release --platform macos --package-types dmg  # package macOS .dmg
```

**Windows is never built locally** — it's cross-built on a remote Windows host:

```bash
./upload-to-windows.sh           # rsync-ish tar+scp upload of source (excludes target/, .git/)
./upload-to-windows.sh bundle    # upload AND run build-windows.ps1 remotely → NSIS .exe in dist/bundle/
```

`build-windows.ps1` runs `dx bundle --release --platform windows --package-types nsis` (it first rebuilds `$env:Path` from the registry because non-interactive SSH sessions get a trimmed PATH).

> Host/credentials are **hard-coded** near the top of `upload-to-windows.sh` (a committed plaintext SSH password). Do not break, duplicate, or leak these values.

## Code Conventions & Common Patterns

- **Error handling**: plain `Result<_, String>` (Chinese messages), `reqwest::Result`, `std::io::Result`. **No** `anyhow`, `thiserror`, or `ServerFnError`. `AppConfig::load()` never panics (falls back to `Default`).
- **Async**: `tokio` is enabled with only `["time", "rt"]` (no macros/full); use Dioxus `spawn` / `use_future` for tasks, `futures_util::StreamExt` for streamed downloads. Start/stop are synchronous and never hold a lock across `await`.
- **Signals-across-await (enforced by `clippy.toml`)**: never hold a `GenerationalRef`/`GenerationalRefMut`/`WriteLock` across `.await`. Clone owned values out of `config.read()` into locals **before** awaiting:
  ```rust
  let (url, secret) = { let c = config.read(); (c.controller_url.clone(), c.secret.clone()) };
  let cfg = ApiClient::new(&url, &secret).configs().await; // borrow already dropped
  ```
- **State management**: `use_signal` (local), `use_resource` (async fetch), `use_future` (dominant: `loop { …; tokio::time::sleep(…).await }` polling), `use_effect` (react to signals), `use_context::<T>()` (shared state). **Not used anywhere**: `use_memo`, `ReadOnlySignal`, `Signal<T>` as a prop.
- **Components (Dioxus 0.7)**: `#[component]` fns, capitalized names. Top-level views take **no props**; helper components take **owned** values (`String`/`bool`/`Element`) plus `EventHandler<T>` callbacks (e.g. `Field(label, value, placeholder, oninput: EventHandler<String>)`), never signals. Remember 0.7: `cx`/`Scope`/`use_state` are gone.
- **RSX/styling**: Tailwind utility strings inline in `rsx!`; brand red is `#e3000f` (`--accent` in `main.css`). Prefer `for`/`if` directly in `rsx!`.
- **serde**: API models are `Deserialize`-only; `AppConfig` is `Serialize + Deserialize + PartialEq` with `#[serde(default)]` on newer fields.
- **Assets are compile-time embedded** — do **not** rely on runtime asset paths. `main.rs` inlines `main.css` + `tailwind.css` into the WebView `<head>` via `with_custom_head` (`include_str!`), embeds tray/window icons with `include_bytes!`, and base64-encodes the sidebar logo into a `data:` URI. Add global styles to `assets/main.css` (or the Tailwind input) so they get inlined.

## Important Files

- `src/main.rs` — entry, `App`, `Route`/`Shell`, tray, single-instance (loopback `127.0.0.1:53682`, release-only), polling loop, asset embedding.
- `src/bootstrap.rs` — bootstrap state + control seizure.
- `src/mihomo/process.rs` — `Controller`, kill paths, `is_elevated`/`elevate_binary` (macOS setuid via AppleScript prompt; Windows no-op, elevated by manifest).
- `src/mihomo/api.rs` — `ApiClient` (`/version`, `/configs`, `/connections`, `/proxies`, `set_mode`, `set_tun`, `select_proxy`, `group_delay`).
- `src/views/flow.rs` — home/status page; holds the `NORMAL_MODE` const (see Testing).
- `Cargo.toml`, `Dioxus.toml` (bundle id `top.zhoumaosen`, icons), `clippy.toml` (await-holding rules), `build.rs` + `proxyzms.rc`/`proxyzms.manifest` (Windows `requireAdministrator`).
- `.github/workflows/ci.yml` (clippy + build on macos-14 & windows-latest), `release.yml` (tag `v*` → 3-target dmg/NSIS build + GitHub Release).

## Runtime/Tooling Preferences

- **Toolchain**: Rust (edition 2021) + the **Dioxus CLI `dx`** (pin `0.7.1`); `cargo` is the only package manager. CI installs `dx` via `cargo binstall dioxus-cli@0.7.1` (no source compile).
- **Default feature** is `desktop` (`dioxus/desktop`); `web`/`mobile` features exist but the app is desktop-only (`Dioxus.toml` `[web.*]` is empty scaffold, no `index.html`).
- **Tailwind v4 is automatic**: the 23-byte root `tailwind.css` (`@import "tailwindcss";`) is the input; `dx` compiles it to `assets/tailwind.css` (committed). No watcher, `package.json`, or `tailwind.config.js`.
- **Platform specifics**: macOS-only `objc` dep (dock/icon, setuid prompt); Windows-only `embed-resource` build-dep (UAC manifest). `Cargo.lock` is committed.

## Testing & QA

- **There are no tests** anywhere in the repo (verified: no `#[test]`, `#[cfg(test)]`, `mod tests`, `tests/`, doctests; CI runs no `cargo test`).
- **Verify changes** with: `cargo clippy --all-targets -- -D warnings` (warnings are hard errors in CI — keep it clean), `cargo build`, then a manual runtime check via `dx serve`.
- For **UI-only iteration** without spawning the real kernel, set `const NORMAL_MODE: bool = false` in `src/views/flow.rs` (revert before committing).
- When adding behavior worth testing, prefer real runnable tests over mocks; never suppress clippy/build failures to make code pass.
