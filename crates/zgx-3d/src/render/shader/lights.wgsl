
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
//X     let z = u32(clamp(d * grid.z + grid.w, 0.0, 23.0));
//X     let tile = min(vec2<u32>(max(pixel * grid.xy, vec2(0.0))), vec2(15u, 8u));
//X     let cluster = (z * 9u + tile.y) * 16u + tile.x;
//X     let pair = clusters[bases.y + cluster / 2u];
//X     let odd = (cluster & 1u) == 1u;
//X     let first = select(pair.x, pair.z, odd);
//X     return vec2(first, first + select(pair.y, pair.w, odd));
//X }
//X // The `i`th index's light: where it is and how far it reaches, its
//X // colour times its intensity and its radius, its cone.
//X fn cluster_light(i: u32) -> array<vec4<f32>, 3> {
//X     let bases = clusters[2];
//X     let at = bases.x + clusters[bases.z + i / 4u][i % 4u] * 3u;
//X     return array(bitcast<vec4<f32>>(clusters[at]), bitcast<vec4<f32>>(clusters[at + 1u]), bitcast<vec4<f32>>(clusters[at + 2u]));
//X }
//X // A light reaching a fragment at `world`: which way, how much, and how
//X // much wider its highlights are than a point's.
//X struct Lamp {
//X     l: vec3<f32>,
//X     radiance: vec3<f32>,
//X     widen: f32,
//X }
//X fn lamp_at(light: array<vec4<f32>, 3>, world: vec3<f32>) -> Lamp {
//X     let to = light[0].xyz - world;
//X     let distance = length(to);
//X     let l = to / distance;
//X     let radius = light[1].w;
//X     // Falling as the square of the distance, from a sphere of its radius
//X     // (bounded where it nearly touches), smoothly to nothing at its reach.
//X     let window = pow(clamp(1.0 - pow(distance / light[0].w, 4.0), 0.0, 1.0), 2.0);
//X     let falloff = window / max(distance * distance + radius * radius, 1e-4);
//X     // Within a spot's cone (all round: 1).
//X     let cone = clamp(dot(light[2].xyz, -l) + light[2].w, 0.0, 1.0);
//X     // A sphere's highlight: as a rougher surface's, by the angle it spans
//X     // (Karis 2013, sphere lights' roughness).
//X     return Lamp(l, light[1].rgb * (falloff * cone * cone), radius / (2.0 * max(distance, 1e-4)));
//X }
//X // `roughness` widened for a light `widen` across (its square, α, grown).
//X fn widened(roughness: f32, widen: f32) -> f32 {
//X     return sqrt(min(roughness * roughness + widen, 1.0));
//X }
