//go:build cgo

package main

import (
	"encoding/json"
	"runtime"
)

// diagData 是设置页「诊断」面板用的 Go 运行时快照。
// 只读运行时统计,不依赖内核是否已 init。
type diagData struct {
	HeapAlloc  uint64 `json:"heap_alloc"`
	HeapInuse  uint64 `json:"heap_inuse"`
	Sys        uint64 `json:"sys"`
	Goroutines int    `json:"goroutines"`
	NumGC      uint32 `json:"num_gc"`
}

func getDiagJSON() (string, error) {
	var m runtime.MemStats
	runtime.ReadMemStats(&m) // 会短暂 stop-the-world,仅手动刷新时调用
	buf, err := json.Marshal(diagData{
		HeapAlloc:  m.HeapAlloc,
		HeapInuse:  m.HeapInuse,
		Sys:        m.Sys,
		Goroutines: runtime.NumGoroutine(),
		NumGC:      m.NumGC,
	})
	if err != nil {
		return "", err
	}
	return string(buf), nil
}
