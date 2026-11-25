// One block kept from before a scene starts (gate.py preloads this beside
// same_run.so, in the counts of a base it does alone, to see how far a scene
// moves by where the heap puts things and nothing else): `ZGX_GATE_SHIFT`
// bytes, always four digits, so nothing but the block differs from one shift
// to another. The same library for every shift, and nothing of it in a
// binary.
//
// cc -O2 -shared -fPIC -o heap_shift.so heap_shift.c
#include <stdlib.h>
#include <string.h>

static void *volatile kept;

__attribute__((constructor)) static void keep(void) {
    const char *asked = getenv("ZGX_GATE_SHIFT");
    size_t size = asked ? strtoul(asked, NULL, 10) : 0;
    if (size) {
        kept = malloc(size);
        if (kept) {
            memset(kept, 1, size);
        }
    }
}
