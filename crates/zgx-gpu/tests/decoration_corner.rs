//! A decorated box (one with shadows, so its fill and its ring are drawn as
//! quads apart) takes the app's corner shape (`set_corner_shape`, CSS's
//! `corner-shape: superellipse(k)`) in its fill as in its ring and shadows:
//! no band between the ring's inner edge and a round fill shows the shadow
//! or what is under the box. A dark popover's corners showed one, the
//! scrim through a dialog's.
use std::sync::Arc;
use zgx::decoration::{Corners, Decoration};
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

const SIZE: u32 = 200;
const SCALE: u32 = 2;
const RADIUS: f32 = 15.;
const BACKDROP: Color = Color(0x14, 0x14, 0x14, 255);
const FILL: Color = Color(0x1f, 0x1f, 0x1f, 255);
/// A dark popover's faint warm edge (6%).
const EDGE: Color = Color(0xf7, 0xea, 0xd4, 0x0f);

fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}

/// The darkest pixel (grey, x, y) a box at (`at`, `at`), 120 by 80, rounded
/// `RADIUS` with corners of shape `k`, covers whole: each pixel whose centre
/// is a device pixel or more inside its outline (the superellipse
/// `|u|^n + |v|^n = 1`, `n = 2^k`, a pixel in), around its top-left corner.
fn darkest_inside(direct: bool, at: f32, k: f32) -> (u8, u32, u32) {
    zgx::style::set_corner_shape(k);
    let mut scene = Scene::new(SIZE as f32, SIZE as f32);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    scene.append(
        root,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad: QuadStyle {
                fill: BACKDROP,
                ..Default::default()
            },
        },
        fixed(SIZE as f32, SIZE as f32),
    );
    // A dark popover: its fill, its faint edge and its shadows (CSS's
    // `0 1 1 0 #00000033, 0 4 8 -4 #0000004d, 0 16 24 -8 #00000066`, a
    // blur of `b` a sigma of `b / 2`), its radius the style's own.
    let quad = QuadStyle {
        fill: FILL,
        radius: RADIUS,
        border_color: EDGE,
        border_width: 1.,
        decoration: Some(Arc::new(Decoration {
            corners: Some(Corners::all(RADIUS)),
            shadows: Some(Arc::from(
                [
                    (0x33, 1., 1., 0.),
                    (0x4d, 4., 8., -4.),
                    (0x66, 16., 24., -8.),
                ]
                .map(|(alpha, y, blur, spread)| BoxShadow {
                    color: Color(0, 0, 0, alpha),
                    offset: Transform { x: 0., y },
                    blur_radius: blur / 2.,
                    spread,
                }),
            )),
            ..Default::default()
        })),
        ..Default::default()
    };
    let id = scene.append(
        root,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad,
        },
        fixed(120., 80.),
    );
    scene.set_transform(id, Transform { x: at, y: at });
    scene.flush();
    let mut gpu = GpuRenderer::new(SIZE * SCALE, SIZE * SCALE).unwrap();
    gpu.set_scale_factor(SCALE as f32);
    gpu.set_direct_metal(direct);
    gpu.render(&scene, &[Rect::new(0., 0., SIZE as f32, SIZE as f32)])
        .unwrap();
    let pixels = gpu.readback().unwrap();
    let grey = |x: u32, y: u32| pixels[((y * SIZE * SCALE + x) * 4) as usize];
    let scale = SCALE as f32;
    let (left, top) = (at * scale, at * scale);
    // A pixel in from the outline: the corner's superellipse a pixel
    // smaller, about the same centre; the straight edges a pixel in.
    let r = RADIUS * scale;
    let n = k.exp2();
    let inside = |x: u32, y: u32| {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let (cx, cy) = (left + r, top + r);
        if px < cx && py < cy {
            let (u, v) = ((cx - px) / (r - 1.), (cy - py) / (r - 1.));
            u.powf(n) + v.powf(n) <= 1.
        } else {
            px >= left + 1. && py >= top + 1.
        }
    };
    let origin = (at * scale) as u32;
    let mut worst: Option<(u8, u32, u32)> = None;
    for y in origin..origin + 80 {
        for x in origin..origin + 80 {
            if inside(x, y) {
                let value = grey(x, y);
                if worst.is_none_or(|(w, _, _)| value < w) {
                    worst = Some((value, x, y));
                }
            }
        }
    }
    worst.expect("the box draws")
}

/// Its corners superellipses (a continuous corner, and a squarer one) or
/// round: nothing inside its outline is darker than its fill.
#[test]
fn a_shadowed_boxs_fill_takes_its_rings_corner_shape() {
    for k in [1., 1.5, 2.] {
        for direct in [true, false] {
            for at in [13., 40., 40.5] {
                let (darkest, x, y) = darkest_inside(direct, at, k);
                assert!(
                    darkest >= FILL.0,
                    "corner shape {k}, direct {direct}, at {at}: inside its outline, \
                     ({x}, {y}) is {darkest}, darker than the fill ({})",
                    FILL.0
                );
            }
        }
    }
    zgx::style::set_corner_shape(1.);
}
