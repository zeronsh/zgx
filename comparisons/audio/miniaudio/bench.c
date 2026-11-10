/* miniaudio 0.11.25 on zgx-audio's comparison scene: 64 voices looping a
   second of stereo noise (each its own, the same noise as the Rust harness)
   over three sound groups, no device, no spatialisation, read with
   ma_engine_read_pcm_frames in blocks; instructions on this thread.
   bench <voices> <rate> <block> <spread|center> <none|gain> [out.f32]
   Built by run.py: clang -O3 -std=c11. */
#define MINIAUDIO_IMPLEMENTATION
#define MA_NO_DEVICE_IO
#define MA_NO_DECODING
#define MA_NO_ENCODING
#define MA_NO_GENERATION
#include "miniaudio.h" /* fetched by run.py, pinned by hash */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern int thread_selfcounts(int kind, uint64_t *buffer, size_t size);
static uint64_t count(void) {
    uint64_t c[2] = {0, 0};
    if (thread_selfcounts(1, c, sizeof c) != 0) { fprintf(stderr, "no counts\n"); exit(1); }
    return c[0];
}

#define SECONDS 10
#define VOICES_MAX 64

static float *noise(uint32_t rate, uint64_t voice) {
    uint64_t seed = 0x2545f4914f6cdd1dULL ^ ((voice * 0x9e3779b97f4a7c15ULL) | 1);
    float *out = malloc(sizeof(float) * rate * 2);
    for (uint32_t i = 0; i < rate * 2; i++) {
        seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
        out[i] = (float)(seed >> 40) / 16777216.0f - 0.5f;
    }
    return out;
}

typedef struct { int voices; uint32_t rate; int block; int spread; int gain_ramp; } Scene;

static uint64_t run(Scene s, float **noises, float *keep) {
    ma_engine_config config = ma_engine_config_init();
    config.noDevice = MA_TRUE;
    config.channels = 2;
    config.sampleRate = 48000;
    config.listenerCount = 1;
    ma_engine engine;
    if (ma_engine_init(&config, &engine) != MA_SUCCESS) { fprintf(stderr, "engine\n"); exit(1); }
    ma_sound_group groups[3];
    for (int g = 0; g < 3; g++) ma_sound_group_init(&engine, MA_SOUND_FLAG_NO_SPATIALIZATION, NULL, &groups[g]);
    static ma_audio_buffer_ref refs[VOICES_MAX];
    static ma_sound sounds[VOICES_MAX];
    float gain = 0.5f / (float)(s.voices > 0 ? s.voices : 1);
    for (int i = 0; i < s.voices; i++) {
        ma_audio_buffer_ref_init(ma_format_f32, 2, noises[i % VOICES_MAX], s.rate, &refs[i]);
        refs[i].sampleRate = s.rate;
        ma_uint32 flags = MA_SOUND_FLAG_NO_SPATIALIZATION | (s.rate == 48000 ? MA_SOUND_FLAG_NO_PITCH : 0);
        if (ma_sound_init_from_data_source(&engine, &refs[i], flags, &groups[i % 3], &sounds[i]) != MA_SUCCESS) { fprintf(stderr, "sound\n"); exit(1); }
        ma_sound_set_looping(&sounds[i], MA_TRUE);
        ma_sound_set_volume(&sounds[i], gain);
        ma_sound_set_pan(&sounds[i], s.spread ? ((float)i / (float)s.voices) * 2.0f - 1.0f : 0.0f);
        ma_sound_start(&sounds[i]);
    }
    float *out = malloc(sizeof(float) * s.block * 2);
    int blocks = SECONDS * 48000 / s.block;
    ma_engine_read_pcm_frames(&engine, out, s.block, NULL);
    if (keep) memcpy(keep, out, sizeof(float) * s.block * 2);
    if (s.gain_ramp)
        for (int i = 0; i < s.voices; i++) ma_sound_set_fade_in_pcm_frames(&sounds[i], 1.0f, 0.5f, (SECONDS + 1) * 48000);
    uint64_t before = count();
    for (int b = 1; b < blocks; b++) {
        ma_uint64 read = 0;
        ma_engine_read_pcm_frames(&engine, out, s.block, &read);
        if (read != (ma_uint64)s.block) { fprintf(stderr, "short read\n"); exit(1); }
        if (keep) memcpy(keep + (size_t)b * s.block * 2, out, sizeof(float) * s.block * 2);
    }
    uint64_t instructions = count() - before;
    for (int i = 0; i < s.voices; i++) {
        if (!ma_sound_is_playing(&sounds[i])) { fprintf(stderr, "voice %d stopped\n", i); exit(1); }
        ma_sound_uninit(&sounds[i]);
    }
    for (int g = 0; g < 3; g++) ma_sound_group_uninit(&groups[g]);
    ma_engine_uninit(&engine);
    free(out);
    return instructions;
}

int main(int argc, char **argv) {
    Scene s = { 64, 48000, 512, 1, 0 };
    if (argc > 1) s.voices = atoi(argv[1]);
    if (argc > 2) s.rate = (uint32_t)atoi(argv[2]);
    if (argc > 3) s.block = atoi(argv[3]);
    if (argc > 4) s.spread = strcmp(argv[4], "spread") == 0;
    if (argc > 5) s.gain_ramp = strcmp(argv[5], "gain") == 0;
    float *noises[VOICES_MAX];
    for (int v = 0; v < VOICES_MAX; v++) noises[v] = noise(s.rate, (uint64_t)v);
    if (argc > 6) {
        size_t samples = (size_t)(SECONDS * 48000 / s.block) * s.block * 2;
        float *keep = calloc(samples, sizeof(float));
        run(s, noises, keep);
        FILE *f = fopen(argv[6], "wb");
        fwrite(keep, sizeof(float), samples, f);
        fclose(f);
        printf("wrote %s\n", argv[6]);
        return 0;
    }
    Scene control = s; control.voices = 0;
    uint64_t best_c = UINT64_MAX, best_a = UINT64_MAX;
    for (int r = 0; r < 5; r++) {
        uint64_t c = run(control, noises, NULL), a = run(s, noises, NULL);
        if (c < best_c) best_c = c;
        if (a < best_a) best_a = a;
    }
    double frames = (double)((SECONDS * 48000 / s.block) - 1) * s.block;
    printf("miniaudio voices %d rate %u block %d %s %s: %.2f a voice a frame (control %.3f a frame)\n", s.voices, s.rate, s.block,
           s.spread ? "spread" : "center", s.gain_ramp ? "gain-ramp" : "settled",
           (double)(best_a - best_c) / s.voices / frames, (double)best_c / frames);
    return 0;
}
