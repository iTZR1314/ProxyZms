#!/usr/bin/env bash
#
# 把 BLE 权限描述键注入到已打包的 .app/Contents/Info.plist。
#
# 背景:dx 0.7 的 [bundle.macos] `info_plist_path` 是**整体替换**、不支持合并,
# 用它会把 CFBundleIdentifier / CFBundleExecutable 等关键键全冲掉。所以采用
# post-build 注入:让 dx 正常产出完整 Info.plist(版本号从 Cargo.toml 同步),
# 再用 plutil 把这两个 key 插进去。
#
# 必须有 NSBluetoothAlwaysUsageDescription:
#   macOS 11+ 任何应用第一次访问 CoreBluetooth 时,系统检查这个 key,
#   缺失则**直接 SIGABRT 进程**(不弹弹窗、不可恢复)。
#
# 用法:
#   ./scripts/inject-macos-info-plist.sh path/to/Foo.app
#
# 必须在 codesign **之前**调用,否则签名校验会发现 Info.plist 被改而失败。

set -euo pipefail

APP_PATH="${1:?usage: $0 <path-to-app>}"
PLIST="$APP_PATH/Contents/Info.plist"

if [ ! -f "$PLIST" ]; then
    echo "::error::Info.plist not found at $PLIST" >&2
    exit 1
fi

# -replace 是 idempotent 的:键存在则覆盖,不存在则等价于 -insert。
# Always 是 macOS 11+ 用的;Peripheral 是 10.15 及更旧版本用的 —— 双写兼容。
plutil -replace NSBluetoothAlwaysUsageDescription \
    -string "VPN JR 需要扫描附近的蓝牙设备,以便在你的手机离开本机时自动锁屏。" \
    "$PLIST"
plutil -replace NSBluetoothPeripheralUsageDescription \
    -string "VPN JR 需要蓝牙访问以实现自动锁屏。" \
    "$PLIST"

echo "✓ Injected NSBluetooth*UsageDescription into $PLIST"
