# Planar against interleaved buffers

The two loops every voice and bus run: a stereo voice into its bus with settled gains, and a bus into its parent, for 64 voices over 10 s in 128-frame blocks. Each is written once over planar buffers (zgx-audio's: a row per channel) and once over interleaved ones, counted in instructions on this thread (macOS, `thread_selfcounts`), least of 5 interleaved runs.

```sh
cd comparisons/audio/layout && cargo run --release --locked
```

Measured on arm64 (Nov 10 2026): a voice into its bus, planar 3.85 against interleaved 4.02 instructions a frame; a bus into its parent, 2.65 against 2.13. A tie, so buses stay planar, which is simpler for mono sources and per-channel effects.
