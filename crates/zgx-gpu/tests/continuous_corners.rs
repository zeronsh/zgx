//! `CornerCurve::Continuous`: `CALayer`'s corner, read from QuartzCore. Pixels
//! against that function and `CALayer`'s own at the sizes iOS draws, and on every
//! path a box's quads can take: a variant the frame loses on the way draws the
//! circle silently.
use std::sync::Arc;
use zgx::compose::prelude::*;
use zgx::decoration::{Background, CornerCurve, Corners, Decoration};
use zgx::scene::*;
use zgx::widgets::Ui;
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
const INK: Color = Color(0, 0, 0, 255);
const REACH: f64 = 1.52866495;
/// `UberShader::supercircle_image` as read from QuartzCore: 0 on the curve.
fn field(a: (f64, f64), t: (f64, f64)) -> f64 {
    let (high, low) = (a.0.max(a.1), a.0.min(a.1));
    let s = if high > 0. { (low / high).min(1.) } else { 0. };
    let p = (((3.15600991 - 0.926054001 * s) * s - 3.64122009) * s + 1.26803005) * s + 0.268530995;
    let round = a.0.hypot(a.1) + 1. - 1. / (1. - s * s * p);
    let v = (
        (a.0 * REACH - 0.528665006).max(0.),
        (a.1 * REACH - 0.528665006).max(0.),
    );
    let circle = v.0.hypot(v.1) * 0.654165566 + 0.345834434;
    let k = if a.1 > a.0 { t.0 } else { t.1 };
    round + (circle - round) * k - 1.
}
/// A way's progress to the circular corner, as `fill_round_rect` works it out.
fn morph(size: f64, radius: f64) -> f64 {
    (2.8915570351735034 - 1.4457785175867517 * size / (radius * REACH)).clamp(0., 1.)
}
/// The field at a point from a box's top left.
fn field_at(x: f64, y: f64, size: (f64, f64), radius: f64) -> f64 {
    let reach = radius * REACH;
    if x >= reach || y >= reach {
        return -x.min(y) / reach;
    }
    field(
        (((reach - x) / reach).max(0.), ((reach - y) / reach).max(0.)),
        (morph(size.0, radius), morph(size.1, radius)),
    )
}
/// The coverage `CALayer` gives a pixel of a box's top left corner: its ramp is
/// a pixel across the field's slope.
fn coverage(column: u32, row: u32, size: (f64, f64), radius: f64, scale: f64) -> f64 {
    let at =
        |column: f64, row: f64| field_at((column + 0.5) / scale, (row + 0.5) / scale, size, radius);
    let (c, r) = (column as f64, row as f64);
    let across = (at(c + 0.5, r) - at(c - 0.5, r)).abs();
    let down = (at(c, r + 0.5) - at(c, r - 0.5)).abs();
    (0.5 - at(c, r) / (across + down).max(1e-4)).clamp(0., 1.)
}
fn box_style(radius: f32, curve: CornerCurve) -> QuadStyle {
    QuadStyle {
        fill: INK,
        radius,
        decoration: Some(Arc::new(Decoration {
            corners: Some(Corners::all(radius)),
            corner_curve: curve,
            ..Default::default()
        })),
        ..Default::default()
    }
}
fn shadow(color: Color, blur_radius: f32) -> BoxShadow {
    BoxShadow {
        color,
        blur_radius,
        offset: Transform { x: 0., y: 0. },
        spread: 0.,
    }
}
fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
/// A window of `size` points at `scale` with a box of `style` at (4, 4), or
/// over what `build` adds first; drawn, and against a full repaint.
struct Drawn {
    pixels: Vec<u8>,
    width: u32,
}
impl Drawn {
    fn alpha(&self, x: u32, y: u32) -> f64 {
        self.pixels[((y * self.width + x) * 4 + 3) as usize] as f64 / 255.
    }
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * self.width + x) * 4) as usize;
        self.pixels[at..at + 4].try_into().unwrap()
    }
}
fn draw(gpu: &mut GpuRenderer, scene: &mut Scene, width: u32) -> Drawn {
    let damage = scene.flush().damage;
    gpu.render(scene, &damage).unwrap();
    Drawn {
        pixels: gpu.readback().unwrap(),
        width,
    }
}
fn window(size: (f32, f32), scale: f32) -> (GpuRenderer, Scene, u32) {
    let (width, height) = (
        (size.0 * scale).round() as u32,
        (size.1 * scale).round() as u32,
    );
    let mut gpu = GpuRenderer::new(width, height).unwrap();
    gpu.set_scale_factor(scale);
    let mut scene = Scene::new(size.0, size.1);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    (gpu, scene, width)
}
fn add(scene: &mut Scene, style: QuadStyle, at: (f32, f32), size: (f32, f32)) -> NodeId {
    let root = scene.root();
    let id = scene.append(
        root,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad: style,
        },
        fixed(size.0, size.1),
    );
    scene.set_transform(id, Transform { x: at.0, y: at.1 });
    id
}

/// The worst difference in levels of 255 between a corner's pixels and the law.
fn worst_level(drawn: &Drawn, at: f64, size: (f64, f64), radius: f64, scale: f64) -> f64 {
    let origin = (at * scale).round() as u32;
    let reach = radius * REACH;
    let (columns, rows) = (
        ((reach + 2.).min(size.0 / 2.) * scale) as u32,
        ((reach + 2.).min(size.1 / 2.) * scale) as u32,
    );
    let mut worst: f64 = 0.;
    for row in 0..rows {
        for column in 0..columns {
            let want = coverage(column, row, size, radius, scale);
            let got = drawn.alpha(origin + column, origin + row);
            worst = worst.max((want - got).abs() * 255.);
        }
    }
    worst
}

#[test]
fn corners_are_what_the_function_read_from_quartzcore_draws() {
    let _gpu_fixture = gpu_fixture();
    for scale in [2., 3.] {
        for (w, h, r) in [
            (100., 100., 16.),
            (100., 100., 40.),
            (100., 100., 50.),
            (64., 44., 16.),
            (60., 44., 21.5),
            (200., 52., 26.),
            (300., 60., 12.),
        ] {
            let (mut gpu, mut scene, width) = window((320., 120.), scale);
            add(
                &mut scene,
                box_style(r, CornerCurve::Continuous),
                (4., 4.),
                (w, h),
            );
            let drawn = draw(&mut gpu, &mut scene, width);
            let worst = worst_level(&drawn, 4., (w as f64, h as f64), r as f64, scale as f64);
            assert!(
                worst <= 2.5,
                "{w}×{h} r{r} at {scale}x: {worst} levels from the law"
            );
        }
    }
}

/// Against the real thing: `CALayer`'s own pixels at 2x and 3x
/// (`continuous_corners.calayer`), fills and a border alike.
#[test]
fn corners_are_within_levels_of_calayers_own_pixels() {
    let _gpu_fixture = gpu_fixture();
    for line in include_str!("continuous_corners.calayer").lines() {
        if line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split(" | ").collect();
        let numbers = |part: &str| -> Vec<f64> {
            part.split_whitespace()
                .map(|number| number.parse().unwrap())
                .collect()
        };
        let [scale, w, h, r, b] = numbers(parts[0])[..] else {
            panic!("{line}")
        };
        let [columns, rows] = numbers(parts[1])[..] else {
            panic!("{line}")
        };
        for direct in [true, false] {
            let (mut gpu, mut scene, width) = window((320., 120.), scale as f32);
            gpu.set_direct_metal(direct);
            let style = match b > 0. {
                true => QuadStyle {
                    fill: Color(0, 0, 0, 0),
                    border_width: b as f32,
                    border_color: INK,
                    ..box_style(r as f32, CornerCurve::Continuous)
                },
                false => box_style(r as f32, CornerCurve::Continuous),
            };
            add(&mut scene, style, (4., 4.), (w as f32, h as f32));
            let drawn = draw(&mut gpu, &mut scene, width);
            let origin = (4. * scale) as u32;
            let mut listed = std::collections::HashMap::new();
            for pixel in parts[2].split_whitespace() {
                let values: Vec<u32> = pixel.split(',').map(|v| v.parse().unwrap()).collect();
                listed.insert((values[0], values[1]), values[2] as f64);
            }
            let mut worst: f64 = 0.;
            for row in 0..rows as u32 {
                for column in 0..columns as u32 {
                    let got = drawn.alpha(origin + column, origin + row) * 255.;
                    match listed.get(&(column, row)) {
                        Some(want) => worst = worst.max((got - want).abs()),
                        // Whole or empty there: it says which by the law's side of it,
                        // so a pixel this draws faintly on either side still counts.
                        None => worst = worst.max(got.min(255. - got)),
                    }
                }
            }
            assert!(
                worst <= 3.,
                "{w}×{h} r{r} border {b} at {scale}x, direct {direct}: {worst} levels from CALayer's"
            );
        }
    }
}

/// A pixel the circular corner covers whole and the continuous one misses.
fn probe(size: f64, radius: f64, scale: f64) -> (u32, u32) {
    let mut best = (0, 0, f64::MIN);
    for row in 0..(radius * scale) as u32 {
        for column in 0..(radius * scale) as u32 {
            let (x, y) = ((column as f64 + 0.5) / scale, (row as f64 + 0.5) / scale);
            let (qx, qy) = (radius - x, radius - y);
            let circle = qx.hypot(qy) - radius;
            let outside = field_at(x, y, (size, size), radius) * radius * REACH;
            let margin = outside.min(-circle);
            if margin > best.2 {
                best = (column, row, margin);
            }
        }
    }
    assert!(best.2 * scale > 0.6, "no pixel tells them apart: {best:?}");
    (best.0, best.1)
}
const R: f32 = 80.;
const SIDE: f32 = 240.;
const AT: f32 = 8.;
/// A box drawn with each curve through `path`, which may change the scene and
/// the renderer: the pixel between the curves tells which it drew.
fn tells_curves_apart(
    what: &str,
    mut path: impl FnMut(&mut GpuRenderer, &mut Scene, NodeId) -> Drawn,
) {
    let scale = 2.;
    let (column, row) = probe(SIDE as f64, R as f64, scale as f64);
    let (x, y) = ((AT * scale) as u32 + column, (AT * scale) as u32 + row);
    for direct in [true, false] {
        for curve in [CornerCurve::Circular, CornerCurve::Continuous] {
            let (mut gpu, mut scene, _) = window((SIDE + 2. * AT, SIDE + 2. * AT), scale);
            gpu.set_direct_metal(direct);
            let id = add(&mut scene, box_style(R, curve), (AT, AT), (SIDE, SIDE));
            let drawn = path(&mut gpu, &mut scene, id);
            let alpha = drawn.at(x, y)[3];
            match curve {
                CornerCurve::Circular => {
                    assert!(alpha > 0, "{what}, direct {direct}: the circle missed")
                }
                CornerCurve::Continuous => {
                    assert_eq!(alpha, 0, "{what}, direct {direct}: drew the circle")
                }
            }
        }
    }
}
fn width_of(_: &Scene) -> u32 {
    ((SIDE + 2. * AT) * 2.) as u32
}

#[test]
fn the_curve_survives_every_path_a_box_can_take() {
    let _gpu_fixture = gpu_fixture();
    tells_curves_apart("plain", |gpu, scene, _| {
        let width = width_of(scene);
        draw(gpu, scene, width)
    });
    // A blended draw, whole: the opaque one splits into bands and an interior.
    tells_curves_apart("translucent", |gpu, scene, id| {
        scene.set_effects(
            id,
            Effects {
                opacity: 0.9,
                ..Default::default()
            },
        );
        let width = width_of(scene);
        draw(gpu, scene, width)
    });
    tells_curves_apart("every quad full, unsplit", |gpu, scene, _| {
        gpu.debug_split_shading(false);
        let width = width_of(scene);
        draw(gpu, scene, width)
    });
    tells_curves_apart("in its layer", |gpu, scene, id| {
        scene.set_isolated(id, true);
        let width = width_of(scene);
        draw(gpu, scene, width)
    });
    tells_curves_apart("over a backdrop blur", |gpu, scene, id| {
        scene.set_effects(
            id,
            Effects {
                blur_radius: 6.,
                ..Default::default()
            },
        );
        let width = width_of(scene);
        draw(gpu, scene, width)
    });
    tells_curves_apart("fading itself", |gpu, scene, id| {
        scene.set_effects(
            id,
            Effects {
                edge_fade: 60.,
                ..Default::default()
            },
        );
        let width = width_of(scene);
        draw(gpu, scene, width)
    });
    for bands in [false, true] {
        tells_curves_apart("under a fade", |gpu, scene, id| {
            gpu.set_fade_bands(bands);
            let root = scene.root();
            scene.set_style(
                root,
                Style {
                    clip: true,
                    fade_edges: [120., 0.],
                    ..scene.style(root)
                },
            );
            // Something under it too: the mask fades them as one.
            let under = scene.append(root, NodeKind::Rect(Color(0, 0, 0, 0)), fixed(10., 10.));
            let _ = (id, under);
            let width = width_of(scene);
            draw(gpu, scene, width)
        });
    }
    tells_curves_apart("moved in a kept frame", |gpu, scene, id| {
        let width = width_of(scene);
        scene.set_transform(id, Transform { x: AT + 3., y: AT });
        draw(gpu, scene, width);
        scene.set_transform(id, Transform { x: AT, y: AT });
        draw(gpu, scene, width)
    });
    tells_curves_apart("a frame after its fill changed", |gpu, scene, id| {
        let width = width_of(scene);
        draw(gpu, scene, width);
        let NodeKind::Panel { quad, layout } = scene.kind(id).clone() else {
            unreachable!()
        };
        scene.set_kind(
            id,
            NodeKind::Panel {
                layout,
                quad: QuadStyle {
                    fill: Color(0, 0, 1, 255),
                    ..quad
                },
            },
        );
        draw(gpu, scene, width)
    });
}

/// The same box drawn whole (unsplit, one blended quad) and in pieces (`push_panel`).
#[test]
fn a_split_panel_draws_the_curve_at_one_and_a_half_radii_as_the_whole_box_does() {
    let _gpu_fixture = gpu_fixture();
    for (size, radius) in [
        ((240., 240.), 80.),
        ((400., 100.), 40.),
        ((400., 300.), 20.),
    ] {
        let frame = |split: bool| {
            let (mut gpu, mut scene, width) = window((420., 320.), 2.);
            gpu.debug_split_shading(split);
            add(
                &mut scene,
                box_style(radius, CornerCurve::Continuous),
                (8., 8.),
                size,
            );
            draw(&mut gpu, &mut scene, width).pixels
        };
        assert!(
            frame(true) == frame(false),
            "{size:?} r{radius}: its pieces draw otherwise than the whole box"
        );
    }
}

/// Quads with circular corners, drawn by the variant's module when a batch
/// merges them into it, are the bytes they are without it.
#[test]
fn circular_boxes_beside_a_continuous_one_draw_what_they_draw_alone() {
    let _gpu_fixture = gpu_fixture();
    let frame = |with: bool| {
        let (mut gpu, mut scene, width) = window((300., 140.), 2.);
        add(
            &mut scene,
            box_style(14., CornerCurve::Circular),
            (6., 6.),
            (40., 30.),
        );
        add(
            &mut scene,
            QuadStyle {
                fill: Color(10, 200, 90, 180),
                radius: 9.,
                border_width: 2.,
                border_color: Color(200, 30, 30, 255),
                shadow: Some(shadow(Color(0, 0, 0, 120), 4.)),
                ..Default::default()
            },
            (60., 6.),
            (50., 40.),
        );
        if with {
            add(
                &mut scene,
                box_style(20., CornerCurve::Continuous),
                (180., 6.),
                (100., 100.),
            );
        }
        let pixels = draw(&mut gpu, &mut scene, width).pixels;
        pixels[..(170 * 2 * 4 * 280)]
            .chunks(width as usize * 4)
            .flat_map(|row| row[..170 * 2 * 4].to_vec())
            .collect::<Vec<u8>>()
    };
    assert!(frame(true) == frame(false));
}

/// Both of a Mac's paths draw the same bytes, as they do circular corners, and a
/// frame of them after a change equals a full repaint.
#[test]
fn direct_metal_draws_what_wgpu_draws() {
    let _gpu_fixture = gpu_fixture();
    let scene_of = |gpu: &mut GpuRenderer, direct: bool| {
        gpu.set_direct_metal(direct);
        let mut scene = Scene::new(320., 140.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        add(
            &mut scene,
            box_style(16., CornerCurve::Continuous),
            (6., 6.),
            (64., 44.),
        );
        add(
            &mut scene,
            QuadStyle {
                fill: Color(30, 90, 200, 200),
                border_width: 2.,
                border_color: Color(240, 240, 240, 255),
                shadow: Some(shadow(Color(0, 0, 0, 140), 5.)),
                ..box_style(21.5, CornerCurve::Continuous)
            },
            (90., 10.),
            (100., 70.),
        );
        add(
            &mut scene,
            box_style(9., CornerCurve::Circular),
            (210., 10.),
            (40., 40.),
        );
        scene
    };
    for scale in [2., 3.] {
        let (width, height) = ((320. * scale) as u32, (140. * scale) as u32);
        let mut frames = Vec::new();
        for direct in [true, false] {
            let mut gpu = GpuRenderer::new(width, height).unwrap();
            gpu.set_scale_factor(scale);
            let mut scene = scene_of(&mut gpu, direct);
            let damage = scene.flush().damage;
            gpu.render(&scene, &damage).unwrap();
            let first = gpu.readback().unwrap();
            gpu.render(&scene, &[Rect::new(0., 0., 320., 140.)])
                .unwrap();
            assert!(
                first == gpu.readback().unwrap(),
                "direct {direct} repainted"
            );
            if cfg!(target_os = "macos") && direct {
                assert!(gpu.debug_metal_frames() > 0, "drawn by wgpu instead");
            }
            frames.push(first);
        }
        let differ = frames[0]
            .iter()
            .zip(&frames[1])
            .map(|(a, b)| a.abs_diff(*b))
            .max();
        assert!(
            frames[0] == frames[1],
            "at {scale}x the paths differ by {differ:?}"
        );
    }
}

/// A border is the box's curve less the curve at its radius less the border's
/// width, on the box inset by it, as `stroke_round_rect` binds them; a shadow's
/// corner moves with the box's.
#[test]
fn a_border_and_a_sharp_shadow_follow_the_curve() {
    let _gpu_fixture = gpu_fixture();
    let scale = 3.;
    let (w, h, r, b): (f32, f32, f32, f32) = (100., 100., 30., 2.);
    let (mut gpu, mut scene, width) = window((120., 120.), scale);
    let style = QuadStyle {
        fill: Color(0, 0, 0, 0),
        border_width: b,
        border_color: INK,
        ..box_style(r, CornerCurve::Continuous)
    };
    add(&mut scene, style, (4., 4.), (w, h));
    let drawn = draw(&mut gpu, &mut scene, width);
    let origin = (4. * scale) as u32;
    let inset = (b * scale) as i64;
    let mut worst: f64 = 0.;
    for row in 0..(r * 1.6 * scale) as u32 {
        for column in 0..(r * 1.6 * scale) as u32 {
            let outer = coverage(column, row, (w as f64, h as f64), r as f64, scale as f64);
            let (c, ro) = (column as i64 - inset, row as i64 - inset);
            let inner = match c >= 0 && ro >= 0 {
                true => coverage(
                    c as u32,
                    ro as u32,
                    ((w - 2. * b) as f64, (h - 2. * b) as f64),
                    (r - b) as f64,
                    scale as f64,
                ),
                false => 0.,
            };
            let got = drawn.alpha(origin + column, origin + row);
            worst = worst.max(((outer - inner).max(0.) - got).abs() * 255.);
        }
    }
    assert!(worst <= 3., "the ring is {worst} levels from the law");
    // A sharp shadow's corner moves with the box's.
    let (w, r) = (200., 60.);
    let shadowed = |curve| {
        let (mut gpu, mut scene, width) = window((220., 220.), scale);
        let style = QuadStyle {
            shadow: Some(shadow(INK, 0.01)),
            fill: Color(0, 0, 0, 0),
            ..box_style(r, curve)
        };
        add(&mut scene, style, (4., 4.), (w, w));
        draw(&mut gpu, &mut scene, width)
    };
    let (circle, continuous) = (
        shadowed(CornerCurve::Circular),
        shadowed(CornerCurve::Continuous),
    );
    let (column, row) = probe(w as f64, r as f64, scale as f64);
    let (x, y) = ((4. * scale) as u32 + column, (4. * scale) as u32 + row);
    assert!(circle.at(x, y)[3] > 150);
    assert!(continuous.at(x, y)[3] < 30);
}

/// The field's gradient grows without bound towards the middle of a corner,
/// where the curve's angle is all that changes: a blurred shadow, reading a
/// distance from them, stays whole under a clear box there.
#[test]
fn a_blurred_shadow_is_whole_across_the_middle_of_a_corner() {
    let _gpu_fixture = gpu_fixture();
    let scale = 2.;
    let (mut gpu, mut scene, width) = window((240., 240.), scale);
    let style = QuadStyle {
        shadow: Some(shadow(INK, 3.)),
        fill: Color(0, 0, 0, 0),
        ..box_style(40., CornerCurve::Continuous)
    };
    add(&mut scene, style, (10., 10.), (200., 200.));
    let drawn = draw(&mut gpu, &mut scene, width);
    // From where the corner's straight parts end, in every direction towards the
    // corner: the angles between the diagonal and the sides are the ones it shows at.
    let middle = ((10. + 40. * REACH) * scale as f64) as u32;
    for across in 0..24 {
        for down in 0..24 {
            let (x, y) = (middle - across, middle - down);
            let alpha = drawn.alpha(x, y);
            assert!(alpha > 0.99, "{across} and {down} in: {alpha}");
        }
    }
}

/// What can't draw on the quad path rasterizes: to the same curve.
#[test]
fn a_rasterized_box_has_the_curve_too() {
    let _gpu_fixture = gpu_fixture();
    let scale = 2.;
    let (w, h, r) = (120., 60., 24.);
    let (mut gpu, mut scene, width) = window((140., 80.), scale);
    let stops: Arc<[zgx::canvas::GradientStop]> = Arc::from(vec![
        zgx::canvas::GradientStop {
            offset: 0.,
            color: INK,
        },
        zgx::canvas::GradientStop {
            offset: 1.,
            color: INK,
        },
    ]);
    let style = QuadStyle {
        decoration: Some(Arc::new(Decoration {
            corners: Some(Corners::all(r)),
            corner_curve: CornerCurve::Continuous,
            background: Some(Background::Linear { angle: 0.7, stops }),
            ..Default::default()
        })),
        fill: INK,
        radius: r,
        ..Default::default()
    };
    add(&mut scene, style, (4., 4.), (w, h));
    let drawn = draw(&mut gpu, &mut scene, width);
    // Its own antialiasing, not the quads': the same area, on the same edge. To
    // the box's middle: past it, the next corner's.
    let origin = (4. * scale) as u32;
    let size = (w as f64, h as f64);
    let (columns, rows) = ((w / 2. * scale) as u32, (h / 2. * scale) as u32);
    let (mut area, mut worst) = (0_f64, 0_f64);
    for row in 0..rows {
        for column in 0..columns {
            let want = coverage(column, row, size, r as f64, scale as f64);
            let got = drawn.alpha(origin + column, origin + row);
            area += got - want;
            worst = worst.max((got - want).abs());
        }
    }
    let shift = area / (r as f64 * 1.6 * scale as f64);
    assert!(
        shift.abs() <= 0.03,
        "the raster's edge is {shift} px from the law's"
    );
    assert!(
        worst <= 0.3,
        "a raster pixel is {worst} of coverage from the law's"
    );
}

/// Scrolled by copying the pixels drawn, continuous boxes in the content come
/// out as they do drawn where they went.
#[test]
fn scroll_copies_of_continuous_corners_match_a_fresh_render() {
    let _gpu_fixture = gpu_fixture();
    let view = |offset: zgx::reactive::Signal<f32>| {
        virtual_list(
            offset,
            48.,
            2,
            || 1000,
            |i| i,
            |_, key, _| {
                div().size(144., 48.).child(
                    div()
                        .size(120., 40.)
                        .rounded(18.)
                        .corner_curve(CornerCurve::Continuous)
                        .bg(if key % 2 == 0 {
                            rgb(0x223344)
                        } else {
                            rgb(0x88aa55)
                        }),
                )
            },
        )
        .w(160.)
        .h(100.)
        .bg(rgb(0x101820))
    };
    let mut gpu = GpuRenderer::new(160, 120).unwrap();
    let mut ui = Ui::new(160., 120.);
    let offset = ui.signal(0_f32);
    let root = overlay()
        .w(160.)
        .h(120.)
        .bg(rgb(0))
        .child(view(offset.clone()));
    ui.mount(root);
    for step in 1..6 {
        offset.set(step as f32 * 7.);
        ui.prepare_frame();
        let mut scene = ui.scene.borrow_mut();
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let scrolled = gpu.readback().unwrap();
        let mut fresh = GpuRenderer::new(160, 120).unwrap();
        fresh
            .render(&scene, &[Rect::new(0., 0., 160., 120.)])
            .unwrap();
        assert!(
            scrolled == fresh.readback().unwrap(),
            "step {step}: the copy differs from drawing it there"
        );
    }
}

/// Hit testing never read the curve: a press lands where it does on any box.
#[test]
fn hit_testing_is_the_same_on_either_curve() {
    let mut hits = Vec::new();
    for curve in [CornerCurve::Circular, CornerCurve::Continuous] {
        let (_, mut scene, _) = window((120., 120.), 1.);
        let id = add(&mut scene, box_style(30., curve), (10., 10.), (100., 100.));
        scene.flush();
        let hit: Vec<bool> = (0..60)
            .flat_map(|y| (0..60).map(move |x| (x, y)))
            .map(|(x, y)| scene.hit_test(x as f32 + 0.5, y as f32 + 0.5) == Some(id))
            .collect();
        hits.push(hit);
    }
    assert!(hits[0] == hits[1]);
}
