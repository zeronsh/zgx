Blessed on Windows 11 25H2 (build 26200.9711) with `ZGX_BLESS=1`, from a
build with every feature, on WARP (Microsoft Basic Render Driver,
d3d10warp.dll 10.0.26100.9711, D3D12Core.dll 10.0.26100.9549) through
Direct3D 12 with FXC, chosen beside a GPU by `WGPU_FORCE_FALLBACK_ADAPTER=1`.
They match with gltf alone too. Run single-threaded (`--test-threads=1`):
this WARP crashes in `ExecuteCommandLists`, or draws nothing, when several
wgpu devices submit at once in one process. Another WARP version (CI's
Windows Server image, an older or newer Windows) may move these pixels.

No `gi_room`: its probes trace by ray queries, which FXC doesn't compile.
