//! A box's strokes follow its fill's corners: a border lies inside the border box (its outer edge
//! the fill's), an outline outside it at its offset, and a spread ring (`0 0 0 1`) against it, each
//! the same thickness round the corner as along the edge, at 1x and 2x and with a corner shape that
//! isn't round. Read along the corner's diagonal, from outside in.
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
/// The frame's side in pixels.
const SIZE: u32 = 240;
const WHITE: Color = Color(255, 255, 255, 255);
const DARK: Color = Color(20, 20, 20, 255);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ink {
    Clear,
    Stroke,
    Fill,
}
/// What a premultiplied sample is, by where it crosses halfway between the inks.
fn ink(p: [f32; 4]) -> Ink {
    if p[3] < 96. {
        return Ink::Clear;
    }
    // Un-premultiplied lightness: the stroke is white, the fill 20.
    match p[0] / p[3] * 255. >= 137. {
        true => Ink::Stroke,
        false => Ink::Fill,
    }
}

/// What a box is drawn with.
#[derive(Clone, Copy)]
struct Box {
    /// Its size in points.
    size: (f32, f32),
    radius: f32,
    border: f32,
    outline: Option<(f32, f32)>,
    ring: f32,
}
impl Default for Box {
    fn default() -> Self {
        Self {
            size: (100., 100.),
            radius: 0.,
            border: 0.,
            outline: None,
            ring: 0.,
        }
    }
}

/// The box in a frame `scale` times its points, at 20 points in.
fn draw(spec: Box, scale: f32, corner_shape: f32, shift: (f32, f32)) -> Vec<u8> {
    zgx::style::set_corner_shape(corner_shape);
    let points = SIZE as f32 / scale;
    let mut scene = Scene::new(points, points);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let root = scene.root();
    let quad = QuadStyle {
        fill: DARK,
        radius: spec.radius,
        border_color: WHITE,
        border_width: spec.border,
        outline: spec.outline.map(|(width, offset)| Outline {
            width,
            color: WHITE,
            offset,
        }),
        shadow: (spec.ring > 0.).then_some(BoxShadow {
            color: WHITE,
            offset: Transform::default(),
            blur_radius: 0.,
            spread: spec.ring,
        }),
        ..Default::default()
    };
    let id = scene.append(
        root,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad,
        },
        Style {
            width: Some(spec.size.0),
            height: Some(spec.size.1),
            ..Default::default()
        },
    );
    scene.set_transform(
        id,
        Transform {
            x: 20. + shift.0,
            y: 20. + shift.1,
        },
    );
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    gpu.set_scale_factor(scale);
    let frame = scene.flush();
    gpu.render(&scene, &frame.damage).unwrap();
    zgx::style::set_corner_shape(1.);
    gpu.readback().unwrap()
}

/// The frame's premultiplied RGBA at the point (`x`, `y`) in pixels, bilinear between pixel centres.
fn sample(p: &[u8], x: f32, y: f32) -> [f32; 4] {
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let at = |x: f32, y: f32| {
        let (x, y) = (
            (x as i32).clamp(0, SIZE as i32 - 1),
            (y as i32).clamp(0, SIZE as i32 - 1),
        );
        let i = ((y * SIZE as i32 + x) * 4) as usize;
        [
            p[i] as f32,
            p[i + 1] as f32,
            p[i + 2] as f32,
            p[i + 3] as f32,
        ]
    };
    let (a, b, c, d) = (
        at(x0, y0),
        at(x0 + 1., y0),
        at(x0, y0 + 1.),
        at(x0 + 1., y0 + 1.),
    );
    std::array::from_fn(|k| {
        (a[k] * (1. - tx) + b[k] * tx) * (1. - ty) + (c[k] * (1. - tx) + d[k] * tx) * ty
    })
}
/// The inks met going in from `from` (pixels) along `direction` (a unit vector) for `reach` pixels,
/// each with the length it runs, in pixels along the line, edges where the ink passes halfway to
/// the next. Sampled every quarter pixel, so a stroke thinner than a pixel step is not missed.
fn along(p: &[u8], from: (f32, f32), direction: (f32, f32), reach: (f32, f32)) -> Vec<(Ink, f32)> {
    const STEP: f32 = 0.25;
    let mut runs: Vec<(Ink, f32)> = Vec::new();
    let mut t = reach.0;
    while t < reach.1 {
        let pixel = sample(p, from.0 + direction.0 * t, from.1 + direction.1 * t);
        let ink = ink(pixel);
        t += STEP;
        match runs.last_mut() {
            Some((last, length)) if *last == ink => *length += STEP,
            _ => runs.push((ink, STEP)),
        }
    }
    runs
}
/// Along the top-left diagonal, in from outside the corner.
fn along_diagonal(p: &[u8], scale: f32, shift: (f32, f32)) -> Vec<(Ink, f32)> {
    let corner = ((20. + shift.0) * scale, (20. + shift.1) * scale);
    let unit = std::f32::consts::FRAC_1_SQRT_2;
    along(p, corner, (unit, unit), (-10. * scale, 18. * scale))
}
/// Along the top edge's middle, going down from outside.
fn along_edge(p: &[u8], scale: f32, shift: (f32, f32)) -> Vec<(Ink, f32)> {
    along(
        p,
        ((70. + shift.0) * scale, (20. + shift.1) * scale),
        (0., 1.),
        (-10. * scale, 18. * scale),
    )
}
fn kinds(runs: &[(Ink, f32)]) -> Vec<Ink> {
    runs.iter().map(|(ink, _)| *ink).collect()
}
/// The length of run `which`.
fn thickness(runs: &[(Ink, f32)], which: usize) -> f32 {
    runs[which].1
}

/// Where the box lies: on the pixel grid, and between pixels (a centred composer's).
const SHIFTS: [(f32, f32); 3] = [(0., 0.), (0.3, 0.7), (0.5, 0.5)];

/// Every stroke the same thickness at the corner as along the edge, in the order outside to in,
/// at the scale, corner shape and place.
fn check(name: &str, spec: Box, order: &[Ink], scale: f32, shape: f32) {
    for shift in SHIFTS {
        let p = draw(spec, scale, shape, shift);
        let (diagonal, edge) = (
            along_diagonal(&p, scale, shift),
            along_edge(&p, scale, shift),
        );
        let at = format!("{name} {scale}x k{shape} at {shift:?}");
        let column: Vec<[f32; 4]> = (-3..4)
            .map(|dy| {
                sample(
                    &p,
                    (70. + shift.0) * scale,
                    (20. + shift.1) * scale + dy as f32 * 0.5,
                )
            })
            .collect();
        assert_eq!(kinds(&edge), order, "{at}: edge {edge:?} column {column:?}");
        assert_eq!(
            kinds(&diagonal),
            order,
            "{at}: the corner has the same layers, outside to in: {diagonal:?} (edge {edge:?})"
        );
        // The layers between the outside and the fill: how far the scan began and ended is no layer's.
        for layer in 1..order.len() - 1 {
            assert!(
                (thickness(&diagonal, layer) - thickness(&edge, layer)).abs() <= 0.75,
                "{at}: layer {layer} is {} round the corner, {} along the edge ({diagonal:?} vs {edge:?})",
                thickness(&diagonal, layer),
                thickness(&edge, layer)
            );
        }
    }
}

use Ink::{Clear, Fill, Stroke};

#[test]
fn a_border_lies_inside_the_box_its_corner_as_thick_as_its_edge() {
    let _gpu = gpu_fixture();
    for scale in [1., 2.] {
        for shape in [1., 1.5] {
            let spec = Box {
                radius: 20.,
                border: 1.,
                ..Default::default()
            };
            check("border 1", spec, &[Clear, Stroke, Fill], scale, shape);
            let spec = Box {
                radius: 20.,
                border: 3.,
                ..Default::default()
            };
            check("border 3", spec, &[Clear, Stroke, Fill], scale, shape);
        }
    }
}

#[test]
fn an_outline_rings_the_box_at_its_offset_round_the_corner() {
    let _gpu = gpu_fixture();
    for scale in [1., 2.] {
        for shape in [1., 1.5] {
            let spec = Box {
                radius: 20.,
                outline: Some((1., 2.)),
                ..Default::default()
            };
            check(
                "outline 1 at 2",
                spec,
                &[Clear, Stroke, Clear, Fill],
                scale,
                shape,
            );
        }
    }
}

#[test]
fn a_spread_ring_hugs_the_box_round_the_corner() {
    let _gpu = gpu_fixture();
    for scale in [1., 2.] {
        for shape in [1., 1.5] {
            let spec = Box {
                radius: 20.,
                ring: 1.,
                ..Default::default()
            };
            check("ring 0 0 0 1", spec, &[Clear, Stroke, Fill], scale, shape);
        }
    }
}

#[test]
fn a_border_with_a_spread_ring_is_one_box() {
    // A toolbar's: rounded 15, a 1 border, and a ring round it.
    let _gpu = gpu_fixture();
    for scale in [1., 2.] {
        for shape in [1., 1.5] {
            let spec = Box {
                radius: 15.,
                border: 1.,
                ring: 1.,
                ..Default::default()
            };
            check("toolbar", spec, &[Clear, Stroke, Fill], scale, shape);
        }
    }
}

#[test]
fn a_half_point_border_never_leaves_the_fill_poking_out_at_the_corner() {
    let _gpu = gpu_fixture();
    for scale in [1., 2.] {
        for shape in [1., 1.5] {
            let p = draw(
                Box {
                    radius: 20.,
                    border: 0.5,
                    ..Default::default()
                },
                scale,
                shape,
                (0., 0.),
            );
            // From outside in along the diagonal, the first thing met is
            // lighter than the fill: the stroke's edge is the fill's.
            let corner = 20. * scale;
            let unit = std::f32::consts::FRAC_1_SQRT_2;
            let first = (0..)
                .map(|i| -10. * scale + i as f32 * 0.25)
                .take_while(|t| *t < 18. * scale)
                .map(|t| sample(&p, corner + unit * t, corner + unit * t))
                .find(|pixel| pixel[3] >= 8.)
                .expect("something is drawn");
            let lightness = first[0] / first[3] * 255.;
            assert!(
                lightness > 80.,
                "{scale}x k{shape}: the first thing in, {first:?}, is the dark fill"
            );
        }
    }
}

#[test]
fn a_pill_and_a_large_radius_card_keep_their_ring_at_the_corner() {
    // A composer's: rounded 999 (a pill) or 27.5 on a box just over twice that tall, a `0 0 0 1`
    // ring (the edge), at a continuous corner shape.
    let _gpu = gpu_fixture();
    for scale in [1., 2.] {
        for shape in [1., 1.5] {
            let pill = Box {
                size: (80., 40.),
                radius: 999.,
                ring: 1.,
                ..Default::default()
            };
            check("pill ring", pill, &[Clear, Stroke, Fill], scale, shape);
            let card = Box {
                size: (80., 56.4),
                radius: 27.5,
                ring: 1.,
                ..Default::default()
            };
            check("card ring", card, &[Clear, Stroke, Fill], scale, shape);
            let bordered = Box {
                size: (80., 56.4),
                radius: 27.5,
                border: 1.,
                ring: 1.,
                ..Default::default()
            };
            check(
                "card border + ring",
                bordered,
                &[Clear, Stroke, Fill],
                scale,
                shape,
            );
        }
    }
}

#[test]
fn a_ring_round_an_opaque_box_leaves_no_seam_at_a_fractional_edge() {
    // The ring and the fill both anti-alias a pixel the box's edge falls in; drawn one over the
    // other they left it a quarter see-through, a line of what is behind.
    let _gpu = gpu_fixture();
    for scale in [1., 2.] {
        for (w, h, r) in [(80., 56.4, 27.5), (80., 56., 27.5), (80., 40., 999.)] {
            for shift in SHIFTS {
                let spec = Box {
                    size: (w, h),
                    radius: r,
                    ring: 1.,
                    ..Default::default()
                };
                let p = draw(spec, scale, 1.5, shift);
                let x = ((50. + shift.0) * scale) as i32;
                let alphas: Vec<u8> = (10..(60. * scale) as i32)
                    .map(|y| p[((y * SIZE as i32 + x) * 4 + 3) as usize])
                    .collect();
                // From the first pixel the ring fills (the one before it is its outer edge) to the fill's
                // inside, every pixel is whole.
                let first = alphas.iter().position(|a| *a >= 240).expect("a ring");
                assert!(
                    alphas[first..].iter().all(|a| *a >= 240),
                    "{scale}x {w}x{h} r{r} at {shift:?}: a seam in {:?}",
                    &alphas[first.saturating_sub(2)..first + 8]
                );
            }
        }
    }
}
