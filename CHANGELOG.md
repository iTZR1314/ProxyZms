# Changelog

## v0.3.3 — 2026-10-08

支持开机自启静默运行（不显示主窗口、macOS 收起 Dock 图标）并自动开启 TUN。

### 特性与修复

- **开机自启静默拉起**: 自启动项附带 `--autostart` 参数。登录拉起时窗口不显示，macOS 收起 Dock 图标（仅在顶部菜单栏驻留托盘）；点击托盘或从启动台再次打开时无缝唤醒主窗口并恢复 Dock 图标。
- **自启动自动开启 TUN**: 内核初始化与配置应用完成后，自启场景自动拉起 TUN 模式并同步更新托盘状态与配置。
- **托盘切换持久化**: 修复了通过托盘菜单切换 TUN 时未写回磁盘配置文件的问题。
- **设置页说明优化**: 明确自启动行为说明。

## v0.3.2 — 2026-10-08

修复 macOS 上「开机启动」勾选后实际不生效的问题。

### 修复

- **macOS:开机启动勾选了却不生效**:launchd 的 disabled 覆盖表优先于 LaunchAgent plist,
  label 一旦被标成 disabled(如在系统设置 → 登录项里关过开关),plist 写得再对也不会在登录时
  加载,而设置页只看 plist 是否存在,于是勾选框显示「开」、实际没开。现在开启时会补一次
  `launchctl enable` 清掉 disabled 标记,勾选框状态也同时检查该覆盖表
- Windows 走注册表 Run 项,不受影响

## v0.3.1 — 2026-10-08

FFI 内核的内存 / 线程问题修复(以 macOS 为主)、退出清理统一、设置页新增诊断面板。

### 修复

- **日志订阅从不退订**:重启内核后,残留订阅者的 200 条缓冲写满,会让 mihomo 在持锁状态下
  阻塞,进而卡死全进程的日志调用。现在退订时真正 `UnSubscribe`,并等转发 goroutine 退出
- **每条日志泄漏一个 C 字符串**:Go 侧分配的事件 JSON 在回调返回后补上 `C.free`
- **事件回调指针读写不同步**:回调与其上下文指针现在在同一把读写锁下读取
- **macOS:每次开 TUN 泄漏一个 utun fd**:helper 经 SCM_RIGHTS 传来的 fd 改用 `OwnedFd`
  持有,任何失败路径都会自动关闭
- **macOS:外部 TUN fd 的所有权与"清除"语义**:Go 侧 dup 自己的一份,未被取走的旧副本在再次
  注入 / 关闭内核时回收;"清除"改用 `0`(sing-tun 以 0 表示未提供,`-1` 会被当成真实 fd);
  关闭 TUN 后清掉过期编号,避免 fd 号被复用后被误接管
- 9 处 `CString::into_raw()` 改为借用指针(`into_raw` 会把所有权交给 Go,而 Go 不会释放)

### 退出清理

- Ctrl-C / SIGTERM、托盘「退出」、macOS Cmd+Q 现在共用同一套清理流程:先停内核,再让
  macOS helper 释放它持有的 utun(helper 感知不到主进程退出,此前会留下 utun 与系统路由)
- 每一步最多等 2 秒;托盘退出原先注释写"1s 内强退",实际是无限期等待,现已修正

### 新增

- 设置页「06 / 诊断」:Go 堆与 goroutine 数、进程内存、按类型统计的文件描述符、僵尸子进程。
  可「记为基线」后对比增量,用来判断开关 TUN 等操作是否在泄漏资源。fd 与僵尸进程统计仅 macOS

### 已知问题

- macOS 上每次开 / 关 TUN,sing-tun 会 `Start()` 一次 `dscacheutil -flushcache` 而不 `Wait()`,
  产生僵尸子进程(只占进程表项,不占 fd 和内存)。问题在上游依赖,本版未处理
- 开关 TUN 十次后 pipe 与 kqueue 各多出若干且未回落,来源尚未查明,诊断面板可用于继续观察

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
- 为兼容 Windows x64 MSVC 链接器，Go 对象的 SEH unwind section 会被移除；不影响 Go `panic/recover`，
  但原生调试器无法沿 Go 栈展开
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
