//go:build darwin && cgo

package main

import "golang.org/x/sys/unix"

// dupFD 复制一份 fd(CLOEXEC,编号 ≥3,避开 stdio)。
func dupFD(fd int) (int, error) {
	return unix.FcntlInt(uintptr(fd), unix.F_DUPFD_CLOEXEC, 3)
}

func closeFD(fd int) { _ = unix.Close(fd) }
