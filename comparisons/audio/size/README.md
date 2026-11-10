# What zgx-audio adds to a binary

One small program built four ways (release, fat LTO, one codegen unit, stripped), sizes compared:

```sh
cd comparisons/audio/size
for f in "" "--features zgx-audio" "--features vorbis" "--features device"; do
  cargo build --release --locked --bin size $f && ls -l target/release/size
done
```

| | bytes (arm64, Nov 10 2026) | added |
| --- | --- | --- |
| std alone | 319,280 | |
| the core: the mixer, WAV, a sound played and a WAV streamed | 469,312 | +146 KiB |
| with `vorbis` (lewton): an Ogg loaded and streamed | 618,512 | +146 KiB, the decoder alone |
| with `device` (cpal; measured before streams existed: 570,240 against 419,632 for the core then) | | +147 KiB |

Each arm does the work it pays for, so nothing is left out as dead code. The design's budget is the core under 300 KiB, with each decoder measured on its own.
