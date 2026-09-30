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

// adoptTunFD:macOS helper 注入的外部 utun fd。写入 currentConfig.General.Tun.FileDescriptor,
// 使得下一次 setTun(true) 走"接管既有 fd"路径而不是自己 connect(utun_control).
// fd <0 表示清除(回到自创建路径——需要 root)。idempotent。
func adoptTunFD(fd int) error {
	if err := ensureInit(); err != nil {
		return err
	}
	configMu.Lock()
	defer configMu.Unlock()
	if currentConfig == nil || currentConfig.General == nil {
		return errors.New("config not applied")
	}
	currentConfig.General.Tun.FileDescriptor = fd
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
