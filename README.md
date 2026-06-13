# ProxyZms

> 一个用 Rust 编写的桌面代理客户端 —— 下载、启动并接管 [mihomo](https://github.com/MetaCubeX/mihomo) 内核,提供中文图形界面。

ProxyZms 把 mihomo 代理内核包装成一个开箱即用的桌面应用:它替你下载对应平台的内核二进制、拉取订阅、启动并通过 mihomo 的 External Controller REST API 实时控制代理。界面用 [Dioxus 0.7](https://dioxuslabs.com/)(Rust + RSX + Tailwind)构建,通过系统 WebView 渲染,最终打包成单文件可执行程序。

## ✨ 功能

- **内核托管** —— 首次启动自动下载对应平台的 mihomo 二进制与默认配置,无需手动安装。
- **订阅管理** —— 填入订阅链接即可拉取节点配置为 `config.yaml`;留空则使用内置最小默认配置。
- **代理控制** —— 节点分组切换、延迟测速、规则/全局/直连模式切换。
- **TUN 模式** —— 一键开关透明代理(需要管理员权限创建 TUN 设备)。
- **实时状态** —— 仪表盘显示实时上/下行速度、连接列表、IPv6 可达性探测。
- **系统托盘** —— 托盘图标随 TUN 状态切换,右键菜单快速启动/停止/退出;关闭窗口收起到托盘后台常驻。
- **单实例运行** —— 通过回环端口加锁,重复启动会唤起已有窗口而非开新进程。

## 🔒 设计要点

**进程归属不变量** —— *主程序不在运行,mihomo 内核就一定不在运行。* 应用在每一条退出路径上都保证清理子进程:正常关闭/panic 由 `Drop` 兜底,Ctrl-C/SIGTERM 由信号处理器清理,崩溃/SIGKILL 的残留则由下次启动时根据 `mihomo.pid` 回收。

**控制权接管** —— 应用把自己视为 mihomo External Controller 的唯一管理者。每次启动都会**剥除**订阅 YAML 里可能携带的 `external-controller` / `secret` 等顶层字段,并**重新注入**本地的控制器地址与密钥,防止订阅劫持控制通道。

## 📦 安装

从 [Releases](https://github.com/iTZR1314/ProxyZms/releases) 下载对应平台的安装包:

| 平台 | 产物 |
|------|------|
| macOS (Apple Silicon) | `ProxyZms-macos-arm64.dmg` |
| Windows x64 | `ProxyZms-windows-x64-setup.exe` |
| Windows ARM64 | `ProxyZms-windows-arm64-setup.exe` |

> Windows 安装包内嵌 `requireAdministrator` 清单,会以管理员身份运行(TUN/Wintun 需要)。
> macOS 首次开启 TUN 时会弹出授权对话框,为内核二进制赋予创建 TUN 设备所需的权限。

## 🛠️ 从源码构建

需要 [Rust](https://rustup.rs/) 与 [Dioxus CLI](https://dioxuslabs.com/learn/0.7/getting_started/):

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
├─ bootstrap.rs     # 数据目录管理、内核下载、订阅拉取、控制权接管
├─ config.rs        # AppConfig,持久化到 <config_dir>/proxy-zms/config.json
├─ format.rs        # 速度 / 字节数格式化
├─ mihomo/
│  ├─ api.rs        # External Controller REST 客户端
│  ├─ process.rs    # Controller:内核进程生命周期 + 提权
│  └─ types.rs      # API 响应的 serde 模型
└─ views/
   ├─ dashboard.rs  # 状态页:引导状态机、自动启动、实时速度
   ├─ proxies.rs    # 节点分组 / 延迟测速 / TUN 开关
   ├─ connections.rs# 连接列表
   └─ settings.rs   # 设置编辑
```

## 🛠️ 技术栈

Rust · [Dioxus 0.7](https://dioxuslabs.com/)(desktop / WebView)· Tailwind CSS · [mihomo](https://github.com/MetaCubeX/mihomo) 内核 · GitHub Actions(三平台自动构建发布)

## 📄 许可证

本项目以 [GNU General Public License v3.0](LICENSE) 授权发布。
