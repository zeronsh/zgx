//! Renderers on threads of one process, each on a context of its own, all
//! drawing at once: every one draws its own pixels. On WARP (Direct3D 12 on
//! the CPU, 10.0.26100) a device per thread faulted in the driver or drew
//! nothing at all; there they share one device.
use std::sync::{Arc, Barrier};
use zgx::scene::{Color, Layout, NodeKind, Rect, Scene, Style};
use zgx_gpu::GpuRenderer;

const THREADS: u8 = 8;
const FRAMES: u8 = 12;

fn scene(color: Color) -> Scene {
    let mut scene = Scene::new(64., 64.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let root = scene.root();
    scene.append(
        root,
        NodeKind::Rect(color),
        Style {
            width: Some(64.),
            height: Some(64.),
            ..Default::default()
        },
    );
    scene
}

#[test]
fn renderers_drawing_at_once_on_threads_each_draw_their_pixels() {
    if GpuRenderer::new(8, 8).is_err() {
        eprintln!("no GPU here");
        return;
    }
    // Every thread makes its renderer, then all draw together.
    let start = Arc::new(Barrier::new(THREADS.into()));
    let threads: Vec<_> = (0..THREADS)
        .map(|thread| {
            let start = start.clone();
            std::thread::spawn(move || {
                let mut gpu = GpuRenderer::new(64, 64).unwrap();
                start.wait();
                let full = [Rect::new(0., 0., 64., 64.)];
                for frame in 0..FRAMES {
                    // Whole channels (no rounding or dither to allow for),
                    // a different colour each frame and on each thread.
                    let bits = (thread + frame) % 7 + 1;
                    let channel = |bit: u8| if bits & bit != 0 { 255 } else { 0 };
                    let color = Color(channel(1), channel(2), channel(4), 255);
                    let mut scene = scene(color);
                    scene.flush();
                    gpu.render(&scene, &full).unwrap();
                    let pixels = gpu.readback().unwrap();
                    let expected = [color.0, color.1, color.2, color.3];
                    for (i, pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
                        assert_eq!(
                            pixel,
                            &expected,
                            "thread {thread}, frame {frame}, pixel {i} ({:?})",
                            gpu.adapter_info().name
                        );
                    }
                }
                FRAMES
            })
        })
        .collect();
    let drawn: u32 = threads
        .into_iter()
        .map(|thread| u32::from(thread.join().unwrap()))
        .sum();
    assert_eq!(drawn, u32::from(THREADS) * u32::from(FRAMES));
}
