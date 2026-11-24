/* A zgx module in C, for tests/native_modules.rs: zgx_module.h alone, no
 * Rust. Its manifest's bytes are made by the test (manifest.h). */
#include "zgx_module.h"
#include "manifest.h"

/* The runtime's functions, kept as the module loads. */
static const zgx_api *api_of_module;
#define api api_of_module

static uint32_t failed(zgx_value *out, const char *why) {
    api->string((const uint8_t *)why, strlen(why), out);
    return ZGX_FAILED;
}

/* cwords.count(text): how many words a string has. */
static uint32_t count(void *user, const zgx_value *args, size_t len, zgx_value *out) {
    (void)user;
    (void)len;
    size_t bytes = 0;
    const char *text = zgx_str(&args[0], &bytes);
    if (!text) return failed(out, "argument 1 (text) must be a string");
    size_t words = 0;
    int inside = 0;
    for (size_t i = 0; i < bytes; i++) {
        int space = text[i] == ' ' || text[i] == '\n' || text[i] == '\t';
        if (!space && !inside) words++;
        inside = !space;
    }
    zgx_num(out, (double)words);
    return ZGX_DONE;
}

/* cwords.total(numbers, scale): a list's numbers summed, times `scale` if
 * it is given: items read in place, and a second argument. */
static uint32_t total(void *user, const zgx_value *args, size_t len, zgx_value *out) {
    (void)user;
    size_t n = 0;
    const zgx_value *items = zgx_items(&args[0], &n);
    if (!items) return failed(out, "argument 1 (numbers) must be a list");
    double sum = 0;
    for (size_t i = 0; i < n; i++)
        if (items[i].kind == ZGX_NUM) sum += items[i].held.number;
    if (len > 1 && args[1].kind == ZGX_NUM) sum *= args[1].held.number;
    zgx_num(out, sum);
    return ZGX_DONE;
}

/* cwords.describe(row): `{ id, n, kept }` of a map's `id` and `n` fields:
 * fields found in place, a string held once more, and a map made. */
static uint32_t describe(void *user, const zgx_value *args, size_t len, zgx_value *out) {
    (void)user;
    (void)len;
    const zgx_value *row = zgx_opened(api, &args[0]);
    const zgx_value *id = zgx_field_of(row, "id", 2);
    const zgx_value *n = zgx_field_of(row, "n", 1);
    if (!id || id->kind != ZGX_STR) return failed(out, "argument 1 (row) field `id` must be a string");
    zgx_bytes keys[3] = {{(const uint8_t *)"id", 2}, {(const uint8_t *)"n", 1}, {(const uint8_t *)"missing", 7}};
    zgx_value values[3];
    zgx_retain(api, id, &values[0]);
    zgx_num(&values[1], n && n->kind == ZGX_NUM ? n->held.number + 1 : -1);
    zgx_bool(&values[2], zgx_field_of(row, "zz", 2) == NULL);
    api->map(keys, values, 3, out);
    return ZGX_DONE;
}

/* cwords.keep(value), an action, and cwords.kept(): a value held between
 * calls in the module's state for the thread, and let go of. */
static void kept_init(uint8_t *state) { zgx_nil((zgx_value *)state); }
static void kept_free(uint8_t *state) { zgx_release(api, (zgx_value *)state); }
static uint32_t keep(void *user, const zgx_value *arg, zgx_value *out) {
    (void)out;
    zgx_value *kept = user;
    zgx_value was = *kept;
    zgx_retain(api, arg, kept);
    zgx_release(api, &was);
    return ZGX_DONE;
}
static uint32_t kept(void *user, const zgx_value *args, size_t len, zgx_value *out) {
    (void)args;
    (void)len;
    zgx_retain(api, (zgx_value *)user, out);
    return ZGX_DONE;
}
static void nothing(void *user) { (void)user; }

#define NAME(text) (const uint8_t *)(text), sizeof(text) - 1

#undef api
ZGX_MODULE_ENTRY(cwords) {
    if (api->abi != ZGX_MODULE_ABI || api->size < sizeof(zgx_api) ||
        registrar->size < sizeof(zgx_registrar))
        return ZGX_REFUSED_ABI;
    const zgx_registrar *r = registrar;
    api_of_module = api;
    r->function(r->host, NAME("cwords.count"), count, NULL, nothing);
    r->function(r->host, NAME("cwords.total"), total, NULL, nothing);
    r->function(r->host, NAME("cwords.describe"), describe, NULL, nothing);
    void *state = r->state(r->host, NAME("cwords kept"), sizeof(zgx_value), _Alignof(zgx_value),
                           kept_init, kept_free);
    r->action(r->host, NAME("cwords.keep"), keep, state, nothing);
    r->function(r->host, NAME("cwords.kept"), kept, state, nothing);
    return 0;
}
