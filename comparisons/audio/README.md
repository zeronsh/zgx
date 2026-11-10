# zgx-audio against kira and miniaudio

One scene in three mixers, rendered offline, counted in instructions retired on the mixing thread for each voice and each frame. `python3 comparisons/audio/run.py` runs zgx-audio's and miniaudio's arms on macOS in about two minutes. It fetches `miniaudio.h` at its tag and checks its hash, builds both harnesses, and prints their lines below.

**The kira arm isn't in this repository** until a licence decision on one of kira's dependencies (triple_buffer, under MPL-2.0; everything else kira pulls in is MIT or Apache-2.0). Its numbers below were measured with the same harness and the same method, plus kira 0.10.8 through a `Backend` that hands over its `Renderer`, and were first reported in #1942.

Measured Nov 10 2026 on an Apple M4 Pro (macOS 27.0.1, rustc 1.99.1, Apple clang 17.0.0) at zgx-audio as of 91873a7b3. **arm64 only:** x86 hasn't been counted.

## The scene

64 voices, each looping its own second of stereo noise (64 different sounds, so no two voices read the same memory or add up to a louder copy of one), over three buses: kira's sub-tracks, miniaudio's sound groups. Gain is 0.5/64 each, and pans are spread from -1 to 1 unless an arm says centred. Each arm renders 10 s in blocks (512 frames unless it says otherwise) and takes the least of 5 runs, interleaved with a control arm of no voices, which is subtracted. Every arm checks that each voice is still playing at the end.

- **zgx-audio**: `Audio::offline`, limiter off unless the arm says on.
- **kira 0.10.8** (`default-features = false`; not in the repository, see above): a `Backend` that hands over kira's `Renderer`, so its output can be kept. `internal_buffer_size` is the arm's block.
- **miniaudio 0.11.25**: `ma_engine` with `noDevice`, sounds from `ma_audio_buffer_ref` data sources with `MA_SOUND_FLAG_NO_SPATIALIZATION` (and `NO_PITCH` at 48 kHz), read with `ma_engine_read_pcm_frames`. Built with `clang -O3`.

## The same work

Each engine's 10 s output was kept and compared with zgx-audio's, with limiters off:

| | RMS L / R | against zgx-audio |
| --- | --- | --- |
| 48 kHz, centred: kira | 0.018032 / 0.018043 | correlation 1.0000000, largest difference 2.2e-8 |
| 48 kHz, centred: miniaudio | 0.018032 / 0.018043 | 1 frame later; correlation 1.0000000, largest difference 1.5e-8 |
| 44.1 kHz, centred: kira against cubic | 0.016258 / 0.016300 | correlation 1.0000000, largest difference 3.3e-6 |
| 44.1 kHz, centred: miniaudio against linear | 0.014701 / 0.014743 | RMS within 0.05%; correlation 0.990 at the best whole-frame lag (its resampler's phase is fractional) |

Spread, the pan laws differ (kira's near side rises to +3 dB, miniaudio's far side falls in straight lines, zgx-audio's near side stays at 1), and the correlation is 0.99.

## Instructions a voice a frame

| arm | zgx-audio | kira | miniaudio |
| --- | --- | --- | --- |
| stereo 48 kHz, settled | 6.84 | 206.5 | 16.3 |
| … blocks of 128 | 6.87 | 208.1 | 21.0 |
| … blocks of 1024 | 6.83 | 206.2 | 16.1 |
| … all pans centred | 6.83 | 190.7 | 11.2 |
| … with zgx-audio's limiter | 7.07 | — | — |
| stereo 44.1 kHz, cubic (kira's interpolator: the same Catmull-Rom) | 54.6 | 202.0 | — |
| stereo 44.1 kHz, linear (miniaudio's engine: linear, no low-pass) | 26.4 | — | 115.7 |
| gain ramping on every voice | 9.82 | 206.6 | 49.3 |
| gain and pan ramping | 15.9 | 207.0 | — (no pan ramp) |
| gain, pan and pitch ramping | 89.9 | 207.6 | — (no pitch ramp) |
| one voice | 10.3 | 206.8 | 94.1 |
| control, no voices (a frame, not a voice) | 5.10 | 148.8 | 7.13 |

zgx-audio's 6.84 by stage, from arms that remove one at a time: 3.03 reading the sound (voices at gain 0, read but not mixed), 3.63 gain and pan into the bus (`master` minus `silent`), 0.17 the three buses into master (`buses` minus `master`), and 0.24 the limiter.

## Effects against miniaudio's

`python3 comparisons/audio/run.py --effects` runs each stock effect against miniaudio's that does the same, on the same noise. One instance of each runs on its own looping second for the outputs; past the first 0.1 s (a zgx-audio voice fades in over 1 ms) they must agree to 90 dB under the signal. The cost arms run 16 instances, each on its own bus and looping noise, least of 5 runs, a control without the effects subtracted. zgx-audio's figures include its effect plumbing, about 6 instructions a frame: interleaving the bus's block in and out, its parameters, and keeping its output finite.

Measured Nov 10 2026 on an Apple M1 Max (macOS 26.0.1, rustc 1.99.1, Apple clang 17.0.0).

| effect | output against miniaudio's | zgx-audio | miniaudio |
| --- | --- | --- | --- |
| low-pass, 12 dB (`ma_lpf2`, Q 0.707) | 117.4 dB under | 27.70 | 46.61 |
| low-pass, 24 dB (`ma_lpf`, order 4: Butterworth sections) | 121.3 dB under | 42.76 | 133.62 |
| high-pass, 12 dB (`ma_hpf2`) | 101.3 dB under | 34.74 | 46.54 |
| peaking band, +6 dB, Q 1 (`ma_peak2`) | 117.2 dB under | 32.04 | 46.52 |
| low-pass, its cutoff moving every frame | — (miniaudio has no per-frame modulation) | 93.02 | — |
| delay, 0.25 s, feedback 0.5 (`ma_delay`, its comb) | identical, bit for bit | 12.50 | 41.63 |
| reverb, 8-line feedback delay network, size 1, decay 2 s | — (miniaudio has no reverb) | 103.83 | — |

## What isn't the same

- kira evaluates every parameter per frame in decibels (two `powf` a frame) and its position in f64, whether or not anything moves. That is most of its flat ~205.
- miniaudio has no pan or pitch ramps, and its control arm includes its node graph's fixed cost per block.
- Instructions, not time: missing the cache costs time at the same count.
- None of the three filters the images a resampler makes; a windowed sinc would, at a higher cost.
