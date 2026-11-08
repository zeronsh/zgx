# zgx-fx

Ready-made visual effects for [zgx](../zgx), made with its public API alone: an app that doesn't depend on this crate pays nothing. Luau apps copy `luau/fx.luau` beside their scripts.

```rust
use zgx_fx::{FxExt, Glow};
div().size(200., 80.).rounded(12.).fx(Glow::neon())
```

```luau
local fx = require("fx")
return ui.div(fx.with(fx.glow { preset = "neon" }, { w = 200, h = 80, rounded = 12 }))
```

Lights: `glow`, `pulse`, `ripple`, `wave`, `shimmer`, `sparkles`, `aurora`, `grain`, `electric`, `fire`, `motion_blur`. Over what's behind: `heat_haze`, `shockwave`, `lens`, `rim_lens`, `crt`, `chroma_split`, `fluted_glass`. Fills: `liquid_metal`, `mesh_gradient`, `dither`, `grain_gradient`, `metaballs`, `swirl`, `voronoi`, `waves`, `halftone`, `plastic`. Run `cargo run -p zgx-fx --example fx_sheet -- out.png` to see them, and `cargo run --release -p zgx-fx --example fx_timings` for what each costs the GPU.

The gallery (`ZGX_SCRIPT_DIR=crates/zgx-fx/examples/gallery cargo run -p zgx-script --example hot_reload`) shows them all, and three effects written in its own file with public props alone: a lava melt, and a trail and a bloom made of shader passes reading moving boxes.

If an effect here needs something zgx doesn't offer, that's a capability zgx lacks: it goes in zgx as a general one, not here.
