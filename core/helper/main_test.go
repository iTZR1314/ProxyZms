//go:build darwin && cgo

package main

import (
	"encoding/json"
	"testing"
)

func TestBuildTunOptionsFromEffectiveMihomoConfig(t *testing.T) {
	var req request
	err := json.Unmarshal([]byte(`{
		"op":"tun-on",
		"name":"utun9",
		"tun":{
			"auto-route":true,
			"inet4-address":["198.18.0.1/30"],
			"inet6-address":["fdfe:dcba:9876::1/126"]
		}
	}`), &req)
	if err != nil {
		t.Fatalf("decode effective mihomo config: %v", err)
	}
	if req.Tun == nil {
		t.Fatal("effective TUN config was not decoded")
	}

	opts, err := buildTunOptions(req.Tun, req.Name)
	if err != nil {
		t.Fatalf("build tun options: %v", err)
	}
	if opts.MTU != 9000 {
		t.Fatalf("default MTU = %d, want 9000", opts.MTU)
	}
	if len(opts.Inet4Address) != 1 || len(opts.Inet6Address) != 1 {
		t.Fatalf("addresses were not passed through: v4=%v v6=%v", opts.Inet4Address, opts.Inet6Address)
	}
	routes, err := opts.BuildAutoRouteRanges(false)
	if err != nil {
		t.Fatalf("build auto-route ranges: %v", err)
	}
	if len(routes) == 0 {
		t.Fatal("auto-route produced no system routes")
	}
}
