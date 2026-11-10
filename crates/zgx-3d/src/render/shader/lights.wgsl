
//X // Clustered lights (`render/lights.rs`): a header, the lights three words each,
//X // each cluster's first index and count two to a word, the indices four to
//X // a word.
//X @group(0) @binding(5) var<storage, read> clusters: array<vec4<u32>>;
//X // Where in the indices the lights of the cluster holding the fragment at
//X // `pixel` (its position in the target), seen at `world`, start and end.
//X fn cluster_span(pixel: vec2<f32>, world: vec3<f32>) -> vec2<u32> {
//X     let grid = bitcast<vec4<f32>>(clusters[0]);
//X     let facing = bitcast<vec4<f32>>(clusters[1]);
//X     let bases = clusters[2];
//X     // Its depth along the camera's facing, sliced by its logarithm (or
//X     // linearly, seen orthographically).
//X     let depth = dot(world - frame.camera.xyz, facing.xyz);
//X     let d = select(log(max(depth, 1e-6)), depth, facing.w > 0.5);
//X     // The grid's tiles across and down, and its slices.
//X     let counts = clusters[3];
//X     let z = min(u32(max(d * grid.z + grid.w, 0.0)), counts.z - 1u);
//X     let tile = min(vec2<u32>(max(pixel * grid.xy, vec2(0.0))), counts.xy - 1u);
//X     let cluster = (z * counts.y + tile.y) * counts.x + tile.x;
//X     let pair = clusters[bases.y + cluster / 2u];
//X     let odd = (cluster & 1u) == 1u;
//X     let first = select(pair.x, pair.z, odd);
//X     return vec2(first, first + select(pair.y, pair.w, odd));
//X }
//X // Where the `i`th index's light starts: where it is and how far it
//X // reaches, its colour times its intensity and its radius, its cone.
//X fn cluster_light(i: u32) -> u32 {
//X     let bases = clusters[2];
//X     return bases.x + clusters[bases.z + i / 4u][i % 4u] * 3u;
//X }
//X // A light reaching a fragment at `world`: which way, how much, and how
//X // much wider its highlights are than a point's.
//X struct Lamp {
//X     l: vec3<f32>,
//X     radiance: vec3<f32>,
//X     widen: f32,
//X }
//X fn lamp_at(at: u32, world: vec3<f32>) -> Lamp {
//X     // Read word by word (not as an array, which Metal may keep in memory).
//X     let place = bitcast<vec4<f32>>(clusters[at]);
//X     let shine = bitcast<vec4<f32>>(clusters[at + 1u]);
//X     let aim = bitcast<vec4<f32>>(clusters[at + 2u]);
//X     let to = place.xyz - world;
//X     let distance = max(length(to), 1e-4);
//X     let l = to / distance;
//X     let radius = shine.w;
//X     // Falling as the square of the distance, from a sphere of its radius
//X     // (bounded where it nearly touches), smoothly to nothing at its reach.
//X     let window = pow(clamp(1.0 - pow(distance / place.w, 4.0), 0.0, 1.0), 2.0);
//X     let falloff = window / max(distance * distance + radius * radius, 1e-4);
//X     // Within a spot's cone (all round: 1).
//X     let cone = clamp(dot(aim.xyz, -l) + aim.w, 0.0, 1.0);
//X     // A sphere's highlight: as a rougher surface's, by the angle it spans
//X     // (Karis 2013, sphere lights' roughness).
//X     // Nothing where it doesn't reach, whatever it holds (never inf × 0).
//X     let reach = falloff * cone * cone;
//X     return Lamp(l, select(vec3(0.0), shine.rgb * reach, reach > 0.0), radius / (2.0 * distance));
//X }
//X // `roughness` widened for a light `widen` across (its square, α, grown).
//X fn widened(roughness: f32, widen: f32) -> f32 {
//X     return sqrt(min(roughness * roughness + widen, 1.0));
//X }
