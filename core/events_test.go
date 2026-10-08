//go:build cgo

package main

import (
	"os"
	"os/exec"
	"runtime"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/metacubex/mihomo/log"
)

// unhookLogs 必须真正退订:孤儿订阅者的缓冲(200)写满后,mihomo observable
// 会在持锁状态下阻塞,进而让全进程的 log.Xxxln 永久卡死。
// 这里模拟「重启内核」(hook → unhook → 再 hook)之后继续产生大量日志。
func TestUnhookLogsDoesNotStallMihomoLog(t *testing.T) {
	log.SetLevel(log.SILENT) // 不往 stdout 刷;事件仍会进 logCh

	for i := 0; i < 3; i++ {
		hookLogsOnce()
		// 等转发 goroutine 真正进入 select 再退订;否则它可能在 unhook 之后才
		// 起跑,读到被置 nil 的 stop channel,反而一直在排空订阅而掩盖泄漏。
		time.Sleep(50 * time.Millisecond)
		unhookLogs()
		time.Sleep(50 * time.Millisecond)
	}
	hookLogsOnce()
	defer unhookLogs()

	done := make(chan struct{})
	go func() {
		defer close(done)
		for i := 0; i < 2000; i++ { // 远大于订阅缓冲 200
			log.Debugln("stall probe %d", i)
		}
	}()

	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("mihomo log 被阻塞:unhookLogs 留下了未退订的订阅者")
	}
}

func rssKB(t *testing.T) int {
	t.Helper()
	out, err := exec.Command("ps", "-o", "rss=", "-p", strconv.Itoa(os.Getpid())).Output()
	if err != nil {
		t.Fatalf("读取 RSS 失败: %v", err)
	}
	kb, err := strconv.Atoi(strings.TrimSpace(string(out)))
	if err != nil {
		t.Fatalf("解析 RSS %q 失败: %v", out, err)
	}
	return kb
}

// emitEvent 交给 C 回调的 JSON 由 Go 分配,回调返回后必须由 Go 释放
// (callback.h 约定 Rust 只拷贝不 free)。未注册回调时同样不能泄漏。
func TestEmitEventDoesNotLeakCString(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("依赖 ps 读取 RSS")
	}
	payload := logEventPayload{Level: "info", Payload: strings.Repeat("x", 512)}

	for i := 0; i < 1000; i++ { // 热身,让分配器稳定
		emitEvent("log", payload)
	}
	runtime.GC()
	before := rssKB(t)

	const n = 200000 // 每条约 560B,泄漏时合计 ≈ 110MB
	for i := 0; i < n; i++ {
		emitEvent("log", payload)
	}
	runtime.GC()
	after := rssKB(t)

	if delta := after - before; delta > 30*1024 {
		t.Fatalf("emitEvent %d 次后 RSS 增长 %d KB,C.CString 没有释放", n, delta)
	}
}
