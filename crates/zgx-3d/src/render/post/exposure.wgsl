// Automatic exposure: a histogram of the scene's log luminance, then its
// middle's mean, the exposure eased toward it.
struct Settings {
    // log2 of the dimmest mean adapted to, the range up to the brightest,
    // the shares left out below and above.
    range: vec4<f32>,
    // Stops a second brightening and darkening, the frame's step in
    // seconds, and 1 where the last state is to be dropped.
    speed: vec4<f32>,
}
struct State {
    // log2 of the mean luminance adapted to, and of the frame's.
    adapted: f32,
    target_: f32,
    // 1 once a frame has adapted.
    made: f32,
    pad: f32,
}
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> bins: array<atomic<u32>, 256>;
@group(0) @binding(2) var<storage, read_write> state: State;
@group(0) @binding(3) var<uniform> settings: Settings;

var<workgroup> local: array<atomic<u32>, 256>;

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}

// Bin 0 holds what's too dark to count; 1 to 255 the range.
@compute @workgroup_size(16, 16)
fn histogram(@builtin(global_invocation_id) at: vec3<u32>, @builtin(local_invocation_index) i: u32) {
    atomicStore(&local[i], 0u);
    workgroupBarrier();
    let size = textureDimensions(scene);
    if at.x < size.x && at.y < size.y {
        let c = textureLoad(scene, vec2<i32>(at.xy), 0);
        // A pixel's own colour, its coverage divided out; the background
        // (nothing drawn) not counted.
        if c.a > 0.004 {
            let l = luma(c.rgb / c.a);
            var bin = 0u;
            if l > 1e-6 {
                let t = clamp((log2(l) - settings.range.x) / settings.range.y, 0.0, 1.0);
                bin = 1u + u32(t * 254.0);
            }
            atomicAdd(&local[bin], 1u);
        }
    }
    workgroupBarrier();
    let n = atomicLoad(&local[i]);
    if n > 0u {
        atomicAdd(&bins[i], n);
    }
}

var<workgroup> counts: array<u32, 256>;

@compute @workgroup_size(256)
fn adapt(@builtin(local_invocation_index) i: u32) {
    counts[i] = atomicExchange(&bins[i], 0u);
    workgroupBarrier();
    if i != 0u {
        return;
    }
    var total = 0u;
    for (var b = 1u; b < 256u; b++) {
        total += counts[b];
    }
    var target_ = state.adapted;
    if total > 0u {
        // The mean of the bins between the shares left out, in log2.
        let low = settings.range.z * f32(total);
        let high = settings.range.w * f32(total);
        var below = 0.0;
        var sum = 0.0;
        var weight = 0.0;
        for (var b = 1u; b < 256u; b++) {
            let n = f32(counts[b]);
            let start = max(below, low);
            let end = min(below + n, high);
            if end > start {
                let centre = settings.range.x + (f32(b) - 0.5) / 254.0 * settings.range.y;
                sum += centre * (end - start);
                weight += end - start;
            }
            below += n;
        }
        if weight > 0.0 {
            target_ = sum / weight;
        }
    }
    target_ = clamp(target_, settings.range.x, settings.range.x + settings.range.y);
    if state.made < 0.5 || settings.speed.w > 0.5 {
        state.adapted = target_;
    } else {
        // Linearly in stops: up and down at their own rates.
        let rate = select(settings.speed.y, settings.speed.x, target_ > state.adapted);
        let step = rate * settings.speed.z;
        state.adapted += clamp(target_ - state.adapted, -step, step);
    }
    state.target_ = target_;
    state.made = 1.0;
}
