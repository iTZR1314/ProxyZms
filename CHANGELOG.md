# Changelog

## v0.3.0 — 2026-09-30

**内核 FFI 化（mihomo 进进程）**：告别「独立子进程 + External Controller REST/WebSocket」，
mihomo 以 Go c-archive 静态库形式链入 Rust 主程序，UI 与内核同进程。

### 架构变更

- **不再有** mihomo 二进制运行期下载（原 R2 镜像下载流程全删）
- **不再有** external-controller 端口 / secret / WebSocket
- **不再有** 进程树管理（pidfile / kill_tracked / cleanup_previous / Drop 杀子进程）
- 所有控制面走 Go bridge → Rust FFI 函数调用，错误以统一 JSON 信封 `{"ok","data","error"}` 返回
- 设置页新增 Mihomo 原生局域网共享：可配置 `allow-lan` / `mixed-port`，默认关闭

### macOS：TUN 真正可用（root helper + SCM_RIGHTS）

- 新增 `core/helper` Go root daemon（launchd `top.zhoumaosen.ProxyZms.Helper`）
- 点 TUN ON 时：
  1. 若 helper 未装，AppleScript 弹一次密码框把它装进 `/Library/PrivilegedHelperTools/`
     + `/Library/LaunchDaemons/`，launchd 常驻
  2. 主 App 通过 unix socket 请求 helper 建 utun
  3. helper 用 SCM_RIGHTS 把 fd 传回主 App
  4. 主 App `proxyzms_adopt_tun(fd)` 注入内核，mihomo ReCreateTun 直接接管既有 fd
     （sing-tun@v0.4.24 原生支持 `Options.FileDescriptor != 0`）
- 主 App 全程不提权，不需要 setuid
- 设置页新增「macOS Helper」区块：安装/卸载按钮 + 说明

### Windows：单文件内核

- exe 内嵌 `requireAdministrator` manifest，双击一次 UAC，整进程即管理员
- Go 内核静态链入 exe；sing-tun v0.4.24 已按 amd64/arm64 用 `go:embed` 嵌入
  对应的 Wintun DLL，并通过内存加载，不需要用户另行下载或摆放 `wintun.dll`
- Windows TUN 不需要 macOS root helper / SCM_RIGHTS；仍需 UAC 管理员权限

### 开发者体验

- 新增 `DIOXUS_NO_KERNEL=1` 环境变量跳过内核启动，UI 调试不必起 mihomo
  （取代旧 `NORMAL_MODE` 常量）
- 配置持久化新增：`mode` / `tun_enable` / `tun_stack` / `log_level` / `selected_map`
  （重启后恢复上次代理模式、日志级别、手选节点）
- `work_dir` 覆盖真正生效（bootstrap 提供 `effective_data_dir` / `effective_config_path`）
- 设置页新增「重启内核」「更新订阅并应用」「安装/卸载 macOS Helper」按钮
- 删除设置项：mihomo 路径、Secret、删除并重新下载核心（概念已不存在）

### CI

- ci.yml / release.yml 加 `actions/setup-go@v6`（Go 1.25.x，build.rs 需要）
- release.yml windows-arm64 leg 装 msys2 ucrt64 `mingw-w64-ucrt-aarch64-gcc`
  （Go cgo 交叉编译必须）

### 兼容

- 老配置（v0.2.x 的 `mihomo_path` / `controller_url` / `secret` 字段）serde 自动忽略，
  不会报错；新字段带 `serde(default)` 平滑升级
- Windows 应用 exe 内含 Go c-archive / mihomo 与对应架构的 Wintun DLL；NSIS `.exe` 为安装包

## v0.2.14 — 之前的 release

见 GitHub Releases 历史。
