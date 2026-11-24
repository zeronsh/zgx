---
section: added
---
- **`zgx-fx`, an optional package of visual effects**: glow, pulse, ripple, wave, shimmer, sparkles, aurora, grain, electric, fire, motion blur; heat haze, shockwave, lens, clear glass, CRT, chromatic split, fluted glass over what's behind a node; liquid metal, mesh gradient, dither, grain gradient, metaballs, swirl, voronoi, waves, halftone and plastic fills. Rust: `view.fx(Glow::neon())`. Luau: copy `luau/fx.luau`, `require("fx")`. Made only with zgx's public API: an app that doesn't depend on it links none of it. Not on crates.io yet: use it from the repository, with every zgx crate from the same revision (one from crates.io beside it would be a second zgx); `luau/fx.luau` needs no crate.
