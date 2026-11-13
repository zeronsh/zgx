// What a gate scene allocates (gate.py preloads this beside same_run.so in
// a run of its own, natively, never under valgrind and never into anything
// shipped): every call to glibc's allocator, counted as it passes through.
// Each binary's allocator ends in glibc's (Rust's `System`, and the benches'
// own counting allocators on top of it), so one library counts every scene
// alike without changing a binary, and the counted runs' instructions are
// untouched: those runs don't load it.
//
// At exit it writes `allocations reallocations frees bytes peak` to the
// file `ZGX_ALLOC_COUNT_OUT` names: calls to malloc, calloc and the aligned
// allocators; calls to realloc; calls to free; bytes asked for (realloc's
// new size each time); and the most bytes held at once, as glibc measures
// a block (`malloc_usable_size`). A program whose allocator bypasses glibc
// (mimalloc) shows next to nothing here, which gate.py reports as such.
//
// cc -O2 -shared -fPIC -o count_allocs.so count_allocs.c
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <malloc.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

extern void *__libc_malloc(size_t size);
extern void *__libc_calloc(size_t count, size_t size);
extern void *__libc_realloc(void *pointer, size_t size);
extern void *__libc_memalign(size_t alignment, size_t size);
extern void __libc_free(void *pointer);

static uint64_t allocations, reallocations, frees, bytes;
static int64_t live, peak;

static void held(int64_t change) {
    int64_t now = __atomic_add_fetch(&live, change, __ATOMIC_RELAXED);
    int64_t high = __atomic_load_n(&peak, __ATOMIC_RELAXED);
    while (now > high &&
           !__atomic_compare_exchange_n(&peak, &high, now, 1, __ATOMIC_RELAXED, __ATOMIC_RELAXED)) {
    }
}

static void *counted(void *pointer, size_t size) {
    if (pointer) {
        __atomic_add_fetch(&allocations, 1, __ATOMIC_RELAXED);
        __atomic_add_fetch(&bytes, size, __ATOMIC_RELAXED);
        held((int64_t)malloc_usable_size(pointer));
    }
    return pointer;
}

void *malloc(size_t size) { return counted(__libc_malloc(size), size); }

void *calloc(size_t count, size_t size) { return counted(__libc_calloc(count, size), count * size); }

void *memalign(size_t alignment, size_t size) { return counted(__libc_memalign(alignment, size), size); }

void *aligned_alloc(size_t alignment, size_t size) { return memalign(alignment, size); }

int posix_memalign(void **out, size_t alignment, size_t size) {
    if (alignment < sizeof(void *) || (alignment & (alignment - 1)) != 0) {
        return EINVAL;
    }
    void *pointer = memalign(alignment, size);
    if (!pointer && size) {
        return ENOMEM;
    }
    *out = pointer;
    return 0;
}

void free(void *pointer) {
    if (pointer) {
        __atomic_add_fetch(&frees, 1, __ATOMIC_RELAXED);
        held(-(int64_t)malloc_usable_size(pointer));
    }
    __libc_free(pointer);
}

void *realloc(void *pointer, size_t size) {
    if (!pointer) {
        return malloc(size);
    }
    int64_t was = (int64_t)malloc_usable_size(pointer);
    void *moved = __libc_realloc(pointer, size);
    if (moved || size == 0) {
        __atomic_add_fetch(&reallocations, 1, __ATOMIC_RELAXED);
        __atomic_add_fetch(&bytes, size, __ATOMIC_RELAXED);
        held((moved ? (int64_t)malloc_usable_size(moved) : 0) - was);
    }
    return moved;
}

__attribute__((destructor)) static void report(void) {
    const char *path = getenv("ZGX_ALLOC_COUNT_OUT");
    if (!path) {
        return;
    }
    // Read before formatting: snprintf allocates nothing, but nothing after
    // this line is counted either way.
    unsigned long long counts[5] = {
        __atomic_load_n(&allocations, __ATOMIC_RELAXED), __atomic_load_n(&reallocations, __ATOMIC_RELAXED),
        __atomic_load_n(&frees, __ATOMIC_RELAXED),       __atomic_load_n(&bytes, __ATOMIC_RELAXED),
        (unsigned long long)__atomic_load_n(&peak, __ATOMIC_RELAXED),
    };
    char line[160];
    int length = snprintf(line, sizeof line, "%llu %llu %llu %llu %llu\n", counts[0], counts[1], counts[2],
                          counts[3], counts[4]);
    int file = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (file >= 0) {
        ssize_t written = write(file, line, (size_t)length);
        (void)written;
        close(file);
    }
}
