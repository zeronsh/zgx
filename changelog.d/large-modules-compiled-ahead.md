---
section: performance
---
- **A data module of megabytes is compiled while the window is made** (zgx, zgx-script): `compile_ahead` passed over any module larger than 1 MB, so a feed kept in a Luau string compiled on the main thread at its `require`. It now takes files up to its 16 MB budget. What it compiled comes back to the worker as the load lets go of it, and the worker frees it and rests its thread as the decoding thread does (`set_decoder_rest`). mimalloc keeps pages that another thread freed for the thread that made them, and a window's peak footprint was otherwise up to 15 MB higher. A 6.8 MB feed's `require` takes 2.3-2.5 ms, not 18-20, the heavy list's first frame comes 7-19 ms sooner, and its peak footprint is 7 MB lower (M5 Max).
