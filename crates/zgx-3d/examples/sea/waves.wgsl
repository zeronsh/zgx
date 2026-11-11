// The sea's wave field (examples/sea.rs): a sum of wind waves from a
// spectrum, each on the lattice of the tile so the tile repeats without a
// seam, written as height and its two slopes into a float texture the
// water's material reads. Two passes of this source: `tick` moves the sea's
// own clock on, then `field` writes the waves at that time.

struct Frame { time: f32, delta: f32, index: u32 }
@group(0) @binding(0) var<uniform> frame: Frame;
// Each wave: its wave vector (x, z, radians a metre), height (m), phase.
@group(0) @binding(1) var<storage, read> waves: array<vec4<f32>>;
// The sea's own clock: x its seconds, y how fast it runs (0: still), z its
// tile in metres, w the waves in use.
@group(0) @binding(2) var<storage, read_write> clock: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> now: array<vec4<f32>>;
@group(0) @binding(4) var field: texture_storage_2d<rgba16float, write>;

const G: f32 = 9.81;

// A step at most a tenth of a second: a sea started again after a pause
// goes on from where it stood.
@compute @workgroup_size(1)
fn tick() {
    clock[0].x += min(frame.delta, 0.1) * clock[0].y;
}

@compute @workgroup_size(8, 8)
fn field_at(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(field);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let c = now[0];
    let p = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(size) * c.z;
    var sum = vec3(0.0);
    let count = min(u32(c.w), arrayLength(&waves));
    for (var i = 0u; i < count; i++) {
        let w = waves[i];
        let k = w.xy;
        let omega = sqrt(G * length(k));
        let phase = dot(k, p) - omega * c.x + w.w;
        let s = sin(phase);
        sum += vec3(w.z * cos(phase), -w.z * k.x * s, -w.z * k.y * s);
    }
    textureStore(field, id.xy, vec4(sum, 1.0));
}
