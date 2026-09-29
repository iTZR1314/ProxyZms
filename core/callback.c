#include "callback.h"

proxyzms_event_cb g_proxyzms_event_cb = 0;
void*             g_proxyzms_event_ud = 0;

void invoke_proxyzms_event(const char* json) {
    proxyzms_event_cb cb = g_proxyzms_event_cb;
    if (cb == 0) {
        return;
    }
    cb(json, g_proxyzms_event_ud);
}
