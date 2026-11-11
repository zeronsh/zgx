Blessed on Windows 11 25H2 (build 26200.9711) with `ZGX_BLESS=1`, from a
build with every feature, on an NVIDIA GeForce RTX 4090 (driver
32.0.16.1742) through Direct3D 12 with FXC. They match with gltf alone
too, and repeated runs match exactly. The family is named for this exact
adapter, so another NVIDIA GPU, or this one under another name, finds no
goldens here. They are within 3 levels of `warp` in every scene.

No `gi_room`: its probes trace by ray queries, which FXC doesn't compile.
