// The sea's wave field, two compute passes of this source:
// - `tick` moves the sea's own clock on by the frame's step (at most a
//   tenth of a second), so a sea paused and started again goes on from
//   where it stood;
// - `waves` sums a spectrum of wind waves over one tile of the sea at that
//   clock's time, written as a texture the water's material reads (height,
//   and its slope along x and z). Every wave's vector lies on the tile's
//   lattice, so the tile repeats without a seam.
// (The clock is the compute lane's idea, from its parked sea example.)
struct Frame { time: f32, delta: f32, index: u32 }
@group(0) @binding(0) var<uniform> frame: Frame;
// Each wave: its vector (radians a metre, x and z), height (m), phase.
@group(0) @binding(1) var<storage, read> spectrum: array<vec4f>;
// The sea's clock: x its seconds.
@group(0) @binding(2) var<storage, read_write> clock: array<vec4f>;
@group(0) @binding(3) var<storage, read> now: array<vec4f>;
@group(0) @binding(4) var field: texture_storage_2d<rgba16float, write>;

const SIDE: u32 = 512u;
const TILE: f32 = 128.0;
const G: f32 = 9.81;

@compute @workgroup_size(1)
fn tick() {
    clock[0].x += min(frame.delta, 0.1);
}

@compute @workgroup_size(8, 8)
fn waves(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= SIDE || id.y >= SIDE {
        return;
    }
    let p = vec2f(id.xy) / f32(SIDE) * TILE;
    let t = now[0].x;
    var h = 0.0;
    var slope = vec2f(0.0);
    let count = arrayLength(&spectrum);
    for (var i = 0u; i < count; i++) {
        let w = spectrum[i];
        let k = w.xy;
        // Deep water: a wave of wavenumber |k| moves at sqrt(g |k|).
        let omega = sqrt(G * length(k));
        let phase = dot(k, p) - omega * t + w.w;
        h += w.z * sin(phase);
        slope += w.z * cos(phase) * k;
    }
    textureStore(field, id.xy, vec4f(h, slope, 0.0));
}
