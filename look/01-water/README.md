# 01: the water

**Notice:** the sea is dark, murky slate reflecting the sky, with waves that hold still at every distance (no moiré, no speckle at the horizon or on the far plane), and the sun's glint at 40 s is a broad soft path, not clipped white fields; the turquoise strip along the quay and the white band at the hull are gone.

What changed (`examples/harbor/src/shaders/water.wgsl`): the scene behind is no longer multiplied by 2^11 (with the post chain on it is already the scene's light: `scene_is_light()`); the waves come from a compute pass's field, averaged over each pixel's footprint and faded into roughness where a pixel can't hold them; roughness from the wind (Cox and Munk); murky water that lets back almost nothing of what's under it; refraction a small bend that fades to none toward grazing and with distance.

Still to come: reflections of the ship and cranes (the engine's), the ground (next), the sky (the engine's).
