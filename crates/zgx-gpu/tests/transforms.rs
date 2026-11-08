//! Local transforms (scale, rotation, skew) composite cached layers under a
//! matrix: pixels land where the transform puts them, and transform-only
//! changes repaint no layer content.
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
fn rotated(radians: f32) -> LocalTransform {
    LocalTransform {
        rotate: radians,
        ..LocalTransform::IDENTITY
    }
}
fn scaled(factor: f32) -> LocalTransform {
    LocalTransform {
        scale: (factor, factor),
        ..LocalTransform::IDENTITY
    }
}
const SIZE: u32 = 200;
fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * SIZE + x) * 4) as usize;
    pixels[at..at + 4].try_into().unwrap()
}
fn scene() -> Scene {
    let mut scene = Scene::new(SIZE as f32, SIZE as f32);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    scene
}
/// An opaque `color` rect of `width` × `height` at (`x`, `y`).
fn rect(
    scene: &mut Scene,
    parent: NodeId,
    x: f32,
    y: f32,
    size: (f32, f32),
    color: Color,
) -> NodeId {
    let id = scene.append(parent, NodeKind::Rect(color), fixed(size.0, size.1));
    scene.set_transform(id, Transform { x, y });
    id
}
const RED: Color = Color(255, 0, 0, 255);
const BLUE: Color = Color(0, 0, 255, 255);
/// Render the flushed frame, then check it against a full repaint.
fn render(gpu: &mut GpuRenderer, scene: &mut Scene) -> (zgx_gpu::GpuStats, Vec<u8>) {
    let damage = scene.flush().damage;
    let stats = gpu.render(scene, &damage).unwrap();
    let pixels = gpu.readback().unwrap();
    gpu.render(scene, &[Rect::new(0., 0., SIZE as f32, SIZE as f32)])
        .unwrap();
    assert_eq!(
        pixels,
        gpu.readback().unwrap(),
        "incremental frame differs from full"
    );
    (stats, pixels)
}

#[test]
fn rotated_square_lands_rotated_with_smooth_edges() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
        gpu.set_direct_metal(direct);
        let mut scene = scene();
        let root = scene.root();
        let square = rect(&mut scene, root, 70., 70., (60., 60.), RED);
        render(&mut gpu, &mut scene);
        scene.set_local_transform(square, rotated(FRAC_PI_4));
        let (stats, p) = render(&mut gpu, &mut scene);
        assert_eq!(stats.layer_repaints, 1);
        assert_eq!(pixel(&p, 100, 100), [255, 0, 0, 255]);
        // The diamond reaches 42 px from its centre: above the laid-out box...
        assert_eq!(pixel(&p, 100, 62), [255, 0, 0, 255]);
        // ...but not into its corners.
        assert_eq!(pixel(&p, 73, 73), [0; 4]);
        assert_eq!(pixel(&p, 127, 127), [0; 4]);
        // Edges are antialiased rather than stair-stepped.
        let row: Vec<u8> = (50..70).map(|x| pixel(&p, x, 100)[3]).collect();
        assert!(
            row.iter().any(|a| *a > 20 && *a < 235),
            "hard edge: {row:?}"
        );
        assert_eq!(row.first(), Some(&0));
        assert_eq!(row.last(), Some(&255));
    }
}

#[test]
fn scale_and_skew_place_pixels_by_their_matrix() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let root = scene.root();
    let small = rect(&mut scene, root, 30., 30., (20., 20.), RED);
    let sheared = rect(&mut scene, root, 100., 100., (40., 40.), BLUE);
    scene.set_local_transform(small, scaled(2.));
    scene.set_local_transform(
        sheared,
        LocalTransform {
            skew: (FRAC_PI_4, 0.),
            ..LocalTransform::IDENTITY
        },
    );
    let (_, p) = render(&mut gpu, &mut scene);
    // Twice the size about its centre (40, 40): 20..60.
    assert_eq!(pixel(&p, 22, 22), [255, 0, 0, 255]);
    assert_eq!(pixel(&p, 57, 40), [255, 0, 0, 255]);
    assert_eq!(pixel(&p, 18, 40), [0; 4]);
    // Skewed about (120, 120): row y = 135 spans x 115..155.
    assert_eq!(pixel(&p, 150, 135), [0, 0, 255, 255]);
    assert_eq!(pixel(&p, 105, 135), [0; 4]);
    // Row y = 105 spans x 85..125.
    assert_eq!(pixel(&p, 88, 105), [0, 0, 255, 255]);
    assert_eq!(pixel(&p, 135, 105), [0; 4]);
}

#[test]
fn nested_transforms_compose_and_clip_inside_the_rotation() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let root = scene.root();
    let panel = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        fixed(100., 100.),
    );
    scene.set_local_transform(
        panel,
        LocalTransform {
            scale: (2., 2.),
            origin: (0., 0.),
            ..LocalTransform::IDENTITY
        },
    );
    let card = rect(&mut scene, panel, 20., 20., (20., 20.), RED);
    scene.set_local_transform(card, rotated(FRAC_PI_4));
    let (stats, p) = render(&mut gpu, &mut scene);
    assert_eq!(stats.layer_repaints, 2);
    // The card's centre (30, 30) shows at (60, 60), its diamond 28 px out.
    assert_eq!(pixel(&p, 60, 60), [255, 0, 0, 255]);
    assert_eq!(pixel(&p, 60, 36), [255, 0, 0, 255]);
    assert_eq!(pixel(&p, 42, 42), [0; 4]);
    // Rotating the inner card repaints the panel's texture, not the card's.
    scene.set_local_transform(card, rotated(0.));
    let (stats, p) = render(&mut gpu, &mut scene);
    assert_eq!((stats.layer_repaints, stats.layer_cache_hits), (1, 0));
    assert_eq!(pixel(&p, 42, 42), [255, 0, 0, 255]);

    // A clipping card rotated a quarter turn clips its overflow to its
    // rotated box.
    let mut scene = self::scene();
    let root = scene.root();
    let clip = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        Style {
            clip: true,
            ..fixed(40., 40.)
        },
    );
    scene.set_transform(clip, Transform { x: 100., y: 100. });
    rect(&mut scene, clip, 0., 0., (80., 10.), BLUE);
    scene.set_local_transform(clip, rotated(FRAC_PI_2));
    let (_, p) = render(&mut gpu, &mut scene);
    // Rotated about (120, 120): the row runs down x 130..140, y 100..140.
    assert_eq!(pixel(&p, 135, 105), [0, 0, 255, 255]);
    assert_eq!(pixel(&p, 135, 138), [0, 0, 255, 255]);
    assert_eq!(pixel(&p, 135, 150), [0; 4]);
    assert_eq!(pixel(&p, 105, 105), [0; 4]);
}

#[test]
fn group_opacity_applies_once_to_a_rotated_layer() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let root = scene.root();
    let group = scene.append(root, NodeKind::Container(Layout::Overlay), fixed(40., 40.));
    scene.set_transform(group, Transform { x: 80., y: 80. });
    rect(&mut scene, group, 0., 0., (40., 40.), RED);
    rect(&mut scene, group, 0., 0., (40., 40.), BLUE);
    scene.set_effects(
        group,
        Effects {
            opacity: 0.5,
            ..Default::default()
        },
    );
    scene.set_local_transform(group, rotated(FRAC_PI_4));
    let (_, p) = render(&mut gpu, &mut scene);
    // Blue over red, then halved once: no red shows through.
    // Half of 255 rounds either way, by GPU.
    let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1);
    assert!(
        near(pixel(&p, 100, 100), [0, 0, 128, 128]),
        "{:?}",
        pixel(&p, 100, 100)
    );
    assert!(
        near(pixel(&p, 100, 75), [0, 0, 128, 128]),
        "{:?}",
        pixel(&p, 100, 75)
    );
}

#[test]
fn transform_only_frames_composite_cached_layers_without_repainting() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
        gpu.set_direct_metal(direct);
        let mut scene = scene();
        let root = scene.root();
        // Four labelled cards, far enough apart to damage separately.
        let cards: Vec<NodeId> = [(10., 10.), (110., 10.), (10., 110.), (110., 110.)]
            .into_iter()
            .map(|(x, y)| {
                let card =
                    scene.append(root, NodeKind::Container(Layout::Overlay), fixed(50., 30.));
                scene.set_transform(card, Transform { x, y });
                rect(&mut scene, card, 0., 0., (50., 30.), RED);
                scene.append(
                    card,
                    NodeKind::Text {
                        text: "card".into(),
                        color: Color(255, 255, 255, 255),
                        font_size: 12.,
                    },
                    fixed(50., 30.),
                );
                card
            })
            .collect();
        for card in &cards {
            scene.set_local_transform(*card, rotated(0.1));
        }
        let (stats, _) = render(&mut gpu, &mut scene);
        assert_eq!(stats.layer_repaints, 4);
        let allocated = gpu.debug_cache_stats().layer_bytes;
        for frame in 1..=5 {
            for card in &cards {
                scene.set_local_transform(*card, rotated(0.1 + frame as f32 * 0.2));
            }
            let metal = gpu.debug_metal_frames();
            let (stats, _) = render(&mut gpu, &mut scene);
            // Where the adapter takes the direct Metal path (unified memory),
            // a transform-only frame takes it too.
            if direct && metal > 0 {
                assert!(gpu.debug_metal_frames() > metal);
            }
            if !direct {
                assert_eq!(gpu.debug_metal_frames(), 0);
            }
            assert_eq!(stats.layer_repaints, 0, "direct={direct}");
            assert_eq!(stats.layer_cache_hits, 4);
            assert_eq!(stats.layer_texture_allocations, 0);
            assert_eq!(stats.shaped_nodes, 0);
            assert_eq!(stats.glyph_uploads, 0);
            // One pass of four composites: no content is drawn again.
            assert_eq!(stats.render_passes, 1);
            assert_eq!(stats.instances, 4, "direct={direct}");
        }
        assert_eq!(gpu.debug_cache_stats().layer_bytes, allocated);
        // Back through identity and out again: the texture is kept.
        scene.set_local_transform(cards[0], LocalTransform::IDENTITY);
        let (stats, _) = render(&mut gpu, &mut scene);
        assert_eq!(stats.layer_repaints, 0);
        assert_eq!(gpu.debug_cache_stats().layer_textures, 4);
        scene.set_local_transform(cards[0], rotated(0.3));
        let (stats, _) = render(&mut gpu, &mut scene);
        assert_eq!(stats.layer_repaints, 0);
        assert_eq!(stats.layer_texture_allocations, 0);
    }
}

#[test]
fn raster_scale_rerenders_only_when_it_changes() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let root = scene.root();
    let card = scene.append(root, NodeKind::Container(Layout::Overlay), fixed(100., 50.));
    scene.set_transform(card, Transform { x: 50., y: 75. });
    scene.append(
        card,
        NodeKind::Text {
            text: "zoom".into(),
            color: Color(255, 255, 255, 255),
            font_size: 12.,
        },
        fixed(100., 50.),
    );
    let tick = rect(&mut scene, root, 0., 0., (1., 1.), Color(0, 0, 0, 0));
    scene.set_local_transform(card, rotated(0.2));
    assert_eq!(render(&mut gpu, &mut scene).0.layer_repaints, 1);
    let bytes = gpu.debug_cache_stats().layer_bytes;
    // (scale, repaints). Smaller than laid out: √2 steps, three quarters
    // of a step of hysteresis. Larger, moving: the raster it has while it
    // is fine enough, else the next √2 step above (room to grow into).
    let steps = [
        (0.9, 0),
        (0.8, 0),
        (0.6, 1),
        (0.55, 0),
        (0.65, 0),
        (0.45, 1),
        (0.6, 0),
        (0.8, 1),
        (1.0, 1),
        (1.005, 0),
        (1.1, 1),
        (1.1, 0),
        (1.25, 0),
        (1.4, 0),
        (2.5, 1),
        (2.5, 0),
    ];
    for (factor, repaints) in steps {
        scene.set_local_transform(
            card,
            LocalTransform {
                rotate: 0.2,
                ..scaled(factor)
            },
        );
        let (stats, _) = render(&mut gpu, &mut scene);
        assert_eq!(stats.layer_repaints, repaints, "scale {factor}");
        // Its frame drawn once: the new scale's transform damaged it whole.
        if factor > 1. && repaints == 1 {
            assert_eq!(stats.layer_cache_hits, 0, "scale {factor}");
        }
    }
    // Held a frame, at its scale exactly; held on, kept.
    assert_eq!(hold(&mut gpu, &mut scene, tick).layer_repaints, 1);
    assert_eq!(hold(&mut gpu, &mut scene, tick).layer_repaints, 0);
    scene.set_local_transform(card, rotated(0.2));
    assert_eq!(render(&mut gpu, &mut scene).0.layer_repaints, 1);
    // Rotation and skew at scale 1 never re-render.
    for angle in [0.5, 1.0, 2.0, 3.0] {
        scene.set_local_transform(
            card,
            LocalTransform {
                rotate: angle,
                skew: (angle * 0.1, 0.),
                ..LocalTransform::IDENTITY
            },
        );
        let (stats, _) = render(&mut gpu, &mut scene);
        assert_eq!(stats.layer_repaints, 0);
        assert_eq!(stats.layer_texture_allocations, 0);
    }
    // Magnified 3×, the layer renders finer than the window: more texels
    // (at 1× it may keep a texture a larger step allocated).
    scene.set_local_transform(card, scaled(3.));
    assert_eq!(render(&mut gpu, &mut scene).0.layer_repaints, 1);
    assert!(gpu.debug_cache_stats().layer_bytes > 4 * bytes);
}

/// Red pixels in column `x`.
fn column(pixels: &[u8], x: u32) -> u32 {
    (0..SIZE)
        .filter(|y| pixel(pixels, x, *y) == [255, 0, 0, 255])
        .count() as u32
}

#[test]
fn a_tilt_in_perspective_shows_the_nearer_side_larger() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
        gpu.set_direct_metal(direct);
        let mut scene = scene();
        let root = scene.root();
        let card = rect(&mut scene, root, 50., 50., (100., 100.), RED);
        render(&mut gpu, &mut scene);
        // Turned 30° about its vertical axis, seen from 300 points: its right
        // side goes away (shorter), its left comes nearer (taller).
        let tilt = LocalTransform {
            rotate_y: 30f32.to_radians(),
            perspective: 300.,
            ..LocalTransform::IDENTITY
        };
        scene.set_local_transform(card, tilt);
        let (stats, p) = render(&mut gpu, &mut scene);
        assert_eq!(stats.layer_repaints, 1, "direct {direct}");
        let matrix = tilt.projective(Rect::new(50., 50., 100., 100.));
        let (left, top) = matrix.point(50., 50.);
        let (right, top_right) = matrix.point(150., 50.);
        assert!(top < 50. && top_right > 50., "{top} {top_right}");
        let (near, far) = (
            column(&p, left.ceil() as u32 + 2),
            column(&p, right.floor() as u32 - 2),
        );
        assert!(
            near > 100 && far < 100 && near > far + 10,
            "direct {direct}: {near} {far}"
        );
        // Its centre stays, and what it no longer covers is clear.
        assert_eq!(pixel(&p, 100, 100), [255, 0, 0, 255]);
        assert_eq!(pixel(&p, right.ceil() as u32 + 2, 100), [0; 4]);
        // Hits follow the projection: inside the tilted card, not past it.
        assert_eq!(scene.hit_test(left + 3., 100.), Some(card));
        assert_ne!(scene.hit_test(right + 3., 100.), Some(card));

        // Without perspective the tilt only narrows it, both sides alike.
        scene.set_local_transform(
            card,
            LocalTransform {
                perspective: 0.,
                ..tilt
            },
        );
        let (_, p) = render(&mut gpu, &mut scene);
        let half = 50. * 30f32.to_radians().cos();
        let (l, r) = (
            (100. - half).ceil() as u32 + 1,
            (100. + half).floor() as u32 - 1,
        );
        assert_eq!(column(&p, l), 100);
        assert_eq!(column(&p, r), 100);
        assert_eq!(pixel(&p, (100. + half).ceil() as u32 + 1, 100), [0; 4]);
    }
}

/// A `size` × `size` window, cleared to transparent, at device scale 1.
fn window(size: u32) -> (GpuRenderer, Scene) {
    let gpu = GpuRenderer::new(size, size).unwrap();
    let mut scene = Scene::new(size as f32, size as f32);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    (gpu, scene)
}
fn alphas(pixels: &[u8]) -> Vec<u8> {
    pixels.chunks(4).map(|p| p[3]).collect()
}
/// Pixels neither clear nor covered: what antialiasing and resampling smear.
fn partial(alphas: &[u8]) -> usize {
    alphas.iter().filter(|a| **a > 24 && **a < 231).count()
}
/// White text of `font_size` in a `width` × `height` box at (`x`, `y`).
fn label(scene: &mut Scene, x: f32, y: f32, size: (f32, f32), font_size: f32) -> NodeId {
    let root = scene.root();
    let card = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        fixed(size.0, size.1),
    );
    scene.set_transform(card, Transform { x, y });
    scene.append(
        card,
        NodeKind::Text {
            text: "zag".into(),
            color: Color(255, 255, 255, 255),
            font_size,
        },
        fixed(size.0, size.1),
    );
    card
}

#[test]
fn a_magnified_layer_rasterizes_at_the_scale_it_shows() {
    let _gpu_fixture = gpu_fixture();
    const WINDOW: u32 = 400;
    // Text 8× magnified about its corner, and the same text on a display
    // of 8× density: pixel for pixel alike, not a small raster stretched.
    let (mut gpu, mut scene) = window(WINDOW);
    let card = label(&mut scene, 20., 20., (40., 20.), 12.);
    scene.set_local_transform(
        card,
        LocalTransform {
            origin: (0., 0.),
            ..scaled(8.)
        },
    );
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    let magnified = alphas(&gpu.readback().unwrap());
    let mut gpu = GpuRenderer::new(WINDOW, WINDOW).unwrap();
    gpu.set_scale_factor(8.);
    let mut scene = Scene::new(WINDOW as f32 / 8., WINDOW as f32 / 8.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    label(&mut scene, 2.5, 2.5, (40., 20.), 12.);
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    let native = alphas(&gpu.readback().unwrap());
    let covered = native.iter().filter(|a| **a > 127).count();
    assert!(covered > 2000, "{covered}");
    let differ = magnified
        .iter()
        .zip(&native)
        .filter(|(a, b)| a.abs_diff(**b) > 48)
        .count();
    // Stretched, every edge smears over a band several pixels wide.
    assert!(
        differ * 50 < covered,
        "{differ} of {covered} pixels differ; partial {} vs {}",
        partial(&magnified),
        partial(&native)
    );
}

#[test]
fn a_pushed_in_line_keeps_its_hard_edges() {
    let _gpu_fixture = gpu_fixture();
    const WINDOW: u32 = 400;
    // A one-point line under a push, slight or deep, stays as hard-edged as
    // drawn: as wide as the scale makes it, a pixel of coverage each side.
    // At 20× the stage is past the budget whole: what shows of it is drawn.
    let (mut gpu, mut scene) = window(WINDOW);
    let root = scene.root();
    let stage = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        fixed(WINDOW as f32, WINDOW as f32),
    );
    rect(&mut scene, stage, 0., 195., (WINDOW as f32, 1.), RED);
    rect(&mut scene, stage, 195., 0., (1., WINDOW as f32), RED);
    // Something else changing: a frame after the push, the line holds.
    let tick = rect(&mut scene, root, 0., 0., (1., 1.), Color(0, 0, 0, 0));
    for push in [1.07, 1.0731, 2.5, 2.4567, 20.] {
        scene.set_local_transform(stage, scaled(push));
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        hold(&mut gpu, &mut scene, tick);
        let p = gpu.readback().unwrap();
        // Rows across the horizontal line, columns across the vertical one.
        let (at, _) = scaled(push)
            .projective(Rect::new(0., 0., WINDOW as f32, WINDOW as f32))
            .point(195., 195.);
        let near = (at.floor() as u32).saturating_sub(3)..((at + push).ceil() as u32 + 3);
        let row: Vec<u8> = near
            .clone()
            .map(|y| pixel_in(&p, WINDOW, 40, y)[3])
            .collect();
        let col: Vec<u8> = near.map(|x| pixel_in(&p, WINDOW, x, 40)[3]).collect();
        for profile in [row, col] {
            let soft = partial(&profile);
            assert!(soft <= 2, "scale {push}: {profile:?}");
            let full = profile.iter().filter(|a| **a >= 231).count() as f32;
            assert!((full - push).abs() <= 1., "scale {push}: {profile:?}");
        }
    }
}
/// A frame with `tick` changed and nothing else, drawn whole (as a video's
/// are): what moved last frame holds still in this one.
fn hold(gpu: &mut GpuRenderer, scene: &mut Scene, tick: NodeId) -> zgx_gpu::GpuStats {
    let level = match scene.kind(tick) {
        NodeKind::Rect(Color(_, _, _, a)) => a ^ 1,
        _ => 0,
    };
    scene.set_kind(tick, NodeKind::Rect(Color(0, 0, 0, level)));
    scene.flush();
    gpu.render(scene, &[Rect::new(0., 0., 4096., 4096.)])
        .unwrap()
}
fn pixel_in(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    pixels[at..at + 4].try_into().unwrap()
}

#[test]
fn a_deep_push_draws_what_shows_and_keeps_it_while_that_holds() {
    let _gpu_fixture = gpu_fixture();
    const WINDOW: u32 = 400;
    let (mut gpu, mut scene) = window(WINDOW);
    let root = scene.root();
    let stage = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        fixed(WINDOW as f32, WINDOW as f32),
    );
    rect(&mut scene, stage, 0., 195., (WINDOW as f32, 1.), RED);
    let frame = |gpu: &mut GpuRenderer, scene: &mut Scene| {
        let damage = scene.flush().damage;
        gpu.render(scene, &damage).unwrap()
    };
    frame(&mut gpu, &mut scene);
    // 20×: whole, the stage would take 256 MB; what shows takes the window.
    scene.set_local_transform(stage, scaled(20.));
    assert_eq!(frame(&mut gpu, &mut scene).layer_repaints, 1);
    let cache = gpu.debug_cache_stats();
    assert_eq!(cache.layers_coarser, 0);
    assert!(cache.layer_bytes < 4 * 4 * (WINDOW * WINDOW) as usize);
    // Panned a little, it shows from what was drawn around the view...
    scene.set_transform(stage, Transform { x: 30., y: -30. });
    let stats = frame(&mut gpu, &mut scene);
    assert_eq!((stats.layer_repaints, stats.layer_cache_hits), (0, 1));
    // ...and far, it draws what shows there.
    scene.set_transform(stage, Transform { x: 300., y: 0. });
    assert_eq!(frame(&mut gpu, &mut scene).layer_repaints, 1);
    let p = gpu.readback().unwrap();
    // Its line still where the push puts it, a hard 20 pixels.
    let (_, y) = scaled(20.)
        .projective(Rect::new(0., 0., WINDOW as f32, WINDOW as f32))
        .point(0., 195.);
    let col: Vec<u8> = (y as u32 - 2..y as u32 + 22)
        .map(|y| pixel_in(&p, WINDOW, 200, y)[3])
        .collect();
    assert_eq!(col.iter().filter(|a| **a == 255).count(), 20, "{col:?}");
    assert_eq!(partial(&col), 0, "{col:?}");
}

#[test]
fn a_glyph_magnified_past_the_atlas_still_draws() {
    let _gpu_fixture = gpu_fixture();
    const WINDOW: u32 = 400;
    let (mut gpu, mut scene) = window(WINDOW);
    let root = scene.root();
    let stage = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        fixed(WINDOW as f32, WINDOW as f32),
    );
    let label = scene.append(
        stage,
        NodeKind::Text {
            text: "l".into(),
            color: Color(255, 255, 255, 255),
            font_size: 40.,
        },
        fixed(40., 60.),
    );
    scene.set_transform(label, Transform { x: 190., y: 170. });
    let frame = |gpu: &mut GpuRenderer, scene: &mut Scene| {
        let damage = scene.flush().damage;
        let stats = gpu.render(scene, &damage).unwrap();
        (stats, gpu.readback().unwrap())
    };
    let (_, p) = frame(&mut gpu, &mut scene);
    // The stem's middle, where the push goes in.
    let ink: Vec<u32> = (0..WINDOW)
        .filter(|x| (0..WINDOW).any(|y| pixel_in(&p, WINDOW, *x, y)[3] > 128))
        .collect();
    let stem = (ink[0] + ink[ink.len() - 1]) as f32 / 2. + 0.5;
    // 40 points at 80× is a glyph of 3,200 pixels, past what the atlas
    // rasterizes: the layer draws again no finer than the glyph fits, and
    // the stem still covers the middle of the window, not a gap. Deeper,
    // that raster serves on; back within the atlas, it is exact again.
    for (push, repaints) in [(80., 2), (85., 0), (30., 1), (90., 1)] {
        scene.set_local_transform(
            stage,
            LocalTransform {
                origin: (stem / WINDOW as f32, 0.5),
                ..scaled(push)
            },
        );
        let (stats, p) = frame(&mut gpu, &mut scene);
        assert_eq!(gpu.take_issues(), Vec::<String>::new());
        assert_eq!(stats.layer_repaints, repaints, "scale {push}");
        assert_eq!(pixel_in(&p, WINDOW, 200, 200)[3], 255, "scale {push}");
        assert_eq!(gpu.debug_cache_stats().layers_coarser, 0);
    }
}

const PARAGRAPH: (u32, u32) = (400, 240);
/// A paragraph in a 300 × 150 box, and a node beside it in the box that
/// can change each frame, as a film's content does (its layer then drawn
/// again every frame).
fn paragraph(scene: &mut Scene) -> (NodeId, NodeId) {
    let root = scene.root();
    let card = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        fixed(300., 150.),
    );
    scene.append(
        card,
        NodeKind::Text {
            text: "The quick brown fox jumps over the lazy dog. Pack my box with five \
                   dozen liquor jugs. How vexingly quick daft zebras jump! Sphinx of \
                   black quartz, judge my vow."
                .into(),
            color: Color(255, 255, 255, 255),
            font_size: 13.,
        },
        fixed(300., 150.),
    );
    let blink = scene.append(card, NodeKind::Rect(Color(0, 0, 0, 255)), fixed(2., 2.));
    scene.set_transform(blink, Transform { x: 0., y: 146. });
    (card, blink)
}
/// The paragraph's box corner at `corner` and scaled `scale` about it.
fn place(scene: &mut Scene, card: NodeId, (corner, scale): ((f32, f32), f32)) {
    scene.set_transform(
        card,
        Transform {
            x: corner.0,
            y: corner.1,
        },
    );
    scene.set_local_transform(
        card,
        LocalTransform {
            scale: (scale, scale),
            origin: (0., 0.),
            ..LocalTransform::IDENTITY
        },
    );
}
/// How much a moving paragraph's frames differ from each other once the
/// motion is taken out: each pixel of a frame against the last frame's at
/// the same point of the paragraph (bilinearly between its pixels), root
/// mean square over the pixels either shows ink in, in levels of 255.
/// `at(frame)`: its corner and scale. A picture moved rigidly scores above
/// zero too (resampled twice): about 11 panned, 30 zoomed, for this one
/// drawn 4× and box-filtered, or composited from a √2 raster.
fn wobble(at: impl Fn(u32) -> ((f32, f32), f32), frames: u32) -> f32 {
    let (width, height) = PARAGRAPH;
    let mut gpu = GpuRenderer::new(width, height).unwrap();
    let mut scene = Scene::new(width as f32, height as f32);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let (card, blink) = paragraph(&mut scene);
    let gray = |p: &[u8], x: i32, y: i32| -> f32 {
        if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
            return 0.;
        }
        f32::from(p[((y as u32 * width + x as u32) * 4) as usize])
    };
    let mut last: Option<(Vec<u8>, (f32, f32), f32)> = None;
    let (mut sum, mut count) = (0f64, 0u64);
    for frame in 0..frames {
        let (corner, scale) = at(frame);
        place(&mut scene, card, (corner, scale));
        scene.set_kind(blink, NodeKind::Rect(Color((frame % 2) as u8, 0, 0, 255)));
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let p = gpu.readback().unwrap();
        if let Some((before, was, was_scale)) = &last {
            for y in 0..height as i32 {
                for x in 0..width as i32 {
                    // The paragraph's point under this pixel's middle, and
                    // where the last frame showed it.
                    let px = (x as f32 + 0.5 - corner.0) / scale;
                    let py = (y as f32 + 0.5 - corner.1) / scale;
                    let (bx, by) = (was.0 + px * was_scale - 0.5, was.1 + py * was_scale - 0.5);
                    let (x0, y0) = (bx.floor(), by.floor());
                    let (fx, fy) = (bx - x0, by - y0);
                    let (x0, y0) = (x0 as i32, y0 as i32);
                    let then = gray(before, x0, y0) * (1. - fx) * (1. - fy)
                        + gray(before, x0 + 1, y0) * fx * (1. - fy)
                        + gray(before, x0, y0 + 1) * (1. - fx) * fy
                        + gray(before, x0 + 1, y0 + 1) * fx * fy;
                    let now = gray(&p, x, y);
                    if now > 8. || then > 8. {
                        sum += f64::from((now - then) * (now - then));
                        count += 1;
                    }
                }
            }
        }
        last = Some((p, corner, scale));
    }
    (sum / count.max(1) as f64).sqrt() as f32
}

#[test]
fn text_under_a_moving_transform_moves_as_one_picture() {
    let _gpu_fixture = gpu_fixture();
    // Zoomed slowly from 1 to 1.3 over 60 frames, panned a tenth of a pixel
    // a frame, and both, its content changing all the while. Drawn afresh
    // at each frame's scale, its glyphs and lines landed on other pixels
    // each frame: 70, 44 and 77. As one picture: within the floor above.
    let zoom = |f: u32| ((20., 20.), 1. + 0.3 * f as f32 / 60.);
    let pan = |f: u32| ((20.3 + 0.1 * f as f32, 20.), 1.15);
    let both = |f: u32| ((20.3 + 0.1 * f as f32, 20.), 1. + 0.3 * f as f32 / 60.);
    let measured = [wobble(zoom, 61), wobble(pan, 61), wobble(both, 61)];
    assert!(
        measured[0] < 34. && measured[1] < 14. && measured[2] < 35.,
        "zoom, pan, both: {measured:?}"
    );

    // Come to rest, it is drawn at its scale exactly: as sharp as a
    // paragraph shown there from the start.
    let (width, height) = PARAGRAPH;
    let whole = [Rect::new(0., 0., width as f32, height as f32)];
    let mut gpu = GpuRenderer::new(width, height).unwrap();
    let mut scene = Scene::new(width as f32, height as f32);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let (card, blink) = paragraph(&mut scene);
    for frame in 0..=60 {
        place(&mut scene, card, both(frame));
        scene.flush();
        gpu.render(&scene, &whole).unwrap();
    }
    scene.set_kind(blink, NodeKind::Rect(Color(1, 0, 0, 255)));
    scene.flush();
    let stats = gpu.render(&scene, &whole).unwrap();
    assert_eq!(stats.layer_repaints, 1);
    let rest = gpu.readback().unwrap();
    let mut fresh = GpuRenderer::new(width, height).unwrap();
    let mut scene = Scene::new(width as f32, height as f32);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let (card, blink) = paragraph(&mut scene);
    scene.set_kind(blink, NodeKind::Rect(Color(1, 0, 0, 255)));
    place(&mut scene, card, both(60));
    scene.flush();
    fresh.render(&scene, &whole).unwrap();
    let first = fresh.readback().unwrap();
    let differ = rest
        .iter()
        .zip(&first)
        .filter(|(a, b)| a.abs_diff(**b) > 1)
        .count();
    let most = rest.iter().zip(&first).map(|(a, b)| a.abs_diff(*b)).max();
    assert_eq!(differ, 0, "{differ} channels differ, by up to {most:?}");
}

#[test]
fn a_moving_layer_waiting_on_a_picture_draws_each_frame_once() {
    let _gpu_fixture = gpu_fixture();
    // A magnified board drifting by fractions of a point, waiting on a
    // picture all the while: drawn again every frame, and again within the
    // frame when its corner moves (`layer_shifted`). Its texel grid where
    // it was, its corner never moves: each frame draws it once at most,
    // however long it drifts. (Worked out again from where it last landed, the
    // corner wandered by a texel between a frame's drawings, which drew it
    // again, and in a long render without end.)
    const WINDOW: u32 = 300;
    let (mut gpu, mut scene) = window(WINDOW);
    let root = scene.root();
    let board = scene.append(root, NodeKind::Container(Layout::Overlay), fixed(120., 80.));
    rect(&mut scene, board, 0., 0., (120., 80.), RED);
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(400, 400, image::Rgba([0, 0, 255, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let picture = zgx_gpu::assets::decode_image_kept(std::sync::Arc::from(png)).unwrap();
    let held = scene.append(board, NodeKind::Image(picture), fixed(20., 20.));
    scene.set_transform(held, Transform { x: 10., y: 10. });
    let whole = [Rect::new(0., 0., WINDOW as f32, WINDOW as f32)];
    for frame in 0..600 {
        let t = frame as f32;
        scene.set_transform(
            board,
            Transform {
                x: 37.3 + t * 0.0137,
                y: 41.7 + t * 0.0291,
            },
        );
        scene.set_local_transform(
            board,
            LocalTransform {
                scale: (1.9 + t * 1e-4, 1.9 + t * 1e-4),
                ..LocalTransform::IDENTITY
            },
        );
        scene.flush();
        let stats = gpu.render(&scene, &whole).unwrap();
        // (None, once the picture is in.)
        assert!(stats.layer_repaints <= 1, "frame {frame}: {stats:?}");
    }
}

#[test]
fn layers_not_drawn_give_way_to_a_moving_layer_and_its_new_text() {
    let _gpu_fixture = gpu_fixture();
    // Layers drawn once and hidden since (still layers, their textures
    // kept) holding all of the 64 MB budget; then a magnified board in
    // motion, blurred a little (so not drawn in part), a word appearing in
    // it midway. The hidden layers give way: the board is drawn at its
    // scale, its word there the frame it appears.
    const WINDOW: u32 = 400;
    let (mut gpu, mut scene) = window(WINDOW);
    let root = scene.root();
    let whole = [Rect::new(0., 0., WINDOW as f32, WINDOW as f32)];
    let old: Vec<NodeId> = (0..16)
        .map(|_| {
            let id = scene.append(
                root,
                NodeKind::Container(Layout::Overlay),
                fixed(1000., 1000.),
            );
            scene.set_isolated(id, true);
            rect(&mut scene, id, 0., 0., (1000., 1000.), Color(0, 0, 30, 255));
            id
        })
        .collect();
    scene.flush();
    gpu.render(&scene, &whole).unwrap();
    for id in &old {
        scene.set_effects(
            *id,
            Effects {
                opacity: 0.,
                ..Default::default()
            },
        );
    }
    let board = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        fixed(600., 400.),
    );
    scene.set_effects(
        board,
        Effects {
            filter_blur: 0.25,
            ..Default::default()
        },
    );
    let word = scene.append(
        board,
        NodeKind::Text {
            text: "Zeron".into(),
            color: Color(255, 255, 255, 255),
            font_size: 30.,
        },
        fixed(600., 400.),
    );
    scene.set_effects(
        word,
        Effects {
            opacity: 0.,
            ..Default::default()
        },
    );
    for frame in 0..20 {
        let s = 1.3 + frame as f32 * 0.01;
        scene.set_local_transform(
            board,
            LocalTransform {
                scale: (s, s),
                origin: (0., 0.),
                ..LocalTransform::IDENTITY
            },
        );
        if frame == 10 {
            scene.set_effects(word, Effects::default());
        }
        scene.flush();
        gpu.render(&scene, &whole).unwrap();
        let p = gpu.readback().unwrap();
        let ink = p.chunks(4).filter(|c| c[0] > 160).count();
        assert_eq!(ink > 200, frame >= 10, "frame {frame}: {ink} pixels of ink");
        assert_eq!(gpu.debug_cache_stats().layers_coarser, 0, "frame {frame}");
    }
}
