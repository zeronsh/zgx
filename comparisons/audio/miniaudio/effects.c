/* miniaudio's arm of the effect comparison: each instance of an effect
 * runs on its own looping second of stereo noise (the same noise as
 * zgx-audio's effect_cost), 10 s in blocks of 512 frames; the least of 5
 * runs, interleaved with a control that only copies the noise, is
 * subtracted, and instructions an instance a frame are printed.
 *
 *   effects <lpf2|lpf4|hpf2|peak2|delay> [instances] [out.f32]
 *
 * With a path, one instance's 10 s of output is written there, for
 * comparing with zgx-audio's. */
#define MINIAUDIO_IMPLEMENTATION
#define MA_NO_DEVICE_IO
#define MA_NO_DECODING
#define MA_NO_ENCODING
#define MA_NO_GENERATION
#define MA_NO_RESOURCE_MANAGER
#define MA_NO_ENGINE
#define MA_NO_NODE_GRAPH
#include "miniaudio.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define RATE 48000
#define BLOCK 512
#define SECONDS 10

extern int thread_selfcounts(int kind, unsigned long long* buffer, size_t size);
static unsigned long long count(void) {
    unsigned long long c[2];
    if (thread_selfcounts(1, c, sizeof c) != 0) { fprintf(stderr, "no counts\n"); exit(1); }
    return c[0];
}

static float* noise(unsigned long long v) {
    unsigned long long seed = 0x2545f4914f6cdd1dULL ^ ((v * 0x9e3779b97f4a7c15ULL) | 1);
    float* out = malloc(sizeof(float) * RATE * 2);
    for (int i = 0; i < RATE * 2; i++) {
        seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
        out[i] = (float)(seed >> 40) / (float)(1ULL << 24) - 0.5f;
    }
    return out;
}

typedef struct { int kind; ma_lpf2 lpf2; ma_lpf lpf; ma_hpf2 hpf2; ma_peak2 peak2; ma_delay delay; } Effect;

enum { LPF2, LPF4, HPF2, PEAK2, DELAY };

static void make(Effect* e, int kind) {
    e->kind = kind;
    ma_result r = MA_SUCCESS;
    switch (kind) {
    case LPF2: { ma_lpf2_config c = ma_lpf2_config_init(ma_format_f32, 2, RATE, 2000, 0.7071067811865476); r = ma_lpf2_init(&c, NULL, &e->lpf2); break; }
    case LPF4: { ma_lpf_config c = ma_lpf_config_init(ma_format_f32, 2, RATE, 2000, 4); r = ma_lpf_init(&c, NULL, &e->lpf); break; }
    case HPF2: { ma_hpf2_config c = ma_hpf2_config_init(ma_format_f32, 2, RATE, 200, 0.7071067811865476); r = ma_hpf2_init(&c, NULL, &e->hpf2); break; }
    case PEAK2: { ma_peak2_config c = ma_peak2_config_init(ma_format_f32, 2, RATE, 6, 1, 1000); r = ma_peak2_init(&c, NULL, &e->peak2); break; }
    case DELAY: { ma_delay_config c = ma_delay_config_init(2, RATE, RATE / 4, 0.5f); r = ma_delay_init(&c, NULL, &e->delay); break; }
    }
    if (r != MA_SUCCESS) { fprintf(stderr, "init %d failed\n", kind); exit(1); }
}

static void run(Effect* e, float* out, const float* in, int frames) {
    switch (e->kind) {
    case LPF2: ma_lpf2_process_pcm_frames(&e->lpf2, out, in, frames); break;
    case LPF4: ma_lpf_process_pcm_frames(&e->lpf, out, in, frames); break;
    case HPF2: ma_hpf2_process_pcm_frames(&e->hpf2, out, in, frames); break;
    case PEAK2: ma_peak2_process_pcm_frames(&e->peak2, out, in, frames); break;
    case DELAY: ma_delay_process_pcm_frames(&e->delay, out, in, frames); break;
    }
}

/* Instructions for 10 s of `instances` instances (or, with kind < 0, the
 * copies alone); with `keep`, instance 0's output. */
static unsigned long long arm(int kind, int instances, float** sounds, float* keep) {
    Effect* effects = calloc(instances, sizeof(Effect));
    for (int i = 0; kind >= 0 && i < instances; i++) make(&effects[i], kind);
    float in[BLOCK * 2], out[BLOCK * 2];
    float checksum = 0;
    unsigned long long before = count();
    for (int b = 0; b < SECONDS * RATE / BLOCK; b++) {
        int at = (b * BLOCK) % RATE;
        for (int i = 0; i < instances; i++) {
            /* The noise loops: a block may wrap round its end. */
            for (int f = 0; f < BLOCK; f++) {
                int src = ((at + f) % RATE) * 2;
                in[f * 2] = sounds[i][src];
                in[f * 2 + 1] = sounds[i][src + 1];
            }
            if (kind >= 0) run(&effects[i], out, in, BLOCK); else memcpy(out, in, sizeof out);
            checksum += out[0];
            if (keep && i == 0) memcpy(keep + (size_t)b * BLOCK * 2, out, sizeof out);
        }
    }
    unsigned long long after = count();
    if (checksum == 0) { fprintf(stderr, "no sound came out\n"); exit(1); }
    free(effects);
    return after - before;
}

int main(int argc, char** argv) {
    const char* names[] = {"lpf2", "lpf4", "hpf2", "peak2", "delay"};
    int kind = -1;
    for (int k = 0; k < 5; k++) if (argc > 1 && strcmp(argv[1], names[k]) == 0) kind = k;
    if (kind < 0) { fprintf(stderr, "effects <lpf2|lpf4|hpf2|peak2|delay> [instances] [out.f32]\n"); return 2; }
    int instances = argc > 2 ? atoi(argv[2]) : 16;
    float* sounds[16];
    for (int i = 0; i < 16; i++) sounds[i] = noise(i);
    float** per = malloc(sizeof(float*) * instances);
    for (int i = 0; i < instances; i++) per[i] = sounds[i % 16];
    if (argc > 3) {
        size_t n = (size_t)(SECONDS * RATE / BLOCK) * BLOCK * 2;
        float* keep = malloc(sizeof(float) * n);
        arm(kind, 1, per, keep);
        FILE* f = fopen(argv[3], "wb");
        fwrite(keep, sizeof(float), n, f);
        fclose(f);
    }
    unsigned long long control = ~0ULL, best = ~0ULL;
    for (int r = 0; r < 5; r++) {
        unsigned long long c = arm(-1, instances, per, NULL), a = arm(kind, instances, per, NULL);
        if (c < control) control = c;
        if (a < best) best = a;
    }
    double frames = (double)(SECONDS * RATE / BLOCK) * BLOCK;
    printf("miniaudio %s x %d: %.2f instructions an instance a frame\n", names[kind], instances,
           (double)(best - control) / instances / frames);
    return 0;
}
