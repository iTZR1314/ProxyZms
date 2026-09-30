# ProxyZms

> 一个用 Rust + Go 构建的桌面代理客户端 —— 将 [mihomo](https://github.com/MetaCubeX/mihomo) 内核静态链入应用,提供中文图形界面。

ProxyZms 将 Mihomo 编译为 Go `c-archive` 并链接进 Rust 桌面程序。UI 通过 FFI 在进程内控制内核，不需要下载或启动独立 mihomo 二进制，也没有 External Controller 端口、secret 或 WebSocket。应用仍会按用户设置下载订阅 YAML。界面用 [Dioxus 0.7](https://dioxuslabs.com/)(Rust + RSX + Tailwind)构建,通过系统 WebView 渲染。

## ✨ 功能

- **内核集成** —— mihomo 与代理依赖编译进应用；首次启动只需准备默认配置或拉取订阅。
- **局域网共享** —— 可在设置中启用 Mihomo 原生 `allow-lan` 并设置 `mixed-port`；默认关闭。
- **订阅管理** —— 填入订阅链接即可拉取节点配置为 `config.yaml`;留空则使用内置最小默认配置。
- **代理控制** —— 节点分组切换、延迟测速、规则/全局/直连模式切换。
- **TUN 模式** —— 一键开关透明代理(需要管理员权限创建 TUN 设备)。
- **实时状态** —— 仪表盘显示实时上/下行速度、连接列表、IPv6 可达性探测。
- **系统托盘** —— 托盘图标随 TUN 状态切换,右键菜单快速启动/停止/退出;关闭窗口收起到托盘后台常驻。
- **单实例运行** —— 通过回环端口加锁,重复启动会唤起已有窗口而非开新进程。

## 🔒 设计要点

**同进程内核** —— UI 与 Mihomo 位于同一应用进程中，通过 Rust FFI 调用 Go bridge；退出应用即结束内核，不再管理独立 mihomo 子进程。

**单文件内核** —— Go `c-archive` 链入桌面应用。Windows 构建使用 Mihomo 依赖的 sing-tun；其 `go:embed` 按目标架构把 Wintun DLL 编入内核并从内存加载，因此不需要旁置的 `wintun.dll`。Windows TUN 仍需管理员权限；macOS TUN 使用独立 root helper 与 SCM_RIGHTS 传递 utun 文件描述符。

## 📦 安装

从 [Releases](https://github.com/iTZR1314/ProxyZms/releases) 下载对应平台的安装包:

| 平台 | 产物 |
|------|------|
| macOS (Apple Silicon) | `ProxyZms-macos-arm64.dmg` |
| Windows x64 | `ProxyZms-windows-x64-setup.exe` |
| Windows ARM64 | `ProxyZms-windows-arm64-setup.exe` |

> Windows 应用内含 Mihomo 与对应架构的 Wintun DLL，不需要单独下载 `wintun.dll`；开启 TUN 时由内嵌 manifest 请求管理员权限。Dioxus 桌面应用仍使用系统 WebView2 运行时。
> macOS 首次开启 TUN 时会弹出授权对话框安装 root helper；主 App 全程不提权。

### macOS 首次打开

dmg 内的 app 经过 **ad-hoc 签名**(可在 Apple Silicon 上运行),但**未经 Apple 公证**,所以首次打开时 Gatekeeper 会拦一下。二选一即可:

- **右键打开**:把 App 拖进「应用程序」后,右键点图标 → 「打开」→ 在弹窗里再点「打开」。只需做一次。
- **命令行解除隔离**(若提示「已损坏,无法打开」):
  ```bash
  xattr -dr com.apple.quarantine /Applications/ProxyZms.app
  ```

> 想彻底去掉这个提示需要 Apple 开发者证书 + 公证(99 美元/年),本项目暂未启用。

## 🛠️ 从源码构建

需要 [Rust](https://rustup.rs/)、Go 1.24+ (CGO)、C 构建工具链与 [Dioxus CLI](https://dioxuslabs.com/learn/0.7/getting_started/)。macOS 安装 Xcode Command Line Tools；Windows 构建请使用仓库 GitHub Actions 的对应 runner/toolchain 配置。

```bash
cargo install dioxus-cli            # 安装 dx

dx serve                            # 开发模式(默认 desktop 平台,保留控制台日志)
cargo clippy                        # lint
dx bundle --release --platform macos --package-types dmg   # 打包 macOS .dmg
```

Tailwind 在 Dioxus 0.7 中自动启用(读取 `Cargo.toml` 同级的 `tailwind.css`),无需单独的 watcher。

## 🏗️ 项目结构

```
src/
├─ main.rs          # App 根组件:context / 系统托盘 / 状态轮询 / 路由
├─ bootstrap.rs     # 数据目录管理、默认配置与订阅拉取
├─ config.rs        # AppConfig,持久化到 <config_dir>/proxy-zms/config.json
├─ format.rs        # 速度 / 字节数格式化
├─ mihomo/
│  ├─ ffi.rs        # Go c-archive FFI 封装
│  ├─ lifecycle.rs  # 内核生命周期与平台 TUN 管理
│  └─ types.rs      # 内核数据模型
└─ views/
   ├─ flow.rs       # 状态页:引导状态机、自动启动、实时速度
   ├─ proxies.rs    # 节点分组 / 延迟测速 / TUN 开关
   ├─ connections.rs# 连接列表
   └─ settings.rs   # 设置编辑
```

## 🛠️ 技术栈

Rust · Go cgo · [Dioxus 0.7](https://dioxuslabs.com/)(desktop / WebView) · Tailwind CSS · [mihomo](https://github.com/MetaCubeX/mihomo) · GitHub Actions(macOS / Windows 构建发布)

## 📄 许可证

本项目以 [GNU General Public License v3.0](LICENSE) 授权发布。
