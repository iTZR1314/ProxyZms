//go:build cgo

package main

import (
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
	g := cfg.General
	listener.SetAllowLan(g.AllowLan)
	listener.SetBindAddress(g.BindAddress)
	listener.ReCreateHTTP(g.Port, tunnel.Tunnel)
	listener.ReCreateSocks(g.SocksPort, tunnel.Tunnel)
	listener.ReCreateMixed(g.MixedPort, tunnel.Tunnel)
	listener.ReCreateRedir(g.RedirPort, tunnel.Tunnel)
	listener.ReCreateTProxy(g.TProxyPort, tunnel.Tunnel)
	listener.ReCreateTun(g.Tun, tunnel.Tunnel)
	listener.ReCreateTuic(g.TuicServer, tunnel.Tunnel)
	listener.PatchInboundListeners(cfg.Listeners, tunnel.Tunnel, true)
}
