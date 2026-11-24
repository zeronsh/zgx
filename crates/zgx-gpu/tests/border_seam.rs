//! A translucent border over its box's fill, as CSS paints one: the
//! background under the border (`background-clip: border-box`), the border
//! over it. A pixel the box covers whole shows the fill, with the border
//! over it where the ring is; never what is under the box (the backdrop, or
//! the box's own shadow) through the ring. Drawn otherwise, a dark popover
//! showed a seam between its faint edge and its background along its
//! corners, and a light box on a dark backdrop a dark ring.
use std::sync::Arc;
use zgx::decoration::{Corners, Decoration};
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
const SIZE: u32 = 200;
const SCALE: u32 = 2;
/// A faint warm edge (6% white): a dark popover's.
const EDGE: Color = Color(0xf7, 0xea, 0xd4, 0x0f);

fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}

/// `backdrop`, and over it a box at (`at`, `at`) 120 by 80, rounded 15,
/// filled with `fill` and bordered 1 in `EDGE`: its radius and border the
/// style's own (one quad draws fill and ring), or a decoration's
/// (`decorated`: the fill drawn whole, its ring over it).
fn scene(at: f32, backdrop: Color, fill: Color, decorated: bool) -> Scene {
    let mut scene = Scene::new(SIZE as f32, SIZE as f32);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    scene.append(
        root,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad: QuadStyle {
                fill: backdrop,
                ..Default::default()
            },
        },
        fixed(SIZE as f32, SIZE as f32),
    );
    let quad = if decorated {
        QuadStyle {
            fill,
            border_color: EDGE,
            decoration: Some(Arc::new(Decoration {
                corners: Some(Corners::all(15.)),
                border_widths: Some(Insets::all(1.)),
                ..Default::default()
            })),
            ..Default::default()
        }
    } else {
        QuadStyle {
            fill,
            radius: 15.,
            border_color: EDGE,
            border_width: 1.,
            ..Default::default()
        }
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
    scene
}

/// The darkest pixel (its grey, x, y) the box at (`at`, `at`) covers
/// whole, drawn over `backdrop`: around its top-left corner and along its
/// top and left edges, each pixel whose centre is a device pixel or more
/// inside its rounded outline (so none of it is outside the box).
fn darkest_inside(
    direct: bool,
    at: f32,
    (backdrop, fill): (Color, Color),
    decorated: bool,
) -> (u8, u32, u32) {
    let mut gpu = GpuRenderer::new(SIZE * SCALE, SIZE * SCALE).unwrap();
    gpu.set_scale_factor(SCALE as f32);
    gpu.set_direct_metal(direct);
    let mut scene = scene(at, backdrop, fill, decorated);
    scene.flush();
    gpu.render(&scene, &[Rect::new(0., 0., SIZE as f32, SIZE as f32)])
        .unwrap();
    let pixels = gpu.readback().unwrap();
    let grey = |x: u32, y: u32| pixels[((y * SIZE * SCALE + x) * 4) as usize];
    let scale = SCALE as f32;
    let (left, top, radius) = (at * scale, at * scale, 15. * scale);
    // The rounded outline's signed distance at a pixel's centre (device
    // pixels; negative inside), near its top-left corner.
    let inside = |x: u32, y: u32| {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let (cx, cy) = (left + radius, top + radius);
        if px < cx && py < cy {
            ((px - cx).hypot(py - cy)) - radius
        } else {
            (left - px).max(top - py)
        }
    };
    let origin = (at * scale) as u32;
    let mut worst: Option<(u8, u32, u32)> = None;
    for y in origin..origin + 80 {
        for x in origin..origin + 80 {
            if inside(x, y) <= -1. {
                let value = grey(x, y);
                if worst.is_none_or(|(w, _, _)| value < w) {
                    worst = Some((value, x, y));
                }
            }
        }
    }
    let _ = backdrop;
    worst.expect("the box draws")
}

/// A light box with a faint border on a dark backdrop: inside its outer
/// edge, no pixel shows the backdrop (CSS's fill there, the border over
/// it, is the fill a little warmer). Drawn with the fill inside the ring
/// alone, the ring showed the backdrop through it, nearly black.
#[test]
fn a_faint_border_shows_its_fill_under_it_not_the_backdrop() {
    let _gpu_fixture = gpu_fixture();
    let (backdrop, fill) = (Color(0, 0, 0, 255), Color(200, 200, 200, 255));
    for (direct, decorated) in [(true, false), (false, false), (true, true), (false, true)] {
        for at in [40., 40.25, 40.5, 40.75] {
            let (darkest, x, y) = darkest_inside(direct, at, (backdrop, fill), decorated);
            assert!(
                darkest >= fill.0 - 2,
                "direct {direct}, decorated {decorated}, at {at}: inside the box's \
                 edge, ({x}, {y}) is {darkest}, the fill {}",
                fill.0
            );
        }
    }
}

/// A dark popover (its surface #1f1f1f, its faint edge) on a darker cell
/// (#141414): along its corners, nothing inside its edge is darker than its
/// fill (the seam a gallery showed, 23 against 31).
#[test]
fn a_dark_popovers_corners_show_no_seam_inside_their_edge() {
    let _gpu_fixture = gpu_fixture();
    let (backdrop, fill) = (Color(0x14, 0x14, 0x14, 255), Color(0x1f, 0x1f, 0x1f, 255));
    for (direct, decorated) in [(true, false), (false, false), (true, true), (false, true)] {
        for at in [40., 40.25, 40.5, 40.75, 13.] {
            let (darkest, x, y) = darkest_inside(direct, at, (backdrop, fill), decorated);
            assert!(
                darkest >= fill.0,
                "direct {direct}, decorated {decorated}, at {at}: inside the box's \
                 edge, ({x}, {y}) is {darkest}, darker than the fill ({})",
                fill.0
            );
        }
    }
}
