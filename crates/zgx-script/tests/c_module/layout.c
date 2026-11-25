/* Prints the sizes and offsets of zgx_module.h, one "name value" a line,
 * for tests/native_modules.rs to hold against Rust's. */
#include <stdio.h>
#include "zgx_module.h"

#define SIZE(type) printf("sizeof(" #type ") %zu\n", sizeof(type))
#define AT(type, field) printf(#type "." #field " %zu\n", offsetof(type, field))

int main(void) {
    printf("ZGX_MODULE_ABI %u\n", ZGX_MODULE_ABI);
    printf("ZGX_COUNT_BEFORE %d\n", ZGX_COUNT_BEFORE);
    printf("ZGX_MANIFEST_HEAD %d\n", ZGX_MANIFEST_HEAD);
    printf("kinds %d %d %d %d %d %d %d %d\n", ZGX_NIL, ZGX_BOOL, ZGX_NUM, ZGX_STR, ZGX_TEXT,
           ZGX_LIST, ZGX_MAP, ZGX_FOLDED);
    printf("statuses %u %u %u %u\n", ZGX_DONE, ZGX_FAILED, ZGX_REFUSED_ABI, ZGX_SETUP_PANICKED);
    printf("highlight %u %u\n", ZGX_HIGHLIGHT_FAILED, ZGX_HIGHLIGHT_STATES);
    printf("posted %u %u %u\n", ZGX_POSTED, ZGX_GONE, ZGX_HOST_GONE);
    printf("sum %u\n", zgx_manifest_sum((const uint8_t *)"{\"name\":\"c\"}", 12));
    SIZE(zgx_value);
    AT(zgx_value, kind);
    AT(zgx_value, boolean);
    AT(zgx_value, held);
    AT(zgx_value, len);
    SIZE(zgx_field);
    AT(zgx_field, key);
    AT(zgx_field, key_len);
    AT(zgx_field, value);
    SIZE(zgx_bytes);
    AT(zgx_bytes, ptr);
    AT(zgx_bytes, len);
    SIZE(zgx_api);
    AT(zgx_api, abi);
    AT(zgx_api, size);
    AT(zgx_api, string);
    AT(zgx_api, list);
    AT(zgx_api, map);
    AT(zgx_api, slice);
    AT(zgx_api, clone);
    AT(zgx_api, release);
    AT(zgx_api, open);
    AT(zgx_api, flat);
    AT(zgx_api, host_retain);
    AT(zgx_api, host_release);
    AT(zgx_api, post);
    AT(zgx_api, after);
    AT(zgx_api, timer_release);
    AT(zgx_api, state);
    AT(zgx_api, shared);
    AT(zgx_api, shared_get);
    AT(zgx_api, shared_set);
    AT(zgx_api, shared_append);
    AT(zgx_api, shared_observe);
    AT(zgx_api, shared_unobserve);
    AT(zgx_api, shared_release);
    AT(zgx_api, settle);
    AT(zgx_api, promise_release);
    AT(zgx_api, promise_host);
    AT(zgx_api, shared_upsert);
    AT(zgx_api, shared_remove);
    AT(zgx_api, image_buffer);
    AT(zgx_api, image_set);
    AT(zgx_api, image_drop);
    AT(zgx_api, image_remove);
    AT(zgx_api, shared_upsert_at);
    SIZE(zgx_registrar);
    AT(zgx_registrar, size);
    AT(zgx_registrar, host);
    AT(zgx_registrar, function);
    AT(zgx_registrar, function_taking);
    AT(zgx_registrar, action);
    AT(zgx_registrar, state);
    AT(zgx_registrar, host_ref);
    AT(zgx_registrar, async_action);
    AT(zgx_registrar, highlighter);
    return 0;
}
