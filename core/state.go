//go:build cgo

package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"
	"sync"
	"sync/atomic"

	"github.com/metacubex/mihomo/config"
	"github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/hub/executor"
	"github.com/metacubex/mihomo/listener"
	"github.com/metacubex/mihomo/tunnel"
)

// 全局内核状态机（借鉴 CMFA/FlClash hub.go 的 configMu + currentConfig），
// 去掉 Android 专属与 JSON-RPC 包装。
var (
	configMu      sync.Mutex
	currentConfig *config.Config
	isInit        atomic.Bool
	isRunning     atomic.Bool
	homeDir       string
	// adoptedFD:Go 侧 dup 出来、尚未交给 mihomo 的 TUN fd(configMu 保护)。
	// setTun 之后所有权归 mihomo(sing-tun 的 os.File),这里清零;
	// 没等到 setTun 就被替换 / shutdown 时由本包关闭。
	adoptedFD int
)

func initKernel(home string) error {
	if home == "" {
		return errors.New("home dir is empty")
	}
	if err := os.MkdirAll(home, 0o755); err != nil {
		return fmt.Errorf("create home dir: %w", err)
	}
	configMu.Lock()
	defer configMu.Unlock()
	constant.SetHomeDir(home)
	homeDir = home
	isInit.Store(true)
	return nil
}

func ensureInit() error {
	if !isInit.Load() {
		return errors.New("kernel not initialized")
	}
	return nil
}

func isKernelRunning() bool { return isInit.Load() && isRunning.Load() }

func stopKernel() error {
	configMu.Lock()
	defer configMu.Unlock()
	if !isInit.Load() {
		return nil
	}
	isRunning.Store(false)
	listener.Cleanup()
	currentConfig = nil
	if adoptedFD > 0 {
		closeFD(adoptedFD)
		adoptedFD = 0
	}
	isInit.Store(false)
	executor.Shutdown()
	return nil
}

func setMode(mode string) error {
	if err := ensureInit(); err != nil {
		return err
	}
	var m tunnel.TunnelMode
	switch strings.ToLower(strings.TrimSpace(mode)) {
	case "rule", "rules":
		m = tunnel.Rule
	case "global":
		m = tunnel.Global
	case "direct":
		m = tunnel.Direct
	default:
		return fmt.Errorf("unknown mode: %s", mode)
	}
	tunnel.SetMode(m)
	return nil
}

func getMode() (string, error) {
	if err := ensureInit(); err != nil {
		return "", err
	}
	return tunnel.Mode().String(), nil
}

func setTun(enable bool) error {
	if err := ensureInit(); err != nil {
		return err
	}
	configMu.Lock()
	defer configMu.Unlock()
	if currentConfig == nil || currentConfig.General == nil {
		return errors.New("config not applied")
	}
	currentConfig.General.Tun.Enable = enable
	updateListenersLocked(currentConfig)
	// 不论 mihomo 是否接管成功,待用 fd 都已交棒,Go 侧不再持有。
	adoptedFD = 0
	if !enable {
		// 监听已关(连带关了 fd);清掉过期编号,免得 fd 号被复用后误被当作 TUN fd 接管。
		currentConfig.General.Tun.FileDescriptor = 0
	}
	return nil
}

// setLanShare 调整 Mihomo 原生 allow-lan / bind-address / mixed-port。
// 只重建普通代理入站，不触碰 TUN listener，避免共享端口开关打断 utun。
func setLanShare(enable bool, port int) error {
	if err := ensureInit(); err != nil {
		return err
	}
	if port < 1 || port > 65535 {
		return fmt.Errorf("invalid mixed-port: %d", port)
	}
	configMu.Lock()
	defer configMu.Unlock()
	if currentConfig == nil || currentConfig.General == nil {
		return errors.New("config not applied")
	}

	g := currentConfig.General
	oldAllowLan, oldBindAddress, oldMixedPort := g.AllowLan, g.BindAddress, g.MixedPort
	g.AllowLan = enable
	g.BindAddress = "*"
	g.MixedPort = port
	updateProxyListenersLocked(currentConfig)
	if actual := listener.GetPorts().MixedPort; actual != port {
		// 绑定失败时恢复旧监听配置，避免共享端口切换造成现有本地代理也消失。
		g.AllowLan, g.BindAddress, g.MixedPort = oldAllowLan, oldBindAddress, oldMixedPort
		updateProxyListenersLocked(currentConfig)
		return fmt.Errorf("cannot bind mixed-port %d", port)
	}
	return nil
}

// getTunSetupJSON 返回 mihomo 解析后的有效 TUN 配置,供 macOS root helper
// 在接管 fd 之前设置 utun 地址和系统路由。FileDescriptor/Enable 由调用流程控制。
func getTunSetupJSON() (string, error) {
	if err := ensureInit(); err != nil {
		return "", err
	}
	configMu.Lock()
	defer configMu.Unlock()
	if currentConfig == nil || currentConfig.General == nil {
		return "", errors.New("config not applied")
	}
	raw, err := json.Marshal(currentConfig.General.Tun)
	if err != nil {
		return "", fmt.Errorf("marshal tun config: %w", err)
	}
	return string(raw), nil
}

// adoptTunFD:macOS helper 经 SCM_RIGHTS 传来的 utun fd。
// 所有权约定:调用方(Rust)始终自己关闭传入的 fd;这里 dup 一份写入
// currentConfig.General.Tun.FileDescriptor,让下一次 setTun(true) 走"接管既有 fd"
// 路径(而不是自己 connect utun_control,那需要 root)。dup 之后交给 mihomo 的
// sing-tun,TUN 关闭时由它的 os.File 释放。
// fd <= 0 表示清除:sing-tun 以 0 判定"未提供 fd",-1 会被当成真实 fd 而失败。
// 已知残留:mihomo 创建 TUN 失败且尚未接管 fd 时这份 dup 会留着——mihomo 内部
// 不暴露 fd 是否已被关闭,强行 close 有误关已被复用的 fd 号的风险,宁可漏一个。
func adoptTunFD(fd int) error {
	if err := ensureInit(); err != nil {
		return err
	}
	configMu.Lock()
	defer configMu.Unlock()
	if currentConfig == nil || currentConfig.General == nil {
		return errors.New("config not applied")
	}
	// 上一份 dup 若一直没被 setTun 取走,在此回收,避免反复点 TUN 开关时累积
	if adoptedFD > 0 {
		closeFD(adoptedFD)
		adoptedFD = 0
	}
	if fd <= 0 {
		currentConfig.General.Tun.FileDescriptor = 0
		return nil
	}
	dup, err := dupFD(fd)
	if err != nil {
		return fmt.Errorf("dup tun fd: %w", err)
	}
	adoptedFD = dup
	currentConfig.General.Tun.FileDescriptor = dup
	return nil
}

// updateListenersLocked 与 FlClash common.go updateListeners 等价（去掉 features.Android 判断）。
func updateListenersLocked(cfg *config.Config) {
	if cfg == nil {
		return
	}
	updateProxyListenersLocked(cfg)
	listener.ReCreateTun(cfg.General.Tun, tunnel.Tunnel)
}

// updateProxyListenersLocked 按 Mihomo allow-lan/bind-address 重建代理入站,
// 排除 TUN(由 macOS helper + SCM_RIGHTS 单独管理)。
func updateProxyListenersLocked(cfg *config.Config) {
	if cfg == nil || cfg.General == nil {
		return
	}
	g := cfg.General
	listener.SetAllowLan(g.AllowLan)
	listener.SetBindAddress(g.BindAddress)
	listener.ReCreateHTTP(g.Port, tunnel.Tunnel)
	listener.ReCreateSocks(g.SocksPort, tunnel.Tunnel)
	listener.ReCreateMixed(g.MixedPort, tunnel.Tunnel)
	listener.ReCreateRedir(g.RedirPort, tunnel.Tunnel)
	listener.ReCreateTProxy(g.TProxyPort, tunnel.Tunnel)
	listener.ReCreateTuic(g.TuicServer, tunnel.Tunnel)
	listener.PatchInboundListeners(cfg.Listeners, tunnel.Tunnel, true)
}
