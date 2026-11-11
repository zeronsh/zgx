Blessed on Windows 11 25H2 (build 26200.9711) with `ZGX_BLESS=1`, from a
build with every feature, on an NVIDIA GeForce RTX 4090 (driver
32.0.16.1742) through Direct3D 12 with FXC. They match with gltf alone
too, and repeated runs match exactly. The family is named for this exact
adapter, so another NVIDIA GPU, or this one under another name, finds no
goldens here. They are within 3 levels of `warp` in every scene.

`gi_room` is DXC's alone (`WGPU_DX12_COMPILER=dxc`, dxcompiler.dll
1.9.2602.17): its probes trace by ray queries, which FXC never compiles,
so under FXC the room is drawn by faces and compared with
`gi_room_raster`, blessed under FXC. Each matches the other compiler's
run too where both draw it.
