---
section: added
---
- **An engine the app pulls blocks from** (zgx-audio): `Audio::pulled(config, wake)` gives an engine with no thread of its own, and `Pull::fill(out)` mixes each block asked for. It's what a browser's audio worklet drives. Streams decode inline as blocks are filled. Nothing is asked for before the first sound, which calls `wake`, or after the quiet hold, when `fill` says to stop. It sounds as an offline engine does, sample for sample. zgx-audio builds for wasm32.
