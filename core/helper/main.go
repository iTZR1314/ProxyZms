//go:build darwin && cgo

// proxyzms-helper:macOS 上的 root 守护进程,为没有特权的 ProxyZms 主 App
// 建立 utun 设备并把 fd 通过 SCM_RIGHTS 传回。只在本机 unix socket 上监听,
// 并通过 getpeereid 将请求限制到安装时记录的 console 用户 UID。
//
// 用法:
//
//	proxyzms-helper -socket /var/run/proxyzms-helper.sock
//
// 由 launchd 拉起(LaunchDaemon plist, RunAtLoad=YES, KeepAlive=YES);
// 收到 SIGTERM 时清理 socket 并退出。
package main

/*
#include <sys/types.h>
#include <sys/socket.h>
#include <unistd.h>
static int proxyzms_get_peer_uid(int fd, uid_t *uid) {
	gid_t gid;
	return getpeereid(fd, uid, &gid);
}
*/
import "C"

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"net"
	"net/netip"
	"os"
	"os/signal"
	"path/filepath"
	"reflect"
	"syscall"
	"time"

	"github.com/metacubex/sing-tun"
)

const (
	opTunOn               = "tun-on"
	opTunOff              = "tun-off"
	opPing                = "ping"
	helperProtocolVersion = 2
)

var clientUID = flag.Int("client-uid", -1, "allowed unprivileged client UID")

type request struct {
	Op   string    `json:"op"`
	Name string    `json:"name,omitempty"` // utunN,空则由系统分配
	Tun  *tunSetup `json:"tun,omitempty"`
}

type tunSetup struct {
	MTU                      uint32         `json:"mtu"`
	AutoRoute                bool           `json:"auto-route"`
	StrictRoute              bool           `json:"strict-route"`
	Inet4Address             []netip.Prefix `json:"inet4-address"`
	Inet6Address             []netip.Prefix `json:"inet6-address"`
	RouteAddress             []netip.Prefix `json:"route-address"`
	RouteExcludeAddress      []netip.Prefix `json:"route-exclude-address"`
	Inet4RouteAddress        []netip.Prefix `json:"inet4-route-address"`
	Inet6RouteAddress        []netip.Prefix `json:"inet6-route-address"`
	Inet4RouteExcludeAddress []netip.Prefix `json:"inet4-route-exclude-address"`
	Inet6RouteExcludeAddress []netip.Prefix `json:"inet6-route-exclude-address"`
}

type response struct {
	OK      bool   `json:"ok"`
	Error   string `json:"error,omitempty"`
	Name    string `json:"name,omitempty"`
	Fd      int    `json:"fd,omitempty"` // 仅供日志参考;真正的 fd 走 SCM_RIGHTS
	Version uint32 `json:"version,omitempty"`
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
		logf("must run as root (launchd LaunchDaemon), uid=%d", os.Geteuid())
		os.Exit(1)
	}
	if *clientUID < 0 {
		logf("client UID is not configured")
		os.Exit(1)
	}
	logf("starting, pid=%d", os.Getpid())
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
	// socket 放开连接权限,由每条连接的 getpeereid 校验登录用户 UID。
	_ = os.Chmod(*sockPath, 0o666)

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
	if err := authorizeClient(conn); err != nil {
		logf("reject client: %v", err)
		writeResp(conn, response{OK: false, Error: err.Error()})
		return
	}

	var req request
	if err := json.NewDecoder(conn).Decode(&req); err != nil {
		logf("handle: bad json: %v", err)
		writeResp(conn, response{OK: false, Error: "bad request: " + err.Error()})
		return
	}
	logf("handle: op=%s name=%s", req.Op, req.Name)

	switch req.Op {
	case opPing:
		writeResp(conn, response{OK: true, Version: helperProtocolVersion})

	case opTunOn:
		if req.Tun == nil {
			writeResp(conn, response{OK: false, Error: "missing effective tun config"})
			return
		}
		fd, name, err := openTun(req.Tun, req.Name)
		if err != nil {
			logf("openTun failed: %v", err)
			writeResp(conn, response{OK: false, Error: err.Error()})
			return
		}
		if err := sendFd(conn, fd); err != nil {
			logf("sendFd failed: %v", err)
			writeResp(conn, response{OK: false, Error: "sendmsg: " + err.Error()})
			return
		}
		logf("fd sent: fd=%d name=%s", fd, name)
		writeResp(conn, response{OK: true, Name: name, Fd: fd})

	case opTunOff:
		closeTun()
		writeResp(conn, response{OK: true})

	default:
		writeResp(conn, response{OK: false, Error: "unknown op: " + req.Op})
	}
}

func authorizeClient(conn net.Conn) error {
	unixConn, ok := conn.(*net.UnixConn)
	if !ok {
		return errors.New("client is not a unix socket")
	}
	rawConn, err := unixConn.SyscallConn()
	if err != nil {
		return fmt.Errorf("get client socket: %w", err)
	}
	var peerUID C.uid_t
	var result C.int
	err = rawConn.Control(func(fd uintptr) {
		result = C.proxyzms_get_peer_uid(C.int(fd), &peerUID)
	})
	if err != nil {
		return fmt.Errorf("inspect client credentials: %w", err)
	}
	if result != 0 {
		return errors.New("getpeereid failed")
	}
	if int64(peerUID) != int64(*clientUID) {
		return fmt.Errorf("unauthorized client uid %d", uint32(peerUID))
	}
	return nil
}

func openTun(setup *tunSetup, name string) (int, string, error) {
	logf("openTun: mtu=%d name=%s auto-route=%v", setup.MTU, name, setup.AutoRoute)
	if current.t != nil {
		// 已开:重复调用直接返回现有(幂等)
		logf("openTun: reuse existing fd=%d name=%s", current.fd, current.name)
		return current.fd, current.name, nil
	}
	opts, err := buildTunOptions(setup, name)
	if err != nil {
		return -1, "", err
	}
	t, err := tun.New(opts)
	if err != nil {
		return -1, "", fmt.Errorf("tun.New: %w", err)
	}
	logf("openTun: sing-tun created OK, extracting fd via reflect")
	name2 := opts.Name
	if name2 == "" {
		_ = t.Close()
		return -1, "", fmt.Errorf("must specify tun name (e.g. utun9)")
	}
	fd := extractDarwinFd(t)
	if fd < 0 {
		_ = t.Close()
		return -1, "", fmt.Errorf("cannot extract fd from darwin tun (reflect missed tunFd)")
	}
	logf("openTun: extracted fd=%d name=%s", fd, name2)
	current.t = t
	current.fd = fd
	current.name = name2
	return fd, name2, nil
}

func buildTunOptions(setup *tunSetup, name string) (tun.Options, error) {
	mtu := setup.MTU
	if mtu == 0 {
		mtu = 9000 // mihomo sing_tun 默认值
	}
	inet4Route, inet6Route := splitPrefixes(setup.RouteAddress)
	inet4Route = append(inet4Route, setup.Inet4RouteAddress...)
	inet6Route = append(inet6Route, setup.Inet6RouteAddress...)
	inet4Exclude, inet6Exclude := splitPrefixes(setup.RouteExcludeAddress)
	inet4Exclude = append(inet4Exclude, setup.Inet4RouteExcludeAddress...)
	inet6Exclude = append(inet6Exclude, setup.Inet6RouteExcludeAddress...)
	if setup.AutoRoute && len(setup.Inet4Address)+len(setup.Inet6Address) == 0 {
		return tun.Options{}, fmt.Errorf("auto-route enabled but effective TUN addresses are empty")
	}
	opts := tun.Options{
		Name:                     name,
		MTU:                      mtu,
		AutoRoute:                setup.AutoRoute,
		StrictRoute:              setup.StrictRoute,
		Inet4Address:             setup.Inet4Address,
		Inet6Address:             setup.Inet6Address,
		Inet4RouteAddress:        inet4Route,
		Inet6RouteAddress:        inet6Route,
		Inet4RouteExcludeAddress: inet4Exclude,
		Inet6RouteExcludeAddress: inet6Exclude,
	}
	return opts, nil
}

func splitPrefixes(prefixes []netip.Prefix) ([]netip.Prefix, []netip.Prefix) {
	var inet4, inet6 []netip.Prefix
	for _, prefix := range prefixes {
		if prefix.Addr().Is4() {
			inet4 = append(inet4, prefix)
		} else if prefix.Addr().Is6() {
			inet6 = append(inet6, prefix)
		}
	}
	return inet4, inet6
}

func logf(format string, args ...interface{}) {
	fmt.Fprintf(os.Stderr, "[helper] "+format+"\n", args...)
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
