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
)

var callbackMu sync.RWMutex

type apiResponse struct {
	OK    bool            `json:"ok"`
	Data  json.RawMessage `json:"data,omitempty"`
	Error string          `json:"error,omitempty"`
}

func marshalResponse(resp apiResponse) *C.char {
	data, err := json.Marshal(resp)
	if err != nil {
		return C.CString(`{"ok":false,"error":"marshal error"}`)
	}
	return C.CString(string(data))
}

func okPayload(data json.RawMessage) *C.char {
	return marshalResponse(apiResponse{OK: true, Data: data})
}

func errPayload(err error) *C.char {
	return marshalResponse(apiResponse{OK: false, Error: err.Error()})
}

func rawJsonString(data string) *C.char {
	return okPayload(json.RawMessage(data))
}

func okBool(v bool) *C.char {
	buf, _ := json.Marshal(v)
	return okPayload(buf)
}

//export proxyzms_set_event_callback
func proxyzms_set_event_callback(cb C.proxyzms_event_cb, userData unsafe.Pointer) {
	callbackMu.Lock()
	defer callbackMu.Unlock()
	C.g_proxyzms_event_cb = cb
	C.g_proxyzms_event_ud = userData
}

//export proxyzms_init
func proxyzms_init(homeDir *C.char) *C.char {
	hookLogsOnce()
	home := C.GoString(homeDir)
	if err := initKernel(home); err != nil {
		return errPayload(err)
	}
	return okBool(true)
}

//export proxyzms_apply_config
func proxyzms_apply_config(yaml *C.char, selectedMapJSON *C.char) *C.char {
	y := C.GoString(yaml)
	selectedMap := map[string]string{}
	if selectedMapJSON != nil {
		raw := C.GoString(selectedMapJSON)
		if raw != "" {
			_ = json.Unmarshal([]byte(raw), &selectedMap)
		}
	}
	warning, err := applyConfig(y, selectedMap)
	if err != nil {
		return errPayload(err)
	}
	if warning != "" {
		return rawJsonString(fmt.Sprintf(`{"warning":%q}`, warning))
	}
	return okBool(true)
}

//export proxyzms_shutdown
func proxyzms_shutdown() *C.char {
	unhookLogs()
	if err := stopKernel(); err != nil {
		return errPayload(err)
	}
	return okBool(true)
}

//export proxyzms_is_running
func proxyzms_is_running() *C.char {
	return okBool(isKernelRunning())
}

//export proxyzms_set_mode
func proxyzms_set_mode(mode *C.char) *C.char {
	if err := setMode(C.GoString(mode)); err != nil {
		return errPayload(err)
	}
	return okBool(true)
}

//export proxyzms_get_mode
func proxyzms_get_mode() *C.char {
	m, err := getMode()
	if err != nil {
		return errPayload(err)
	}
	return rawJsonString(fmt.Sprintf(`"%s"`, m))
}

//export proxyzms_set_tun
func proxyzms_set_tun(enable C.int) *C.char {
	if err := setTun(enable != 0); err != nil {
		return errPayload(err)
	}
	return okBool(true)
}

//export proxyzms_adopt_tun
func proxyzms_adopt_tun(fd C.int) *C.char {
	if err := adoptTunFD(int(fd)); err != nil {
		return errPayload(err)
	}
	return okBool(true)
}

//export proxyzms_set_log_level
func proxyzms_set_log_level(level *C.char) *C.char {
	if err := setLogLevel(C.GoString(level)); err != nil {
		return errPayload(err)
	}
	return okBool(true)
}

//export proxyzms_validate_config
func proxyzms_validate_config(yaml *C.char) *C.char {
	if _, err := validateConfig(C.GoString(yaml)); err != nil {
		return errPayload(err)
	}
	return okBool(true)
}

//export proxyzms_get_proxies
func proxyzms_get_proxies() *C.char {
	data, err := getProxies()
	if err != nil {
		return errPayload(err)
	}
	return rawJsonString(data)
}

//export proxyzms_select_proxy
func proxyzms_select_proxy(groupName *C.char, proxyName *C.char) *C.char {
	if err := selectProxy(C.GoString(groupName), C.GoString(proxyName)); err != nil {
		return errPayload(err)
	}
	return okBool(true)
}

//export proxyzms_test_delay
func proxyzms_test_delay(proxyName *C.char, testURL *C.char, timeoutMs C.int) *C.char {
	data, err := testDelay(C.GoString(proxyName), C.GoString(testURL), int64(timeoutMs))
	if err != nil {
		return errPayload(err)
	}
	return rawJsonString(data)
}

//export proxyzms_get_connections
func proxyzms_get_connections() *C.char {
	data, err := getConnections()
	if err != nil {
		return errPayload(err)
	}
	return rawJsonString(data)
}

//export proxyzms_close_connection
func proxyzms_close_connection(id *C.char) *C.char {
	if err := closeConnection(C.GoString(id)); err != nil {
		return errPayload(err)
	}
	return okBool(true)
}

//export proxyzms_get_traffic
func proxyzms_get_traffic(total C.int) *C.char {
	data, err := getTraffic(total != 0)
	if err != nil {
		return errPayload(err)
	}
	return rawJsonString(data)
}

//export proxyzms_update_subscription
func proxyzms_update_subscription(name *C.char) *C.char {
	if err := updateSubscription(C.GoString(name)); err != nil {
		return errPayload(err)
	}
	return okBool(true)
}

//export proxyzms_free_string
func proxyzms_free_string(s *C.char) {
	if s != nil {
		C.free(unsafe.Pointer(s))
	}
}

func main() {}
