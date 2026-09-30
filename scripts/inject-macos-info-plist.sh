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

set_plist_string() {
    local key="$1"
    local value="$2"
    if /usr/libexec/PlistBuddy -c "Print :$key" "$PLIST" >/dev/null 2>&1; then
        plutil -replace "$key" -string "$value" "$PLIST"
    else
        plutil -insert "$key" -string "$value" "$PLIST"
    fi
}

# 把 Finder、Dock 和应用切换器显示名改成产品名;可执行文件/Bundle ID 仍保持稳定。
set_plist_string CFBundleName "施展魔法"
set_plist_string CFBundleDisplayName "施展魔法"

# Always 是 macOS 11+ 用的;Peripheral 是 10.15 及更旧版本用的 —— 双写兼容。
set_plist_string NSBluetoothAlwaysUsageDescription \
    "施展魔法需要扫描附近的蓝牙设备,以便在你的手机离开本机时自动锁屏。"
set_plist_string NSBluetoothPeripheralUsageDescription \
    "施展魔法需要蓝牙访问以实现自动锁屏。"

echo "✓ Set app display name and injected NSBluetooth*UsageDescription into $PLIST"
