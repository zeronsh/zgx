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
/* Frees a registration's, a job's or a timer's `user` when it is done. */
typedef void (*zgx_free)(void *user);
/* A job zgx_api.post runs on the interface's thread: `host` is the host
 * handle it was posted to, lent. It runs once and frees its own `user` as it
 * does; the post's `free` frees one that never runs (its host went first). */
typedef void (*zgx_job)(void *user, void *host);
/* A timer's call (zgx_api.after), on the interface's thread, `host` lent: a
 * repeating timer goes on while it returns not 0. */
typedef uint32_t (*zgx_fire)(void *user, void *host);
/* A module's async action (`ui.call(name, arg)`): `arg` lent. It returns at
 * once; the module settles `promise` later (zgx_api.settle) or lets go of it
 * (zgx_api.promise_release), exactly one of the two. */
typedef void (*zgx_async_action)(void *user, const zgx_value *arg, void *promise);
/* Where a line highlighter hands each span: bytes from 1, both ends in,
 * and its kind's number. */
typedef void (*zgx_push)(void *sink, uint32_t from, uint32_t to, uint32_t kind);
/* A line highlighter (zgx_registrar.highlighter; unstable): `line`'s spans,
 * each to `push` with `sink`, from what the line before left (0: nothing
 * open); returns what this line leaves. */
typedef uint32_t (*zgx_highlight)(void *user, const uint8_t *line, size_t len, uint32_t state,
                                  void *sink, zgx_push push);
/* What a highlighter returns when it failed: the runtime turns it off for
 * that textarea and says so. Every state is below ZGX_HIGHLIGHT_STATES. */
#define ZGX_HIGHLIGHT_FAILED 0xFFFFFFFFu
#define ZGX_HIGHLIGHT_STATES 0x7FFFFFFFu
/* A shared state's observer (zgx_api.shared_observe): `value` is what it
 * holds now, lent; `host` the state's host handle, lent. */
typedef void (*zgx_observe)(void *user, void *host, const zgx_value *value);

/* What zgx_api.post returns: the job will run, or the host is gone. */
#define ZGX_POSTED 1u
#define ZGX_GONE 0u
/* What shared_set and shared_remove return once the state's host is gone,
 * nothing written: not a count, so told from one. (The writes that return
 * ZGX_DONE or ZGX_FAILED fail then, saying so.) */
#define ZGX_HOST_GONE 0xFFFFFFFFu

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
    /* The host's own reach, on the interface's thread unless said. */
    /* The host handle `host` once more, to keep: any thread. */
    void *(*host_retain)(void *host);
    /* Lets go of a host handle: any thread. */
    void (*host_release)(void *host);
    /* Runs `job` on the host's thread at its next turn, in one reactive
     * batch: any thread. ZGX_POSTED, or ZGX_GONE once the host is (`free`
     * then called at once, on the caller's thread). */
    uint32_t (*post)(void *host, zgx_job job, void *user, zgx_free free);
    /* Calls `fire` after `ms` milliseconds on zgx's clock, then every `ms`
     * while `repeat` is not 0 and `fire` returns not 0: a timer, to let go
     * with timer_release. `user` is freed as it ends. NULL once the host is
     * gone (`free` called at once). */
    void *(*after)(void *host, double ms, uint32_t repeat, zgx_fire fire, void *user,
                   zgx_free free);
    /* Lets go of a timer, stopping it if `stop` is not 0. */
    void (*timer_release)(void *timer, uint32_t stop);
    /* The module's state for this thread (zgx_registrar.state's), asked by
     * a host handle: from a job or a timer's call. */
    uint8_t *(*state)(void *host, const uint8_t *name, size_t len, size_t size, size_t align,
                      void (*init)(uint8_t *state), void (*free)(uint8_t *state));
    /* The host's shared state `key` (scripts' `ui.shared(key)`), made with
     * `initial` (taken) if there is none: a handle, NULL once the host is
     * gone. */
    void *(*shared)(void *host, const uint8_t *key, size_t len, zgx_value *initial);
    /* What it holds, owned; `nil` once its host is gone. */
    void (*shared_get)(const void *shared, zgx_value *out);
    /* Sets it to `value` (taken), keeping the parts that are equal: 1 if
     * anything changed, 0 if not; ZGX_HOST_GONE once its host is gone
     * (nothing set). */
    uint32_t (*shared_set)(const void *shared, zgx_value *value);
    /* Adds text to the string it holds: ZGX_DONE, or ZGX_FAILED with why in
     * `out` (it holds something else, or its host is gone). */
    uint32_t (*shared_append)(const void *shared, const uint8_t *text, size_t len,
                              zgx_value *out);
    /* Calls `call` each time it changes, from now: a subscription, NULL
     * once its host is gone. */
    void *(*shared_observe)(const void *shared, zgx_observe call, void *user, zgx_free free);
    /* Lets go of a subscription. */
    void (*shared_unobserve)(void *subscription);
    /* Lets go of a shared state's handle. */
    void (*shared_release)(void *shared);
    /* Settles an async action's promise: with `value` (taken) if `ok` is not
     * 0, else rejected with `value`, a string of why. Once, on the
     * interface's thread; the promise is let go of with it. */
    void (*settle)(void *promise, uint32_t ok, zgx_value *value);
    /* Lets go of a promise unsettled: it rejects, saying the module dropped
     * it. Any thread. */
    void (*promise_release)(void *promise);
    /* A handle to the host of the script waiting on `promise`, to keep: any
     * thread. */
    void *(*promise_host)(const void *promise);
    /* Writes `rows` (a list, taken) into the list a shared state holds, by
     * each row's string field `key`: a row whose key the list has replaces
     * the first row with that key, in its place; the rest are added at the
     * end in order; of two given with one key the last is written. One pass
     * over the list, the rows not given kept as they are; a state holding
     * nothing (`nil`) yet becomes a list of them. ZGX_DONE, or ZGX_FAILED
     * with why in `out` (a row with no string at `key`, `rows` not a list,
     * the state holding something not a list, or its host gone), nothing
     * written. */
    uint32_t (*shared_upsert)(const void *shared, const uint8_t *key, size_t key_len,
                              zgx_value *rows, zgx_value *out);
    /* Removes the rows of the list a shared state holds whose string field
     * `key` is one of `keys` (`len` of them, lent): every such row. 1 if
     * any was, 0 if none; ZGX_HOST_GONE once its host is gone. */
    uint32_t (*shared_remove)(const void *shared, const uint8_t *key, size_t key_len,
                              const zgx_bytes *keys, size_t len);
    /* Images by name. A picture is RGBA8: four bytes a pixel (red, green,
     * blue, alpha; straight, not premultiplied), rows top to bottom, tightly
     * packed (a row's stride is width * 4), so width * height * 4 bytes. */
    /* A buffer of `len` bytes, all 0, for an image's pixels: a handle, its
     * bytes at `*data`. Allocated as what the image will share, so setting
     * it copies nothing. Any thread: the module's to write, from one thread
     * at a time, until it gives it to image_set or image_drop. NULL if
     * `len` is 0 or more than SIZE_MAX / 4. */
    void *(*image_buffer)(size_t len, uint8_t **data);
    /* Sets the image `host:<name>` (scripts' `ui.image { src = ... }`) to a
     * `width` by `height` picture, `buffer`'s bytes (taken, set or not):
     * its length must be width * height * 4. On the interface's thread;
     * `name` is `<module>.<name>`, and what a module sets goes with its
     * host, or as the module is loaded into it again. ZGX_DONE, or
     * ZGX_FAILED with why in `out`, nothing set: another length, a size of
     * 0, a name not so, or its host gone (as post says ZGX_GONE; the buffer
     * freed). On another thread, ZGX_FAILED and `out` nil (no value is made
     * off the interface's thread). */
    uint32_t (*image_set)(void *host, const uint8_t *name, size_t len, uint32_t width,
                          uint32_t height, void *buffer, zgx_value *out);
    /* Lets go of a buffer not set: any thread. */
    void (*image_drop)(void *buffer);
    /* Removes the image `host:<name>` a module set: not 0 if there was one.
     * On the interface's thread (on another, nothing). */
    uint32_t (*image_remove)(void *host, const uint8_t *name, size_t len);
    /* shared_upsert with a place for each row: `places` (a list, taken) as
     * long as `rows`, each nil (where its key's row is, or at the end if the
     * list has none), a string (just before the row with that key: the
     * list's first of it, or one given, wherever that goes) or true (at the
     * end). A row the list has moves to its place; rows placed before one
     * key go in the order given; one walk of the list. ZGX_DONE, or
     * ZGX_FAILED with why in `out`, nothing written: a row placed before
     * itself, before a key neither the list nor the rows have, rows placed
     * before one another (a before b before a), what shared_upsert refuses,
     * or its host gone. */
    uint32_t (*shared_upsert_at)(const void *shared, const uint8_t *key, size_t key_len,
                                 zgx_value *rows, zgx_value *places, zgx_value *out);
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
    /* A handle to the host being loaded into, kept past the entry: what
     * zgx_api.post and zgx_api.after take. Let go with
     * zgx_api.host_release. */
    void *(*host_ref)(void *host);
    /* Registers an async action scripts call with `ui.call(name, arg)`. */
    void (*async_action)(void *host, const uint8_t *name, size_t len, zgx_async_action call,
                         void *user, zgx_free free);
    /* Registers a line highlighter a textarea calls itself
     * (`ui.highlighter(name, colours)`), its spans' kinds named by `kinds`,
     * `count` of them. Unstable: present where `size` reaches it. */
    void (*highlighter)(void *host, const uint8_t *name, size_t len, const zgx_bytes *kinds,
                        size_t count, zgx_highlight call, void *user, zgx_free free);
} zgx_registrar;

/* A module's entry, called once per host it is loaded into. Returns 0, or
 * a reason it can't load (ZGX_REFUSED_ABI: the runtime speaks another
 * interface, or has a shorter table than the module needs). */
#define ZGX_REFUSED_ABI 1u
/* (A Rust module's setup panicked: nothing of it is registered.) */
#define ZGX_SETUP_PANICKED 2u
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
