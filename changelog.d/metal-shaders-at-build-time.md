---
section: performance
---
- **A window's first frame on macOS no longer waits for its shaders to compile** (zgx-gpu): the shaders every first frame needs are compiled when zgx is built, where Xcode's Metal toolchain is, and loaded at start. A launch whose shader cache missed (the first after an install or a system update) drew its first frame about 150 ms later for compiling them; launches with a warm cache are as before. Built without the toolchain (Linux, a cross build, no Metal toolchain installed), or with `ZGX_NO_METALLIB` set, the shaders compile at start as before. The binary grows by about 50 KB.
