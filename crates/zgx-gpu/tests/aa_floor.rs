//! A rounded box's antialiasing at every scale: how soft its edge is follows
//! its slope a device pixel, at least `least` points (`draw.wgsl`): half a
//! point at 2x and below, where the slopes are at least that, so nothing there
//! changes; one device pixel above, so a side on whole pixels shows no grey
//! column, as a `CALayer`'s doesn't.
use zgx::scene::{Color, Layout, NodeKind, QuadStyle, Scene, Style, Transform};
use zgx_gpu::GpuRenderer;

const W: u32 = 300;
const H: u32 = 240;
const R: f32 = 12.;
const SIDE: f32 = 40.;
const AT: f32 = 8.;

/// A white box of radius `R`, `AT` points in, on black, at `scale`: each
/// pixel's red, top row first.
fn rendered(scale: f32) -> Vec<u8> {
    let mut gpu = GpuRenderer::new(W, H).unwrap();
    gpu.set_scale_factor(scale);
    let (w, h) = (W as f32 / scale, H as f32 / scale);
    let sized = |w: f32, h: f32| Style {
        width: Some(w),
        height: Some(h),
        ..Style::default()
    };
    let mut scene = Scene::new(w, h);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    scene.append(
        scene.root(),
        NodeKind::Rect(Color(0, 0, 0, 255)),
        sized(w, h),
    );
    let quad = scene.append(
        scene.root(),
        NodeKind::Quad(QuadStyle {
            fill: Color(255, 255, 255, 255),
            radius: R,
            ..Default::default()
        }),
        sized(SIDE, SIDE),
    );
    scene.set_transform(quad, Transform { x: AT, y: AT });
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    gpu.readback().unwrap().chunks(4).map(|p| p[0]).collect()
}

/// What pixel (`x`, `y`) covers of that box: `draw.wgsl`'s rounded edge, the
/// least softness `least`.
fn covered(x: u32, y: u32, scale: f32, least: f32) -> f32 {
    let centre = AT + SIDE / 2.;
    let inner = SIDE / 2. - R;
    let at = |p: u32| (p as f32 + 0.5) / scale;
    let (qx, qy) = (
        (at(x) - centre).abs() - inner,
        (at(y) - centre).abs() - inner,
    );
    let (ox, oy) = (qx.max(0.), qy.max(0.));
    let reach = ox.hypot(oy);
    let distance = reach + qx.max(qy).min(0.) - R;
    let (nx, ny) = match reach > 0. {
        true => (ox / reach, oy / reach),
        false if qx > qy => (1., 0.),
        false => (0., 1.),
    };
    let aa = ((nx + ny) / scale).max(least);
    (0.5 - distance / aa).clamp(0., 1.)
}

#[test]
fn a_rounded_box_is_as_soft_as_its_scale_asks_and_no_softer() {
    // (scale, least softness): half a point at 2x and below, as ever.
    for (scale, least) in [(1., 0.5), (1.5, 0.5), (2., 0.5), (2.5, 0.4), (3., 1. / 3.)] {
        let pixels = rendered(scale);
        let mut worst = (0, 0, 0.);
        for y in 0..H {
            for x in 0..W {
                let got = pixels[(y * W + x) as usize] as f32;
                let off = (got - covered(x, y, scale, least) * 255.).abs();
                if off > worst.2 {
                    worst = (x, y, off);
                }
            }
        }
        // (The shader's distances are on a 64th of a pixel.)
        assert!(worst.2 <= 4., "at {scale}x, off by {worst:?}");
    }
}

#[test]
fn a_side_on_whole_pixels_has_no_grey_column_at_3x() {
    let pixels = rendered(3.);
    let row = ((AT + SIDE / 2.) * 3.) as u32;
    let left = (AT * 3.) as u32;
    let across: Vec<u8> = (left - 2..left + 2)
        .map(|x| pixels[(row * W + x) as usize])
        .collect();
    assert_eq!(across, [0, 0, 255, 255]);
}
