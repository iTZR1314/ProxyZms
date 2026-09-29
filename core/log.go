//go:build cgo

package main

import (
	"fmt"
	"strings"

	"github.com/metacubex/mihomo/log"
)

func setLogLevel(level string) error {
	l, ok := parseLogLevel(level)
	if !ok {
		return fmt.Errorf("unknown log level: %s", level)
	}
	log.SetLevel(l)
	return nil
}

func parseLogLevel(s string) (log.LogLevel, bool) {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "debug":
		return log.DEBUG, true
	case "info":
		return log.INFO, true
	case "warning", "warn":
		return log.WARNING, true
	case "error":
		return log.ERROR, true
	case "silent":
		return log.SILENT, true
	default:
		return 0, false
	}
}
