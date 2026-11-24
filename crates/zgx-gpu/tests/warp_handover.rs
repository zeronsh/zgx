//! A renderer made on one thread just as the last renderer on another is
//! dropped, its frame still being drawn: the new one draws its own pixels.
//! On WARP (Direct3D 12 on the CPU) every context shares one device, as two
//! wgpu devices drawing at once faulted in the driver (renderers_on_threads).
//! A renderer's context let go of that device before the renderer's own
//! buffers, textures and queue were gone, and the queue waits there for its
//! last frame: a context made then got a second device beside the first,
//! which crashed zgx-video's render tests on Windows (#2402).
use std::sync::{Arc, mpsc};
use zgx::{
    canvas::*,
    scene::{Color, Layout, NodeKind, Rect, Scene, Style},
};
use zgx_gpu::GpuRenderer;

const ROUNDS: u32 = 40;
/// The frame left drawing as its renderer goes: large, so WARP takes a while.
const BIG: u32 = 1024;
const SMALL: u32 = 64;

/// A gradient over all of it, and many dashed curves.
fn heavy(size: u32, round: u32) -> Scene {
    let side = size as f32;
    let mut canvas = Canvas::new();
    canvas.fill(
        Path::rectangle(Rect::new(0., 0., side, side)),
        Brush::linear(
            Point::new(0., 0.),
            Point::new(side, side),
            vec![
                GradientStop {
                    offset: 0.,
                    color: Color(58, 139, 255, 255),
                },
                GradientStop {
                    offset: 1.,
                    color: Color(255, 80, 40, 255),
                },
            ],
        )
        .unwrap(),
    );
    for k in 0..64u32 {
        let y = side * (k as f32 + 0.5) / 64.;
        let mut curve = Path::builder();
        curve.move_to(0., y).cubic_to(
            Point::new(side / 3., y - 40. - round as f32),
            Point::new(2. * side / 3., y + 40.),
            Point::new(side, y),
        );
        canvas.stroke(
            curve.build().unwrap(),
            Color(255, 255, 255, 200),
            Stroke {
                width: 3.,
                cap: LineCap::Round,
                dash: vec![6., 3.],
                ..Default::default()
            },
        );
    }
    let mut scene = Scene::new(side, side);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let root = scene.root();
    scene.append(
        root,
        NodeKind::Canvas(Arc::new(canvas)),
        Style {
            width: Some(side),
            height: Some(side),
            ..Default::default()
        },
    );
    scene
}

fn solid(color: Color) -> Scene {
    let mut scene = Scene::new(SMALL as f32, SMALL as f32);
    let root = scene.root();
    scene.append(
        root,
        NodeKind::Rect(color),
        Style {
            width: Some(SMALL as f32),
            height: Some(SMALL as f32),
            ..Default::default()
        },
    );
    scene
}

#[test]
fn a_renderer_made_as_the_last_one_goes_draws_its_own_pixels() {
    if GpuRenderer::new(8, 8).is_err() {
        eprintln!("no GPU here");
        return;
    }
    // The giver draws a large frame, says so, and drops its renderer with
    // the frame still drawing; the taker makes one as soon as it hears.
    let (given, heard) = mpsc::channel::<u32>();
    let (done, wait) = mpsc::channel::<()>();
    let taker = std::thread::spawn(move || {
        for round in heard {
            let mut gpu = GpuRenderer::new(SMALL, SMALL).unwrap();
            let color = Color(
                if round % 2 == 0 { 255 } else { 0 },
                255,
                if round % 3 == 0 { 255 } else { 0 },
                255,
            );
            let full = [Rect::new(0., 0., SMALL as f32, SMALL as f32)];
            for _ in 0..3 {
                let mut scene = solid(color);
                scene.flush();
                gpu.render(&scene, &full).unwrap();
                let pixels = gpu.readback().unwrap();
                let i = 4 * (SMALL / 2 * SMALL + SMALL / 2) as usize;
                assert_eq!(
                    &pixels[i..i + 4],
                    &[color.0, color.1, color.2, 255],
                    "round {round} ({})",
                    gpu.adapter_info().name
                );
            }
            drop(gpu);
            done.send(()).unwrap();
        }
    });
    let full = [Rect::new(0., 0., BIG as f32, BIG as f32)];
    for round in 0..ROUNDS {
        let mut gpu = GpuRenderer::new(BIG, BIG).unwrap();
        // Each frame submits the one before it, not waited for.
        for frame in 0..3 {
            let mut scene = heavy(BIG, round + frame);
            scene.flush();
            gpu.render(&scene, &full).unwrap();
        }
        given.send(round).unwrap();
        drop(gpu);
        wait.recv().unwrap();
    }
    drop(given);
    taker.join().unwrap();
}
