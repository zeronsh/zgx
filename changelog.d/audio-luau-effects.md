---
section: added
---
- **Effects on buses in Luau** (zgx-script, `audio`): `bus:add_effect(audio.low_pass { cutoff = ui.tween(cutoff) })` and its kin (`high_pass`, `peaking`, `delay`, `reverb`, `compressor`) put the mixer's stock effects on a bus, and `bus:duck(dialogue, { depth = -9 })` ducks it under another. Every parameter follows a state, a function or a binding with no script per frame; `effect:set`, `effect:fade` and `effect:remove` change it after. `voice:crossfade(to, seconds)` fades one voice into another along equal-power curves. A plugin reaches its own bus as `audio.bus()`, with 4 effects at most, counted against its memory. Lint: `audio-effect-in-builder`.
