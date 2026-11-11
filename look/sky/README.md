# The sky (R6 step 1 and 1b), looked at

Frames from `cargo run --release -p zgx-3d --features media --example atmosphere -- --sun DEG --png PATH` (960x540, ACES, clouds still), on f06fe305c's code, an M1 Max:

- `sun-60.png`: the sun 60 degrees up (out of frame above): the zenith blue, the horizon pale, the clouds white with greyer bases.
- `sun-25.png`: 25 degrees, out of frame above and ahead: haze brightening toward it, the clouds' sunward sides lit warm.
- `sun-04.png`: 4 degrees, its disc in frame: a red horizon, the clouds lit orange from below, the meshes lit by the reddened sun and the sky's environment.

The meshes are lit by `Sky::sun_light` and `Sky::environment` of the same sky. This branch never merges.
