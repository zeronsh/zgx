---
section: fixed
---
- **An offscreen renderer compiles its shaders on its own thread** (zgx-gpu): it waited for the compile thread anyway (no window to wake), and meanwhile both threads allocated, so its frames' work moved run to run with how their allocations interleaved: the gate's shader cells (`fx-layer`, `fx-reader`, `fx-passes`) counted up to 0.46% apart on one binary. A window's frames still go on while its shaders compile off the frame.
