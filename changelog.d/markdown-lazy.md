---
section: added
---
- **A long reply landing whole is parsed over frames** (zgx-script): `ui.markdown(text, { key, lazy = true })` (or `lazy = bytes`; 4 KB by default) answers a long text arriving at once (not by appends) with one entry a block at once, those not parsed yet `{ t = "pending", hint, lines, from, to }`, and parses them over the next frames, the end first, a block asked for with `first` before the others. Blocks keep their index and, parsed, their table; a computed around the call runs again as they come. The frame showing a 25 KB, 385-block reply parses it in 0.25 ms instead of 0.94 ms. Streaming appends and short texts are as before. Rust hosts call `Incremental::set_text_lazy` and `parse_pending`.
