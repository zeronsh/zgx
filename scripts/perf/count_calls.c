// What a gate scene asks of the C library (gate.py preloads this beside
// same_run.so in a native run of a cell it can't explain by zgx's own work,
// never under valgrind and never into anything shipped): calls and bytes of
// memcpy, memmove, memset, memcmp (and bcmp), and of malloc, calloc and the
// aligned allocators, and realloc (the sizes asked for). A heap that lies
// elsewhere costs instructions in these with the same calls and bytes; a
// change's own work makes more of them, or longer ones.
//
// Each function does its own work here rather than glibc's, so the counts
// are of what the program asks, not of what glibc does inside (its calls to
// itself don't pass through here). Counts only: no time is compared.
//
// At exit it writes `name calls bytes` a line to the file `ZGX_CALLS_OUT`
// names. cc -O2 -fno-builtin -fno-tree-loop-distribute-patterns -shared
// -fPIC -o count_calls.so count_calls.c
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

extern void *__libc_malloc(size_t size);
extern void *__libc_calloc(size_t count, size_t size);
extern void *__libc_realloc(void *pointer, size_t size);
extern void *__libc_memalign(size_t alignment, size_t size);

enum { MEMCPY, MEMMOVE, MEMSET, MEMCMP, MALLOC, REALLOC, KINDS };
static const char *const names[KINDS] = {"memcpy", "memmove", "memset", "memcmp", "malloc", "realloc"};
static uint64_t calls[KINDS], bytes[KINDS];

static void note(int kind, size_t size) {
    __atomic_add_fetch(&calls[kind], 1, __ATOMIC_RELAXED);
    __atomic_add_fetch(&bytes[kind], size, __ATOMIC_RELAXED);
}

static void copy(void *to, const void *from, size_t size) {
#if defined(__x86_64__)
    __asm__ volatile("rep movsb" : "+D"(to), "+S"(from), "+c"(size) : : "memory");
#else
    unsigned char *a = to;
    const unsigned char *b = from;
    while (size--) {
        *a++ = *b++;
    }
#endif
}

void *memcpy(void *to, const void *from, size_t size) {
    note(MEMCPY, size);
    copy(to, from, size);
    return to;
}

void *memmove(void *to, const void *from, size_t size) {
    note(MEMMOVE, size);
    unsigned char *a = to;
    const unsigned char *b = from;
    if (a <= b || a >= b + size) {
        copy(to, from, size);
    } else {
        while (size--) {
            a[size] = b[size];
        }
    }
    return to;
}

void *memset(void *to, int byte, size_t size) {
    note(MEMSET, size);
#if defined(__x86_64__)
    void *at = to;
    __asm__ volatile("rep stosb" : "+D"(at), "+c"(size) : "a"(byte) : "memory");
#else
    for (unsigned char *a = to; size--;) {
        *a++ = (unsigned char)byte;
    }
#endif
    return to;
}

static int compare(const void *a, const void *b, size_t size) {
    const unsigned char *x = a, *y = b;
    for (size_t at = 0; at < size; at++) {
        if (x[at] != y[at]) {
            return x[at] < y[at] ? -1 : 1;
        }
    }
    return 0;
}

int memcmp(const void *a, const void *b, size_t size) {
    note(MEMCMP, size);
    return compare(a, b, size);
}

int bcmp(const void *a, const void *b, size_t size) {
    note(MEMCMP, size);
    return compare(a, b, size);
}

void *malloc(size_t size) {
    note(MALLOC, size);
    return __libc_malloc(size);
}

void *calloc(size_t count, size_t size) {
    note(MALLOC, count * size);
    return __libc_calloc(count, size);
}

void *memalign(size_t alignment, size_t size) {
    note(MALLOC, size);
    return __libc_memalign(alignment, size);
}

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

void *realloc(void *pointer, size_t size) {
    if (!pointer) {
        return malloc(size);
    }
    note(REALLOC, size);
    return __libc_realloc(pointer, size);
}

__attribute__((destructor)) static void report(void) {
    const char *path = getenv("ZGX_CALLS_OUT");
    int file = path ? open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644) : -1;
    if (file < 0) {
        return;
    }
    uint64_t seen[KINDS][2];
    for (int kind = 0; kind < KINDS; kind++) {
        seen[kind][0] = __atomic_load_n(&calls[kind], __ATOMIC_RELAXED);
        seen[kind][1] = __atomic_load_n(&bytes[kind], __ATOMIC_RELAXED);
    }
    char line[96];
    for (int kind = 0; kind < KINDS; kind++) {
        int length = snprintf(line, sizeof line, "%s %llu %llu\n", names[kind], (unsigned long long)seen[kind][0],
                              (unsigned long long)seen[kind][1]);
        ssize_t written = write(file, line, (size_t)length);
        (void)written;
    }
    close(file);
}
