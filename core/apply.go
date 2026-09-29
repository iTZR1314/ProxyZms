//go:build cgo

package main

import (
	"fmt"

	"github.com/metacubex/mihomo/config"
	"github.com/metacubex/mihomo/hub/executor"
	"github.com/metacubex/mihomo/log"
	"github.com/metacubex/mihomo/tunnel"
)

// applyConfig 解析 YAML 并让 mihomo 接管全部 proxy/rule/DNS/sniffer/listener。
// selectedMap 把 UI 手选的节点回写到 SelectAble 组（对应 FlClash SetupParams.SelectedMap）。
// 返回 warning 字符串：正常为 ""；一旦 fallback 到 built-in default config 会有描述。
func applyConfig(yaml string, selectedMap map[string]string) (string, error) {
	if err := ensureInit(); err != nil {
		return "", err
	}

	configMu.Lock()
	defer configMu.Unlock()

	rawCfg, err := config.UnmarshalRawConfig([]byte(yaml))
	if err != nil {
		return "", fmt.Errorf("parse config: %w", err)
	}
	cfg, err := config.ParseRawConfig(rawCfg)
	if err != nil {
		// fallback：让 listeners 继续工作，错误信息上抛给 UI
		log.Warnln("config apply failed, falling back to built-in default: %v", err)
		fb, fbErr := config.ParseRawConfig(config.DefaultRawConfig())
		if fbErr != nil {
			return "", fmt.Errorf("fallback failed: %v (original: %w)", fbErr, err)
		}
		executor.ApplyConfig(fb, true)
		currentConfig = fb
		isRunning.Store(true)
		return fmt.Sprintf("config applied with fallback: %v", err), nil
	}

	executor.ApplyConfig(cfg, true)
	currentConfig = cfg
	if len(selectedMap) > 0 {
		patchSelectGroupLocked(selectedMap)
	}
	isRunning.Store(true)
	return "", nil
}

func patchSelectGroupLocked(mapping map[string]string) {
	for name, proxy := range tunnel.Proxies() {
		selected, ok := mapping[name]
		if !ok {
			continue
		}
		sel, ok := proxy.Adapter().(interface{ ForceSet(string) })
		if ok {
			sel.ForceSet(selected)
		}
	}
}

func validateConfig(yaml string) (string, error) {
	if _, err := config.UnmarshalRawConfig([]byte(yaml)); err != nil {
		return "", err
	}
	return "", nil
}
