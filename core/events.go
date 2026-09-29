//go:build cgo

package main

/*
#include "callback.h"
*/
import "C"

import (
	"encoding/json"
	"fmt"
	"sync"

	"github.com/metacubex/mihomo/log"
)

var eventMu sync.Mutex
var logStopOnce sync.Once
var logStopCh chan struct{}

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
	C.invoke_proxyzms_event(C.CString(eventJSON))
}

func hookLogsOnce() {
	logStopOnce.Do(func() {
		sub := log.Subscribe()
		logStopCh = make(chan struct{})
		go func() {
			for {
				select {
				case ev, ok := <-sub:
					if !ok {
						return
					}
					if ev.LogLevel < log.Level() {
						continue
					}
					emitEvent("log", logEventPayload{Level: ev.LogLevel.String(), Payload: ev.Payload})
				case <-logStopCh:
					return
				}
			}
		}()
	})

	// delay event hook 不在上游 mihomo adapter 中（那是 FlClash 自己 fork 的 patch），
	// 延迟结果由 Rust 主动调 proxyzms_test_delay / get_proxies 拿，本桥不主动推。
}

func unhookLogs() {
	eventMu.Lock()
	defer eventMu.Unlock()
	if logStopCh != nil {
		select {
		case <-logStopCh:
		default:
			close(logStopCh)
		}
		logStopCh = nil
	}
	logStopOnce = sync.Once{}
}
