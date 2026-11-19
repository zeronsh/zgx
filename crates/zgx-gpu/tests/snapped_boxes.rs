//! A box laid out between device pixels paints its edges on them, as Blink
//! snaps a layout box (`Scene::snapped_bounds`): a dialog centred at a quarter
//! point draws its edges and a 1 pt border on whole device pixels, at 1x
//! and 2x, settled after an open animation or with none. What a
//! translation moves between pixels (an animation in flight) and what a
//! transform holds keep their places between pixels. A press on an edge
//! pixel hits the box painted there.
use std::sync::Arc;
use zgx::decoration::Decoration;
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
const W: f32 = 160.;
const H: f32 = 120.;
const BG: Color = Color(20, 20, 30, 255);
const FILL: Color = Color(240, 240, 245, 255);
const LINE: Color = Color(200, 60, 40, 255);
/// Where layout puts the panel: a quarter point off the grid each way.
const AT: (f32, f32) = (30.25, 20.25);
const SIZE: (f32, f32) = (100.5, 60.25);

/// A window of `BG` holding a panel of `FILL` with a 1 pt `LINE` border,
/// put at `AT` by its parent's padding (as centring puts a dialog), with
/// the window's pixel scale.
fn dialog(scale: f32) -> (Scene, NodeId) {
    let mut scene = Scene::new(W, H);
    scene.set_pixel_scale(scale);
    let root = scene.root();
    scene.set_kind(
        root,
        NodeKind::Panel {
            layout: Layout::Column,
            quad: QuadStyle {
                fill: BG,
                ..Default::default()
            },
        },
    );
    let mut style = scene.style(root);
    style.padding_edges = Some(Insets {
        top: AT.1,
        right: 0.,
        bottom: 0.,
        left: AT.0,
    });
    scene.set_style(root, style);
    let quad = QuadStyle {
        fill: FILL,
        border_color: LINE,
        decoration: Some(Arc::new(Decoration {
            border_widths: Some(Insets {
                top: 1.,
                right: 1.,
                bottom: 1.,
                left: 1.,
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    let panel = scene.append(
        root,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad,
        },
        Style {
            width: Some(SIZE.0),
            height: Some(SIZE.1),
            ..Default::default()
        },
    );
    (scene, panel)
}
fn draw(scene: &mut Scene, scale: f32, direct: bool) -> (Vec<u8>, usize) {
    let (w, h) = ((W * scale) as u32, (H * scale) as u32);
    let mut gpu = GpuRenderer::new(w, h).unwrap();
    gpu.set_scale_factor(scale);
    gpu.set_direct_metal(direct);
    scene.prepare_layout();
    scene.flush();
    gpu.render(scene, &[Rect::new(0., 0., W, H)]).unwrap();
    (gpu.readback().unwrap(), w as usize)
}
fn near(p: &[u8], c: Color) -> bool {
    [c.0, c.1, c.2]
        .iter()
        .zip(p)
        .all(|(a, b)| a.abs_diff(*b) <= 2)
}
/// Pixels of the column and row through the panel's middle that are none of
/// background, border and fill: an edge drawn between pixels blends them.
fn blended(pixels: &[u8], width: usize, scale: f32) -> usize {
    let height = pixels.len() / 4 / width;
    let x = ((AT.0 + SIZE.0 / 2.) * scale) as usize;
    let y = ((AT.1 + SIZE.1 / 2.) * scale) as usize;
    let at = |x: usize, y: usize| &pixels[(y * width + x) * 4..(y * width + x) * 4 + 4];
    let odd = |p: &[u8]| !(near(p, BG) || near(p, FILL) || near(p, LINE));
    (0..height).filter(|&y| odd(at(x, y))).count() + (0..width).filter(|&x| odd(at(x, y))).count()
}
/// The first device row of the column through the panel that its border
/// paints.
fn top_row(pixels: &[u8], width: usize, scale: f32) -> usize {
    let x = ((AT.0 + SIZE.0 / 2.) * scale) as usize;
    (0..pixels.len() / 4 / width)
        .position(|y| near(&pixels[(y * width + x) * 4..], LINE))
        .expect("the border draws")
}

#[test]
fn a_box_laid_out_between_pixels_paints_its_edges_and_border_on_them() {
    let _gpu_fixture = gpu_fixture();
    for scale in [1., 2.] {
        for direct in [true, false] {
            let (mut scene, _) = dialog(scale);
            let (pixels, width) = draw(&mut scene, scale, direct);
            assert_eq!(
                blended(&pixels, width, scale),
                0,
                "scale {scale}, direct {direct}"
            );
            // Its top edge where the pixels whose centres it holds begin.
            let want = (AT.1 * scale - 0.5).ceil() as usize;
            assert_eq!(top_row(&pixels, width, scale), want, "scale {scale}");
        }
    }
}

#[test]
fn a_box_settled_after_an_open_animation_paints_on_pixels() {
    let _gpu_fixture = gpu_fixture();
    for scale in [1., 2.] {
        let (mut scene, panel) = dialog(scale);
        // Opening: scaled up from 95% (a layer, between pixels), then settled.
        scene.set_local_transform(
            panel,
            LocalTransform {
                scale: (0.95, 0.95),
                ..LocalTransform::IDENTITY
            },
        );
        let (pixels, width) = draw(&mut scene, scale, true);
        assert!(
            blended(&pixels, width, scale) > 0,
            "scale {scale}: mid-animation"
        );
        scene.set_local_transform(panel, LocalTransform::IDENTITY);
        let (pixels, width) = draw(&mut scene, scale, true);
        assert_eq!(blended(&pixels, width, scale), 0, "scale {scale}: settled");
    }
}

#[test]
fn what_moves_or_turns_between_pixels_keeps_its_place() {
    let _gpu_fixture = gpu_fixture();
    for scale in [1., 2.] {
        // A translation in flight, a third of a point: the snapped box moves
        // smoothly with it.
        let (mut scene, panel) = dialog(scale);
        scene.set_transform(panel, Transform { x: 0., y: 0.3 });
        let (pixels, width) = draw(&mut scene, scale, true);
        assert!(blended(&pixels, width, scale) > 0, "scale {scale}: moving");
        // A turned mark: drawn in its own space, as it is.
        let (mut scene, panel) = dialog(scale);
        scene.set_local_transform(
            panel,
            LocalTransform {
                rotate: 0.05,
                ..LocalTransform::IDENTITY
            },
        );
        let (pixels, width) = draw(&mut scene, scale, true);
        assert!(blended(&pixels, width, scale) > 0, "scale {scale}: turned");
    }
}

#[test]
fn a_press_on_an_edge_pixel_hits_the_box_painted_there() {
    let _gpu_fixture = gpu_fixture();
    for scale in [1., 2.] {
        let (mut scene, panel) = dialog(scale);
        let (pixels, width) = draw(&mut scene, scale, true);
        let row = top_row(&pixels, width, scale);
        let x = AT.0 + SIZE.0 / 2.;
        // Each row's centre: the first the border paints hits the panel, the
        // one above it doesn't.
        let centre = |row: usize| (row as f32 + 0.5) / scale;
        assert_eq!(scene.hit_test(x, centre(row)), Some(panel), "scale {scale}");
        assert_ne!(
            scene.hit_test(x, centre(row - 1)),
            Some(panel),
            "scale {scale}"
        );
        // And the left edge, by its column.
        let y = ((AT.1 + SIZE.1 / 2.) * scale) as usize;
        let column = (0..width)
            .position(|x| near(&pixels[(y * width + x) * 4..], LINE))
            .expect("the border draws");
        let middle = AT.1 + SIZE.1 / 2.;
        assert_eq!(
            scene.hit_test(centre(column), middle),
            Some(panel),
            "scale {scale}"
        );
        assert_ne!(
            scene.hit_test(centre(column - 1), middle),
            Some(panel),
            "scale {scale}"
        );
    }
}

/// A hairline wholly outside its clip, beside its edge, paints nothing, as
/// laid out: snapped, its one pixel would fall inside the clip's first
/// (Blink keeps a box's pixel each way). Alike in a whole repaint, whose
/// walk leaves it out by its clip, and a damaged one, whose walk keeps it.
#[test]
fn a_hairline_beside_its_clip_paints_nothing() {
    let _gpu_fixture = gpu_fixture();
    let mut scene = Scene::new(W, H);
    scene.set_pixel_scale(1.);
    let root = scene.root();
    scene.set_kind(
        root,
        NodeKind::Panel {
            layout: Layout::Column,
            quad: QuadStyle {
                fill: BG,
                ..Default::default()
            },
        },
    );
    let mut style = scene.style(root);
    style.padding_edges = Some(Insets {
        top: 10.,
        right: 0.,
        bottom: 0.,
        left: 10.2,
    });
    scene.set_style(root, style);
    let clipping = scene.append(
        root,
        NodeKind::Panel {
            layout: Layout::Column,
            quad: QuadStyle {
                fill: FILL,
                ..Default::default()
            },
        },
        Style {
            width: Some(40.),
            height: Some(40.),
            clip: true,
            ..Default::default()
        },
    );
    // Laid out at 10.2, 0.2 wide, moved back to 10.0: wholly left of the
    // clip, which starts at 10.2.
    let hairline = scene.append(
        clipping,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad: QuadStyle {
                fill: LINE,
                ..Default::default()
            },
        },
        Style {
            width: Some(0.2),
            height: Some(20.),
            ..Default::default()
        },
    );
    scene.set_transform(hairline, Transform { x: -0.2, y: 0. });
    let (w, h) = (W as u32, H as u32);
    let mut gpu = GpuRenderer::new(w, h).unwrap();
    gpu.set_scale_factor(1.);
    scene.prepare_layout();
    scene.flush();
    // No red over grey in the columns around the clip's edge.
    let reddened = |pixels: &[u8]| {
        (9..12)
            .flat_map(|x| (12..28).map(move |y| (x, y)))
            .filter(|&(x, y)| {
                let p = &pixels[(y * w as usize + x) * 4..];
                p[0] as i32 - p[1] as i32 > 15
            })
            .count()
    };
    gpu.render(&scene, &[Rect::new(0., 0., W, H)]).unwrap();
    assert_eq!(reddened(&gpu.readback().unwrap()), 0, "a whole repaint");
    gpu.render(&scene, &[Rect::new(5., 5., 20., 40.)]).unwrap();
    assert_eq!(reddened(&gpu.readback().unwrap()), 0, "a damaged repaint");
}
