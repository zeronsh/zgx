# The harbor's frames, looked at (1920x1080, HUD hidden, tip c3c691449 + the demo's WIP)

Frames: `harbor-{00,10,20,30,40,50}.00s.png` in this folder.

**Verdict:** most of what reads as cheap is mine (scene, materials, models). A smaller set is missing engine primitives, named per item.

## 00 s: out over the water toward the ship and cranes

**Mine:**
1. The far water at the left edge is a white speckle band. The ripples aren't band-limited by distance (they alias into noise), and the sun's glint on aliased normals sparkles.
2. A turquoise band runs along the quay behind the stern. Refraction shows the quay's slabs under the water, absorbed as if through only 3 m of clear water; a harbour is murky.
3. Under the stern sit two black triangles: the transom is a fan closing the hull section, shaded dark, and the hull bottom flares out. These are the "hull planes meeting badly".
4. The hull is one flat grey extrusion, with no bow flare, bulb, transom overhang, bulwark or railing.
5. The superstructure is a plain white box; its windows are only on the forward face. The funnel is a black box with a yellow band.
6. The cranes are flat red boxes with no detail: no ladders, walkways, cabins, bracing plates or end trucks. The machinery houses are white boxes.
7. At this distance the containers read as small coloured blocks; their corrugation is too shallow to read.

**The engine can't:**
- **No fog or aerial perspective** (R6): the yard 600 m away is as crisp and saturated as the near water.
- **No sky** (R6; G2): the background is flat lilac, with no sun glow, gradient or clouds.
- **No reflections** (screen-space, planar or probe; R8): no ship or crane is reflected in the water.

## 10 s: low along the ship's side

**Mine:**
1. The superstructure's side fills half the frame as one flat peach face: no windows, doors, decks or rails.
2. The deck containers' ends show contour rings like wood grain. My container wear shader's noise is stretched over the doors' narrow UV strip and banded.
3. At the horizon the water is a glittering white noise band (ripples aliasing).
4. Near the camera, the water's fine ripples are a regular moiré, not waves.
5. The hull side and deck edge are flat and bare.

**The engine can't:**
- No reflection of the hull in the water (R8).
- No fog.

## 20 s: between the crane legs

**Mine:**
1. **A white fringe on beam edges against the water and apron.** My refraction offset (`screen_uv + slope * 0.03`) samples the scene behind it, which includes the bright foreground at silhouettes. With no `scene_depth` I can't reject foreground samples, so the offset must be tiny or gone.
2. **Large white clipped fields** on the water and apron in the background: the sun's glint on aliased, mirror-smooth (roughness 0.035) ripples, clipping under the exposure.
3. The crane members are flat pastel boxes, washed out in the sun, with no bevels, plates or detail.
4. The spreaders are plain yellow slabs.
5. The containers on the left show smeared pink noise.

**The engine can't:**
- No ambient occlusion or contact shadows (R8, R3): beams meet without any darkening, so everything floats.
- No fog: the distant shore is a flat blue cut-out with a bright line under it.

## 30 s: over the yard's stacks

**Mine:**
1. The ground is a blue mirror. My wet layer (roughness 0.14 everywhere it rains, puddles over half the yard) turns asphalt into glass reflecting the lilac environment. Real wet asphalt is dark, with puddles here and there.
2. The container palette is candy-saturated and noisy. The wear texture's noise reads as smears; there are no doors, locking bars or numbers.
3. The lamp heads are big flat white boxes.
4. The sea at the right edge is a white-noise plane, plus a white rectangle where the far plane and the grid meet.

**The engine can't:**
- No AO or contact shadows: the stacks look pasted onto the ground.

## 40 s: high over the yard, looking back to the sun

**Mine:**
1. The sun's glint path on the right clips to a white field. My mirror-smooth water and aliased slopes put the sun's full specular into big patches; a sea's glitter is many small sparks over a wide path.
2. The whole sea is a fine regular noise pattern (moiré), not waves.
3. Where the water meets the far ridge there's a white glowing line (glitter plus bloom).
4. The ship and cranes are boxes; the superstructure is a white box with dotted windows.
5. The yard is dark and low in contrast at this sun, and the lamps are tiny.

**The engine can't:**
- No fog or aerial perspective: the ridge is a dark cardboard silhouette.
- No sky.

## 50 s: wheeling back over the water

**Mine:**
1. A white speckled wedge on the left: the far water plane beyond the near grid, aliased ripples plus glint.
2. The turquoise band between the ship and the quay (as at 00 s).
3. The two black triangles under the stern (as at 00 s).
4. Plain boxes for the superstructure, funnel, machinery houses and trolleys.

## Across all frames

**Mine:**
- The water's look: aliasing (no derivative band-limiting), mirror roughness, turquoise refraction, and the refraction fringe.
- The wet layer far too strong.
- The models are boxes: the hull form, superstructure, cranes, lamps and quay furniture all need real shapes, bevels and detail.
- The containers need doors, bars and markings, and corrugation that reads (a normal map, as well as geometry).
- The composition: the default frames are mid-tone and flat, with no raking light on wet ground.

**Missing engine primitives:**
- fog and aerial perspective, and volumetric light shafts (R6);
- a sky with gradient, sun and clouds (R6, G2);
- reflections on water: screen-space, planar or probe (R8);
- ambient occlusion and contact shadows (R8, R3);
- scene depth, for the water's depth colour, foam and soft edges, and to reject foreground in refraction (R2b);
- decals for markings (R5).

Bevels and normal detail are mine to make (geometry, normal maps). Shadow filtering looks acceptable in these frames.
