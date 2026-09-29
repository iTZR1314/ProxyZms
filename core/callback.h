#ifndef PROXYZMS_CALLBACK_H
#define PROXYZMS_CALLBACK_H

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Rust 端通过 proxyzms_set_event_callback 注册的回调。Go 端在 mihomo 内部
 * goroutine 中调用 invoke_proxyzms_event → g_proxyzms_event_cb。json 为
 * UTF-8 NUL 结尾；Rust 必须立即 strdup/拷贝后返回，不得保存指针。
 * user_data 是 Rust 注册时透传的不透明指针（一般是
 * Box<Mutex<Sender<CoreEvent>>> 的裸指针）。
 *
 * 取消观察只需传 NULL cb。
 */
typedef void (*proxyzms_event_cb)(const char* json, void* user_data);

/*
 * 全局钩子（每进程仅一份）。Go 的 goroutine 无法把函数指针回传到调用方，
 * 因此通过 C 全局变量中转。
 */
extern proxyzms_event_cb g_proxyzms_event_cb;
extern void*             g_proxyzms_event_ud;

void invoke_proxyzms_event(const char* json);

#ifdef __cplusplus
}
#endif

#endif /* PROXYZMS_CALLBACK_H */
