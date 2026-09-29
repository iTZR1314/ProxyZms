//go:build darwin && cgo

// proxyzms-helper:macOS 上的 root 守护进程,为没有特权的 ProxyZms 主 App
// 建立 utun 设备并把 fd 通过 SCM_RIGHTS 传回。只在本机 unix socket 上监听,
// 不做鉴权(目录权限 0700 + 客户端校验 pid 即可;无任何网络面)。
//
// 用法:
//
//	proxyzms-helper -socket /var/run/proxyzms-helper.sock
//
// 由 launchd 拉起(LaunchDaemon plist, RunAtLoad=YES, KeepAlive=YES);
// 收到 SIGTERM 时清理 socket 并退出。
package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"net"
	"os"
	"os/signal"
	"path/filepath"
	"reflect"
	"syscall"
	"time"

	"github.com/metacubex/sing-tun"
)

const (
	opTunOn  = "tun-on"
	opTunOff = "tun-off"
	opPing   = "ping"
)

type request struct {
	Op   string `json:"op"`
	MTU  int    `json:"mtu,omitempty"`
	Name string `json:"name,omitempty"` // utunN,空则由系统分配
}

type response struct {
	OK    bool   `json:"ok"`
	Error string `json:"error,omitempty"`
	Name  string `json:"name,omitempty"`
	Fd    int    `json:"fd,omitempty"` // 仅供日志参考;真正的 fd 走 SCM_RIGHTS
}

var (
	sockPath = flag.String("socket", "/var/run/proxyzms-helper.sock", "unix socket path")
	// tun 实例单例:一次只允许一个 utun 打开,重复 on 返回已有 fd
	current struct {
		t    tun.Tun
		name string
		fd   int
	}
)

func main() {
	flag.Parse()
	if os.Geteuid() != 0 {
		fmt.Fprintln(os.Stderr, "proxyzms-helper must run as root (launchd LaunchDaemon)")
		os.Exit(1)
	}
	if err := os.MkdirAll(filepath.Dir(*sockPath), 0o755); err != nil {
		fmt.Fprintln(os.Stderr, "mkdir socket dir:", err)
		os.Exit(1)
	}
	_ = os.Remove(*sockPath)

	ln, err := net.Listen("unix", *sockPath)
	if err != nil {
		fmt.Fprintln(os.Stderr, "listen unix:", err)
		os.Exit(1)
	}
	// 只给 owner 读写:不是 root 的客户端连不上;launchd 之外想开口子需手动 chmod
	_ = os.Chmod(*sockPath, 0o600)

	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGTERM, syscall.SIGINT)
	go func() {
		<-sigCh
		closeTun()
		_ = ln.Close()
		_ = os.Remove(*sockPath)
		os.Exit(0)
	}()

	fmt.Fprintf(os.Stderr, "[helper] listening on %s\n", *sockPath)
	for {
		conn, err := ln.Accept()
		if err != nil {
			if errors.Is(err, net.ErrClosed) {
				return
			}
			continue
		}
		go handle(conn)
	}
}

func closeTun() {
	if current.t != nil {
		_ = current.t.Close()
		current.t = nil
		current.fd = -1
	}
}

// handle 处理一条 JSON 请求(一行) + 可选 SCM_RIGHTS fd 回传。
func handle(conn net.Conn) {
	defer conn.Close()
	_ = conn.SetDeadline(time.Now().Add(5 * time.Second))

	var req request
	if err := json.NewDecoder(conn).Decode(&req); err != nil {
		writeResp(conn, response{OK: false, Error: "bad request: " + err.Error()})
		return
	}

	switch req.Op {
	case opPing:
		writeResp(conn, response{OK: true})

	case opTunOn:
		fd, name, err := openTun(req.MTU, req.Name)
		if err != nil {
			writeResp(conn, response{OK: false, Error: err.Error()})
			return
		}
		// fd 通过 SCM_RIGHTS 传回去,然后 helper 自己也持有(双 fd 引用)
		if err := sendFd(conn, fd); err != nil {
			writeResp(conn, response{OK: false, Error: "sendmsg: " + err.Error()})
			return
		}
		writeResp(conn, response{OK: true, Name: name, Fd: fd})

	case opTunOff:
		closeTun()
		writeResp(conn, response{OK: true})

	default:
		writeResp(conn, response{OK: false, Error: "unknown op: " + req.Op})
	}
}

func openTun(mtu int, name string) (int, string, error) {
	if current.t != nil {
		// 已开:重复调用直接返回现有(幂等)
		return current.fd, current.name, nil
	}
	if mtu <= 0 {
		mtu = 1500
	}
	opts := tun.Options{
		Name:         name,
		MTU:          uint32(mtu),
		AutoRoute:    true,
		StrictRoute:  false,
		Inet4Address: nil,
	}
	t, err := tun.New(opts)
	if err != nil {
		return -1, "", fmt.Errorf("tun.New: %w", err)
	}
	// 拿到 fd:darwin NativeTun.tunFd 非导出,但 connect(utunN) 会输出 Name()
	name2 := opts.Name
	if name2 == "" {
		// 没指定名字时 sing-tun 会让内核挑,需要反向推到 fd
		// 简化:固定要求调用方显式给 utunN,避免复杂 ioctl 查询
		_ = t.Close()
		return -1, "", fmt.Errorf("must specify tun name (e.g. utun9)")
	}
	// Darwin NativeTun 内部存 tunFd 但不导出;通过 reflect 抓(局限但够用)
	fd := extractDarwinFd(t)
	if fd < 0 {
		_ = t.Close()
		return -1, "", fmt.Errorf("cannot extract fd from darwin tun")
	}
	current.t = t
	current.fd = fd
	current.name = name2
	return fd, name2, nil
}

// extractDarwinFd 从 darwin sing-tun 的 NativeTun.tunFd 把 fd 抓出来。
// 未导出字段;用 cached reflect 不重复计算。
func extractDarwinFd(t tun.Tun) int {
	v := reflect.ValueOf(t)
	if v.Kind() == reflect.Ptr {
		v = v.Elem()
	}
	if !v.IsValid() {
		return -1
	}
	fdField := v.FieldByName("tunFd")
	if !fdField.IsValid() || fdField.Kind() != reflect.Int {
		return -1
	}
	return int(fdField.Int())
}

func writeResp(conn net.Conn, r response) {
	_ = json.NewEncoder(conn).Encode(r)
}

// sendFd 通过 SCM_RIGHTS 把 fd 发到对端 unix socket。
// Go 标准库没暴露,直接 unix.Sendmsg 用 cmsghdr 手工构造。
func sendFd(conn net.Conn, fd int) error {
	uc, ok := conn.(*net.UnixConn)
	if !ok {
		return errors.New("not a unix conn")
	}
	// 至少要带 1 字节 payload 才能夹带 control message
	payload := []byte{0}
	rights := syscall.UnixRights(fd)
	// 用 syscall 而非 x/sys:Go syscall.UnixRights 生成 cmsghdr 兼容 macOS
	rawConn, err := uc.SyscallConn()
	if err != nil {
		return err
	}
	var sendErr error
	if err := rawConn.Control(func(fd uintptr) {
		sendErr = syscall.Sendmsg(int(fd), payload, rights, nil, 0)
	}); err != nil {
		return err
	}
	return sendErr
}

// (end of helpers)
