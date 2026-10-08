//go:build darwin && cgo

package main

import (
	"os"
	"testing"

	"github.com/metacubex/mihomo/config"
	"golang.org/x/sys/unix"
)

func openFDCount(t *testing.T) int {
	t.Helper()
	entries, err := os.ReadDir("/dev/fd")
	if err != nil {
		t.Fatalf("读取 /dev/fd 失败: %v", err)
	}
	return len(entries)
}

// adoptTunFD 的所有权约定:Go dup 自己的一份,调用方的原 fd 不动;
// 未被 setTun 取走的旧 dup 在再次 adopt / 清除时回收;清除值是 0 而不是 -1。
func TestAdoptTunFDOwnership(t *testing.T) {
	configMu.Lock()
	prevCfg, prevInit := currentConfig, isInit.Load()
	currentConfig = &config.Config{General: &config.General{}}
	adoptedFD = 0
	configMu.Unlock()
	isInit.Store(true)
	t.Cleanup(func() {
		configMu.Lock()
		if adoptedFD > 0 {
			closeFD(adoptedFD)
			adoptedFD = 0
		}
		currentConfig = prevCfg
		configMu.Unlock()
		isInit.Store(prevInit)
	})

	var p [2]int
	if err := unix.Pipe(p[:]); err != nil {
		t.Fatal(err)
	}
	defer unix.Close(p[0])
	defer unix.Close(p[1])

	base := openFDCount(t)

	if err := adoptTunFD(p[0]); err != nil {
		t.Fatalf("adopt: %v", err)
	}
	got := currentConfig.General.Tun.FileDescriptor
	if got <= 0 || got == p[0] {
		t.Fatalf("应写入 dup 出的新 fd,得到 %d(原 fd %d)", got, p[0])
	}
	if n := openFDCount(t); n != base+1 {
		t.Fatalf("adopt 后 fd 数 %d,期望 %d", n, base+1)
	}
	if _, err := unix.FcntlInt(uintptr(p[0]), unix.F_GETFD, 0); err != nil {
		t.Fatalf("调用方的原 fd 被 Go 关掉了: %v", err)
	}

	// 再次 adopt:上一份未被 setTun 取走的 dup 必须回收,fd 总数不能增长
	if err := adoptTunFD(p[1]); err != nil {
		t.Fatalf("re-adopt: %v", err)
	}
	if n := openFDCount(t); n != base+1 {
		t.Fatalf("re-adopt 后 fd 数 %d,期望 %d(旧 dup 泄漏)", n, base+1)
	}

	// 清除:回收待用 fd,编号归 0(sing-tun 以 0 判定"未提供",-1 会被当真 fd)
	if err := adoptTunFD(-1); err != nil {
		t.Fatalf("clear: %v", err)
	}
	if fd := currentConfig.General.Tun.FileDescriptor; fd != 0 {
		t.Fatalf("清除后 FileDescriptor = %d,期望 0", fd)
	}
	if n := openFDCount(t); n != base {
		t.Fatalf("清除后 fd 数 %d,期望 %d", n, base)
	}
}
