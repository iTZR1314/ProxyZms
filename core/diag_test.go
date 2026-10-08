//go:build cgo

package main

import (
	"encoding/json"
	"testing"
)

func TestGetDiagJSON(t *testing.T) {
	raw, err := getDiagJSON()
	if err != nil {
		t.Fatal(err)
	}
	var d diagData
	if err := json.Unmarshal([]byte(raw), &d); err != nil {
		t.Fatalf("diag JSON 不合法: %v (%s)", err, raw)
	}
	if d.Goroutines <= 0 || d.HeapInuse == 0 || d.Sys == 0 {
		t.Fatalf("诊断数值不合理: %+v", d)
	}
}
