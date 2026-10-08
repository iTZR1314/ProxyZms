//go:build cgo

package main

/*
#include <stdlib.h>
#include "callback.h"
*/
import "C"

import (
	"encoding/json"
	"fmt"
	"sync"
	"unsafe"

	"github.com/metacubex/mihomo/common/observable"
	"github.com/metacubex/mihomo/log"
)

var (
	eventMu sync.Mutex
	logHk   *logHook
)

// logHook 是一次 mihomo 日志订阅的全部资源。
// unhook 必须真正 UnSubscribe:mihomo 的 observable 在持锁状态下向每个订阅者的
// 缓冲(200)阻塞写入,孤儿订阅者写满后会让全进程的 log.Xxxln 永久卡死。
type logHook struct {
	sub  observable.Subscription[log.Event]
	done chan struct{} // 转发 goroutine 退出后关闭
}

type logEventPayload struct {
	Level   string `json:"level"`
	Payload string `json:"payload"`
}

type delayEventPayload struct {
	URL   string `json:"url"`
	Name  string `json:"name"`
	Value int32  `json:"value"`
}

func emitEvent(evtType string, payload interface{}) {
	payloadJSON, err := json.Marshal(payload)
	if err != nil {
		return
	}
	eventJSON := fmt.Sprintf(`{"type":%q,"data":%s}`, evtType, string(payloadJSON))

	// 这块内存归 Go:回调(Rust)按 callback.h 约定只拷贝、不保存、不 free,
	// 返回后必须由这里释放,否则每条日志泄漏一份。
	cs := C.CString(eventJSON)
	defer C.free(unsafe.Pointer(cs))

	// C 全局 g_proxyzms_event_cb/ud 由 proxyzms_set_event_callback 在写锁下更新,
	// 读侧必须持读锁调用,否则 cb 与 ud 可能读到新旧混合的值。
	// 回调只做 strdup + mpsc send,不会阻塞;写锁因此只会短暂等待在途回调。
	callbackMu.RLock()
	defer callbackMu.RUnlock()
	C.invoke_proxyzms_event(cs)
}

// hookLogsOnce 订阅 mihomo 日志并转发为事件;已 hook 时是 no-op。
func hookLogsOnce() {
	eventMu.Lock()
	defer eventMu.Unlock()
	if logHk != nil {
		return
	}
	h := &logHook{sub: log.Subscribe(), done: make(chan struct{})}
	logHk = h
	go h.run()

	// delay event hook 不在上游 mihomo adapter 中（那是 FlClash 自己 fork 的 patch），
	// 延迟结果由 Rust 主动调 proxyzms_test_delay / get_proxies 拿，本桥不主动推。
}

func (h *logHook) run() {
	defer close(h.done)
	// 订阅 channel 被 UnSubscribe 关闭后循环自然结束
	for ev := range h.sub {
		if ev.LogLevel < log.Level() {
			continue
		}
		emitEvent("log", logEventPayload{Level: ev.LogLevel.String(), Payload: ev.Payload})
	}
}

// unhookLogs 退订并等待转发 goroutine 退出;返回后不会再有新的回调。
func unhookLogs() {
	eventMu.Lock()
	h := logHk
	logHk = nil
	eventMu.Unlock()
	if h == nil {
		return
	}
	// 顺序不能反:UnSubscribe 要拿 observable 的锁,而 observable 可能正持锁
	// 阻塞在向本订阅者写入。所以必须在转发 goroutine 仍在排空时退订,
	// 退订会关闭 channel,goroutine 排完剩余事件后自行退出。
	log.UnSubscribe(h.sub)
	<-h.done
}
