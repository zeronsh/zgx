/* A C module whose line highlighter pushes what no runtime should trust, for
 * tests/native_highlighter.rs: spans from 0, ending before they start, past
 * the line, of kinds it hasn't, out of order, overlapping, cutting a
 * character, and a million of them. */
#include "zgx_module.h"
#include "manifest.h"

static void nothing(void *user) { (void)user; }
#define NAME(text) (const uint8_t *)(text), sizeof(text) - 1

static uint32_t hostile(void *user, const uint8_t *line, size_t len, uint32_t state, void *sink,
                        zgx_push push) {
    (void)user;
    if (len >= 2 && line[1] == 0xC3) {
        /* "aé…": a span of the first of é's two bytes alone. */
        push(sink, 2, 2, 0);
        return state;
    }
    if (len >= 1 && line[0] == 'x') {
        /* Past the line, alone. */
        push(sink, 2, 4000000000u, 1);
        return state;
    }
    push(sink, 0, 2, 0);          /* from 0 */
    push(sink, 5, 3, 0);          /* ends before it starts */
    push(sink, 3, 4000000000u, 1); /* past the line */
    push(sink, 1, 1, 99);         /* a kind it hasn't */
    push(sink, 2, 2, 0);          /* out of order */
    push(sink, 1, 3, 1);          /* over the two before it */
    for (uint32_t k = 0; k < 1000000; k++) push(sink, (uint32_t)len, (uint32_t)len, 0);
    return state;
}

ZGX_MODULE_ENTRY(hostile) {
    (void)api;
    if (registrar->size < sizeof(zgx_registrar)) return ZGX_REFUSED_ABI;
    const zgx_registrar *r = registrar;
    zgx_bytes kinds[2] = {{(const uint8_t *)"one", 3}, {(const uint8_t *)"two", 3}};
    r->highlighter(r->host, NAME("hostile.spans"), kinds, 2, hostile, NULL, nothing);
    return 0;
}
