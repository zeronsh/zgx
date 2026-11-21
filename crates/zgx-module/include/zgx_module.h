/* zgx native modules: the interface, for C and for anything that speaks C.
 *
 * A module is a library the zgx runtime loads. It exports two symbols, each
 * ending in the module's name (letters, digits and `_`):
 *
 *   zgx_module_manifest_<name>   data: what the module is and registers
 *   zgx_module_v1_<name>         its entry: registers its functions
 *
 * Written by hand from the Rust definition, `zgx_module::abi`
 * (crates/zgx-module/src/abi.rs), which is the source; zgx's tests compile
 * this file and compare every size and offset here with Rust's.
 *
 * THREADS. Every function here is called on the thread that loaded the
 * module (the interface's), and a module calls the runtime's functions on
 * that thread only. A value's count of holders is not atomic: a module
 * touches a value, through a function or in place, only on that thread.
 * The runtime cannot see a count made in place on another thread.
 *
 * WHAT A MODULE MAY RELY ON. The functions in `zgx_api` are the contract:
 * make a value, hold it once more, let go of it, read a list's items and a
 * map's fields. The `static inline` functions below read and count values
 * where the runtime holds them, by its stated layout: a faster way to do
 * the same, and optional. A module that uses none of them is still right.
 *
 * 64-bit targets only.
 */
#ifndef ZGX_MODULE_H
#define ZGX_MODULE_H

#include <stddef.h>
#include <stdint.h>
#include <string.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The interface's number. A module and a runtime agree on it or the module
 * doesn't load. It changes only when something here is taken away or
 * changes meaning; what is added goes at the end of `zgx_api` and
 * `zgx_registrar`, told by their `size`. */
#define ZGX_MODULE_ABI 1u

/* A value's kinds. Any other (7, 8, or past 9) is one only the runtime
 * reads: a module passes it on, and holds and lets go of it through
 * `zgx_api.clone` and `zgx_api.release`. */
enum {
    ZGX_NIL = 0,
    ZGX_BOOL = 1,   /* `boolean` is 0 or 1 */
    ZGX_NUM = 2,    /* `held.number` */
    ZGX_STR = 3,    /* `held.at` is its UTF-8, `len` its bytes */
    ZGX_TEXT = 4,   /* a string past 64 KiB, held in pieces: `zgx_api.flat` */
    ZGX_LIST = 5,   /* `held.at` is its items (`zgx_value`), `len` how many */
    ZGX_MAP = 6,    /* `held.at` is its fields (`zgx_field`), sorted by key */
    ZGX_FOLDED = 9  /* a row kept folded until read: `zgx_api.open` */
};

/* A value: three words. Owned (given back once, or handed to the runtime)
 * or lent (read while what lent it lives): which one is by where it came
 * from, said at each function. */
typedef struct zgx_value {
    uint8_t kind;
    uint8_t boolean;
    uint8_t unused[6];
    union {
        double number;
        const void *at;
    } held;
    size_t len;
} zgx_value;

/* A map's field: its key (UTF-8, not NUL-terminated) and its value. */
typedef struct zgx_field {
    const char *key;
    size_t key_len;
    zgx_value value;
} zgx_field;

/* A string's bytes, lent. */
typedef struct zgx_bytes {
    const uint8_t *ptr;
    size_t len;
} zgx_bytes;

/* How far before a string's text, a list's items or a map's fields its
 * count of holders is: a `size_t`. */
#define ZGX_COUNT_BEFORE 16

/* The runtime's functions for values: the same for every module, for as
 * long as the process runs. Each writes what it makes to `out`, owned. */
typedef struct zgx_api {
    uint32_t abi;  /* ZGX_MODULE_ABI */
    uint32_t size; /* this struct's size as the runtime built it: read no further */
    /* A string of these bytes (UTF-8). */
    void (*string)(const uint8_t *ptr, size_t len, zgx_value *out);
    /* A list of these `len` owned values, which it takes. */
    void (*list)(zgx_value *items, size_t len, zgx_value *out);
    /* A map of these `len` keys and owned values, which it takes, in any
     * order (the last of a key given twice kept). */
    void (*map)(const zgx_bytes *keys, zgx_value *values, size_t len, zgx_value *out);
    /* A list of a list's items `from` to `to` (from 0, `to` excluded), the
     * items held once more, not copied. `nil` if they aren't in it. */
    void (*slice)(const zgx_value *list, size_t from, size_t to, zgx_value *out);
    /* The same value once more: held, not copied. */
    void (*clone)(const zgx_value *value, zgx_value *out);
    /* Gives an owned value back. */
    void (*release)(zgx_value *value);
    /* What a ZGX_FOLDED value opens to, lent for as long as it is; NULL
     * for any other. */
    const zgx_value *(*open)(const zgx_value *value);
    /* A ZGX_TEXT value as one string of all its text (a copy); `nil` for
     * any other. */
    void (*flat)(const zgx_value *value, zgx_value *out);
} zgx_api;

/* What a call returns: done, or failed. Failing, it wrote why (a string)
 * to `out`, and the runtime raises that in the script that called, naming
 * the function and the module. */
#define ZGX_DONE 0u
#define ZGX_FAILED 1u

/* A module's function, as a script calls it (`ui.call_sync(name, arg)`, or
 * by name on the module's table from `ui.module`): `args` are `len` values,
 * lent for the call. There is always one to read: called with none, it is
 * lent a `nil` and `len` is 0. It always writes `out`, owned by the runtime
 * from then: its result and ZGX_DONE, or why it failed and ZGX_FAILED. */
typedef uint32_t (*zgx_function)(void *user, const zgx_value *args, size_t len, zgx_value *out);
/* A function handed its first argument: `arg` is the function's, which
 * gives it back (`zgx_release`) or hands it on (`*out` may be `*arg`). */
typedef uint32_t (*zgx_function_taking)(void *user, zgx_value *arg, zgx_value *out);
/* A module's action (`ui.action(name, arg)`, `ui.invoke`): `arg` lent. It
 * writes `out` only when it fails. */
typedef uint32_t (*zgx_action)(void *user, const zgx_value *arg, zgx_value *out);
/* Frees a registration's `user` when the host that held it goes. */
typedef void (*zgx_free)(void *user);

/* What a module registers with as it loads: valid only inside its entry.
 * Names are UTF-8, `len` bytes, not NUL-terminated. */
typedef struct zgx_registrar {
    uint32_t size; /* as zgx_api.size */
    void *host;    /* handed back in each call below */
    void (*function)(void *host, const uint8_t *name, size_t len, zgx_function call, void *user,
                     zgx_free free);
    void (*function_taking)(void *host, const uint8_t *name, size_t len, zgx_function_taking call,
                            void *user, zgx_free free);
    void (*action)(void *host, const uint8_t *name, size_t len, zgx_action call, void *user,
                   zgx_free free);
    /* The module's state for this thread, by a name of its own: `size`
     * bytes at `align`, made the first time the thread asks (`init` writes
     * it) and the same pointer each time after; freed (`free`) as the
     * thread ends. */
    uint8_t *(*state)(void *host, const uint8_t *name, size_t len, size_t size, size_t align,
                      void (*init)(uint8_t *state), void (*free)(uint8_t *state));
} zgx_registrar;

/* A module's entry, called once per host it is loaded into. Returns 0, or
 * a reason it can't load (ZGX_REFUSED_ABI: the runtime speaks another
 * interface, or has a shorter table than the module needs). */
#define ZGX_REFUSED_ABI 1u
#define ZGX_MODULE_ENTRY(name) \
    uint32_t zgx_module_v1_##name(const zgx_api *api, const zgx_registrar *registrar)

/* A module's manifest: its name, and what it registers with which types,
 * as JSON (zgx's docs/modules.md has the grammar). One piece of read-only
 * data with no pointer in it:
 *
 *   16 bytes  "zgx-module-json\0"
 *    4        the format, 1, little-endian
 *    4        the JSON's length in bytes, little-endian
 *    4        zgx_manifest_sum of the JSON, little-endian
 *    4        zero
 *    ...      the JSON, UTF-8
 *   16 bytes  "\0zgx-module-end\0"
 *
 * The runtime finds it in the library's file by the first 16 bytes, reads
 * the module's name from it, and holds the module to it: a module that
 * registers anything else than its manifest lists is refused. */
#define ZGX_MODULE_MANIFEST(name) const uint8_t zgx_module_manifest_##name[]
#define ZGX_MANIFEST_HEAD 32

/* A manifest's checksum of its JSON: FNV-1a, 32 bits. */
static inline uint32_t zgx_manifest_sum(const uint8_t *json, size_t len) {
    uint32_t sum = 0x811c9dc5u;
    for (size_t i = 0; i < len; i++) sum = (sum ^ json[i]) * 0x01000193u;
    return sum;
}

/* ---- Values a module makes itself: no call. ---- */

static inline void zgx_nil(zgx_value *out) {
    memset(out, 0, sizeof *out);
}
static inline void zgx_bool(zgx_value *out, int value) {
    memset(out, 0, sizeof *out);
    out->kind = ZGX_BOOL;
    out->boolean = value != 0;
}
static inline void zgx_num(zgx_value *out, double value) {
    memset(out, 0, sizeof *out);
    out->kind = ZGX_NUM;
    out->held.number = value;
}

/* ---- The optional fast path: values read where the runtime holds them. ---- */

/* What a script would read of `value`: a folded row opened, else itself. */
static inline const zgx_value *zgx_opened(const zgx_api *api, const zgx_value *value) {
    if (value->kind != ZGX_FOLDED) return value;
    const zgx_value *opened = api->open(value);
    return opened ? opened : value;
}
/* A string's text and its length in bytes; NULL if it isn't a ZGX_STR. */
static inline const char *zgx_str(const zgx_value *value, size_t *len) {
    if (value->kind != ZGX_STR) return NULL;
    *len = value->len;
    return (const char *)value->held.at;
}
/* A list's items and how many; NULL if it isn't a list. */
static inline const zgx_value *zgx_items(const zgx_value *value, size_t *len) {
    if (value->kind != ZGX_LIST) return NULL;
    *len = value->len;
    return (const zgx_value *)value->held.at;
}
/* A map's fields, sorted by key, and how many; NULL if it isn't a map. */
static inline const zgx_field *zgx_fields(const zgx_value *value, size_t *len) {
    if (value->kind != ZGX_MAP) return NULL;
    *len = value->len;
    return (const zgx_field *)value->held.at;
}
/* A map's field by its key; NULL if it has none, or isn't a map. */
static inline const zgx_value *zgx_field_of(const zgx_value *map, const char *key, size_t key_len) {
    size_t len = 0;
    const zgx_field *fields = zgx_fields(map, &len);
    size_t low = 0, high = len;
    while (low < high) {
        size_t mid = low + (high - low) / 2;
        const zgx_field *field = &fields[mid];
        size_t shared = field->key_len < key_len ? field->key_len : key_len;
        int order = memcmp(field->key, key, shared);
        if (order == 0) order = (field->key_len > key_len) - (field->key_len < key_len);
        if (order == 0) return &field->value;
        if (order < 0) low = mid + 1; else high = mid;
    }
    return NULL;
}
/* Whether a value's holders are counted where a module can reach. */
static inline int zgx_counted(const zgx_value *value) {
    return value->kind == ZGX_STR || value->kind == ZGX_LIST || value->kind == ZGX_MAP;
}
/* `value` held once more, written to `out`: counted in place where it can
 * be, through the runtime otherwise. */
static inline void zgx_retain(const zgx_api *api, const zgx_value *value, zgx_value *out) {
    if (value->kind <= ZGX_NUM) {
        *out = *value;
    } else if (zgx_counted(value)) {
        size_t *count = (size_t *)((uint8_t *)value->held.at - ZGX_COUNT_BEFORE);
        if (*count >= SIZE_MAX / 2) {
            api->clone(value, out);
            return;
        }
        *count += 1;
        *out = *value;
    } else {
        api->clone(value, out);
    }
}
/* An owned value given back: one holder fewer in place, or, the last
 * holder's and any other kind, through the runtime. */
static inline void zgx_release(const zgx_api *api, zgx_value *value) {
    if (value->kind <= ZGX_NUM) return;
    if (zgx_counted(value)) {
        size_t *count = (size_t *)((uint8_t *)value->held.at - ZGX_COUNT_BEFORE);
        if (*count > 1) {
            *count -= 1;
            return;
        }
    }
    api->release(value);
}

#ifdef __cplusplus
}
#endif

#endif /* ZGX_MODULE_H */
