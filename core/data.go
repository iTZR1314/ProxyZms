//go:build cgo

package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"github.com/metacubex/mihomo/adapter"
	"github.com/metacubex/mihomo/common/utils"
	"github.com/metacubex/mihomo/constant"
	C "github.com/metacubex/mihomo/constant"
	P "github.com/metacubex/mihomo/constant/provider"
	"github.com/metacubex/mihomo/tunnel"
	"github.com/metacubex/mihomo/tunnel/statistic"
)

// isGroupType：SelectAble/URLTest/Fallback/Relay/LoadBalance 这些可切换的组。
func isGroupType(t C.AdapterType) bool {
	switch t {
	case C.Selector, C.URLTest, C.Fallback, C.Relay, C.LoadBalance:
		return true
	default:
		return false
	}
}

// proxiesData 与 REST GET /proxies 的 JSON 结构一致：
//
//	{"proxies": {"name":{...}}, "all":["group1","group2",...]}
//
// Rust types.rs 的 Proxies/BTreeMap<String,Proxy> 可以直接反序列化此格式。
type proxiesData struct {
	Proxies map[string]C.Proxy `json:"proxies"`
	All     []string           `json:"all"`
}

func getProxies() (string, error) {
	if err := ensureInit(); err != nil {
		return "", err
	}
	all := tunnel.Proxies()
	order := make([]string, 0, len(all))
	for name, p := range all {
		if p == nil {
			continue
		}
		if !isGroupType(p.Type()) {
			continue
		}
		order = append(order, name)
	}
	hasGlobal := false
	for _, n := range order {
		if n == "GLOBAL" {
			hasGlobal = true
			break
		}
	}
	if !hasGlobal {
		if g, ok := all["GLOBAL"]; ok && g != nil && isGroupType(g.Type()) {
			order = append([]string{"GLOBAL"}, order...)
		}
	}
	buf, err := json.Marshal(proxiesData{Proxies: all, All: order})
	if err != nil {
		return "", err
	}
	return string(buf), nil
}

func selectProxy(groupName, proxyName string) error {
	if err := ensureInit(); err != nil {
		return err
	}
	all := tunnel.Proxies()
	p, ok := all[groupName]
	if !ok || p == nil {
		return fmt.Errorf("group not found: %s", groupName)
	}
	sel, ok := p.Adapter().(interface {
		Set(string) error
		ForceSet(string)
	})
	if !ok {
		return fmt.Errorf("group %s is not selectable", groupName)
	}
	if proxyName == "" {
		sel.ForceSet("")
		return nil
	}
	return sel.Set(proxyName)
}

// delayResult 与 REST 返回一致，Rust 侧可直接反序列化。
type delayResult struct {
	Name    string `json:"name"`
	URL     string `json:"url"`
	Timeout int64  `json:"timeout_ms"`
	Delay   int32  `json:"delay"`
	Error   string `json:"error,omitempty"`
}

func testDelay(proxyName, testURL string, timeoutMs int64) (string, error) {
	if err := ensureInit(); err != nil {
		return "", err
	}
	if timeoutMs <= 0 {
		timeoutMs = 5000
	}
	if testURL == "" {
		testURL = constant.DefaultTestURL
	}
	p, ok := tunnel.Proxies()[proxyName]
	if !ok || p == nil {
		return "", fmt.Errorf("proxy not found: %s", proxyName)
	}
	adapt, ok := p.(*adapter.Proxy)
	if !ok {
		return "", fmt.Errorf("%s is not a proxy", proxyName)
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Duration(timeoutMs)*time.Millisecond)
	defer cancel()
	delay, err := adapt.URLTest(ctx, testURL, utils.IntRanges[uint16]{})
	res := delayResult{Name: proxyName, URL: testURL, Timeout: timeoutMs, Delay: -1}
	if err != nil {
		res.Error = err.Error()
	} else if delay != 0 {
		res.Delay = int32(delay)
	}
	buf, mErr := json.Marshal(res)
	if mErr != nil {
		return "", mErr
	}
	return string(buf), nil
}

// getConnections 返回 statistic.Snapshot 的 JSON，与 REST GET /connections 格式一致。
func getConnections() (string, error) {
	if err := ensureInit(); err != nil {
		return "", err
	}
	m := statistic.DefaultManager
	if m == nil {
		return "", errors.New("statistic manager not initialized")
	}
	snap := m.Snapshot()
	buf, err := json.Marshal(snap)
	if err != nil {
		return "", err
	}
	return string(buf), nil
}

func closeConnection(id string) error {
	if err := ensureInit(); err != nil {
		return err
	}
	m := statistic.DefaultManager
	if m == nil {
		return errors.New("statistic manager not initialized")
	}
	c := m.Get(id)
	if c == nil {
		return fmt.Errorf("connection not found: %s", id)
	}
	return c.Close()
}

func getTraffic(total bool) (string, error) {
	if err := ensureInit(); err != nil {
		return "", err
	}
	m := statistic.DefaultManager
	if m == nil {
		return "", errors.New("statistic manager not initialized")
	}
	var up, down int64
	if total {
		up, down = m.Total()
	} else {
		up, down = m.Now()
	}
	buf, err := json.Marshal(map[string]int64{"up": up, "down": down})
	if err != nil {
		return "", err
	}
	return string(buf), nil
}

// updateSubscription 找到 HTTP VehicleType 的 external provider 并调用 Update（会上网拉订阅并落盘缓存）。
func updateSubscription(name string) error {
	if err := ensureInit(); err != nil {
		return err
	}
	if name == "" {
		return errors.New("provider name is empty")
	}
	for _, p := range tunnel.Providers() {
		if p != nil && p.Name() == name && p.VehicleType() == P.HTTP {
			return p.Update()
		}
	}
	for _, p := range tunnel.RuleProviders() {
		if p != nil && p.Name() == name && p.VehicleType() == P.HTTP {
			return p.Update()
		}
	}
	return fmt.Errorf("provider not found: %s", name)
}
