---
section: added
---
- **A layer shader can read what lies behind its node and paint past its box**: `zgx_backdrop(p)` reads it (a heat haze, a lens, a chromatic split), `layer_bleed` runs the shader past the box (a glow, a ripple), `layer_reach` says how far past that it reads, and `input.radii` gives the box's corners. Rust: `Styled::layer_bleed` and `layer_reach`, `Scene::set_layer_extent`.
- **The engine can keep a shader up with no script per frame**: `layer_clock` and `shader_clock` step its time on the frame clock while its node shows (off screen, hidden or with motion reduced it holds no timer), `layer_on` takes it and its layer away while false, and `layer_params` may be a function, followed. Rust: `View::shader_drive`.
- **Glass that is the system's on macOS 26** (`ui.glass`, `zgx::glass::glass`): an `NSGlassEffectView` between the window's planes where it's available, and elsewhere a drawn material measured against it, with the same script. `GlassMaterial::Liquid` puts it behind a whole window.
- **An app's layer shaders compile off the frame**, as fills do: the first frames drawing one show the layer as painted, not a frozen window. `ui.layer_shader` starts compiling as it's declared.
