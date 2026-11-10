//! The camera's fly-through: a closed Catmull-Rom path through key views,
//! a function of frame time alone, so the bench's frames repeat.
use zgx_3d::{Camera, Vec3, vec3};

/// Seconds round the path once.
pub const LOOP: f32 = 60.;

/// The key views: when, from where, toward what.
const KEYS: [(f32, [f32; 3], [f32; 3]); 7] = [
    // Out over the water, the ship and the cranes ahead, the sun behind.
    (0., [-300., 34., -210.], [-60., 22., 0.]),
    // Low along the ship's side.
    (8., [-150., 16., -72.], [10., 26., -16.]),
    // Over the quay's edge, under the booms.
    (16., [-40., 13., 10.], [70., 30., -12.]),
    // Up between the cranes, looking out to sea and into the sun.
    (24., [60., 28., 46.], [120., 34., -60.]),
    // Over the yard's stacks.
    (32., [160., 58., 150.], [0., 0., 170.]),
    // High over the yard, looking back over the harbor to the sun.
    (40., [0., 115., 320.], [0., 18., -60.]),
    // Out over the water again, wheeling round to the start.
    (48., [-230., 72., -60.], [0., 20., 0.]),
];

fn key(i: usize) -> (Vec3, Vec3) {
    let (_, p, t) = KEYS[i % KEYS.len()];
    (Vec3::from(p), Vec3::from(t))
}

/// Catmull-Rom between `p1` and `p2` at `t` (0 to 1).
fn spline(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
    let (t2, t3) = (t * t, t * t * t);
    (p1 * 2.
        + (p2 - p0) * t
        + (p0 * 2. - p1 * 5. + p2 * 4. - p3) * t2
        + (-p0 + p1 * 3. - p2 * 3. + p3) * t3)
        * 0.5
}

/// Where the camera is and what it looks at `seconds` into the loop.
pub fn at(seconds: f32) -> (Vec3, Vec3) {
    let s = seconds.rem_euclid(LOOP);
    let n = KEYS.len();
    let i = KEYS
        .iter()
        .rposition(|(time, _, _)| *time <= s)
        .unwrap_or(0);
    let start = KEYS[i].0;
    let end = if i + 1 < n { KEYS[i + 1].0 } else { LOOP };
    let t = (s - start) / (end - start);
    let (a, b, c, d) = (key(i + n - 1), key(i), key(i + 1), key(i + 2));
    (spline(a.0, b.0, c.0, d.0, t), spline(a.1, b.1, c.1, d.1, t))
}

/// The camera `seconds` into the loop, from `base` (its lens and range).
pub fn camera(seconds: f32, base: Camera) -> Camera {
    let (position, target) = at(seconds);
    Camera {
        position,
        target,
        ..base
    }
}

/// The orbit's centre when the fly-through hands over: the harbor's
/// middle, on the quay.
pub const ORBIT_TARGET: Vec3 = vec3(0., 12., 10.);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_passes_through_its_keys_and_closes() {
        for (time, p, t) in KEYS {
            let (pos, target) = at(time);
            assert!((pos - Vec3::from(p)).length() < 1e-3, "{time}");
            assert!((target - Vec3::from(t)).length() < 1e-3);
        }
        let (a, _) = at(LOOP - 1e-4);
        let (b, _) = at(0.);
        assert!((a - b).length() < 0.1, "the loop closes");
    }

    #[test]
    fn the_path_moves_smoothly() {
        let mut last = at(0.).0;
        for k in 1..=(LOOP * 100.) as usize {
            let p = at(k as f32 / 100.).0;
            // Under 100 m/s anywhere.
            assert!((p - last).length() < 1., "a jump at {}", k as f32 / 100.);
            last = p;
        }
    }
}
