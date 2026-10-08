//go:build cgo && !darwin

package main

import "errors"

// 外部 TUN fd 注入只在 macOS(root helper + SCM_RIGHTS)使用;
// 其它平台 mihomo 自己创建 TUN,这里仅为 state.go 能通过编译。
func dupFD(int) (int, error) { return -1, errors.New("tun fd adoption is only supported on darwin") }

func closeFD(int) {}
