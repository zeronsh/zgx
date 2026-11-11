Blessed on Windows 11 25H2 (build 26200.9711) with `ZGX_BLESS=1`, from a
build with every feature, on WARP (Microsoft Basic Render Driver,
d3d10warp.dll 10.0.26100.9711, D3D12Core.dll 10.0.26100.9549) through
Direct3D 12 with FXC, chosen beside a GPU by `WGPU_FORCE_FALLBACK_ADAPTER=1`.
They match with gltf alone too. Run single-threaded (`--test-threads=1`):
this WARP crashes in `ExecuteCommandLists`, or draws nothing, when several
wgpu devices submit at once in one process. Another WARP version (CI's
Windows Server image, an older or newer Windows) may move these pixels.

The room is drawn by faces on WARP under either compiler since WARP's ray
queries were declined (docs/validation.md, "WARP's ray queries are
declined"), and `gi_room_raster` is what the room test compares.
`gi_room` is kept only so the faces have a traced picture to stand beside
(`faces_drawn_in_place_of_rays_light_the_room_alike`): traced under DXC
(dxcompiler.dll 1.9.2602.17) before the decline, from a run whose rays
matched the CPU reference, within 1 level of Metal's. It can't be blessed
again on WARP, where rays are declined: if the room changes, trace it again
on the RTX 4090 or Metal and compare across families.
