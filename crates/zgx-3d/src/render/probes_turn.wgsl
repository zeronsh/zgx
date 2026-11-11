// A probe's rays (`probes.rs`), shared by its updaters and its blend: a
// spherical Fibonacci set turned by the probe's round, the same turn for
// the same round, so its rounds' rays cover the sphere between them and
// what a probe converges to doesn't depend on when its rounds ran.

// The `i`th of `n` directions spread evenly over the sphere.
fn fibonacci(i: u32, n: u32) -> vec3<f32> {
    let phi = 2.0 * 3.14159265 * fract(f32(i) * 0.618034);
    let z = 1.0 - (2.0 * f32(i) + 1.0) / f32(n);
    let r = sqrt(max(1.0 - z * z, 0.0));
    return vec3(r * cos(phi), r * sin(phi), z);
}
// A number from 0 to 1 for `x`, the same every time.
fn turn_hash(x: u32) -> f32 {
    var h = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return f32((h >> 22u) ^ h) / 4294967295.0;
}
// Ray `i` of `n` in round `round`: the set turned by a random rotation (a
// unit quaternion, Shoemake's) the round picks.
fn turned(i: u32, n: u32, round: u32) -> vec3<f32> {
    let u = vec3(turn_hash(round * 3u), turn_hash(round * 3u + 1u), turn_hash(round * 3u + 2u));
    let a = sqrt(1.0 - u.x);
    let b = sqrt(u.x);
    let tau = 6.28318531;
    let q = vec4(a * sin(tau * u.y), a * cos(tau * u.y), b * sin(tau * u.z), b * cos(tau * u.z));
    let v = fibonacci(i, n);
    let t = 2.0 * cross(q.xyz, v);
    return normalize(v + q.w * t + cross(q.xyz, t));
}
