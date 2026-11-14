//! Renderers made and drawing on several threads at once, each its own
//! (instance, device, pipelines), as an app's video and view hosts make
//! them: a frame's command buffers and lazily made pipelines named while
//! another thread makes a device. Vulkan loaders before 1.4.345
//! (Vulkan-Loader#1866) crash there in debug builds, whose debug layer
//! names the driver's objects; zgx-gpu leaves that layer off on such a
//! loader (`loader_races_debug`). The race needs such a loader to show
//! (Ubuntu 26.04's, 1.4.341): with a fixed one, or another platform's, this
//! passes either way.
use zgx::scene::{Color, NodeKind, Scene, Style};
use zgx_gpu::GpuRenderer;

#[test]
fn renderers_made_and_drawing_on_eight_threads_at_once_each_draw() {
    if GpuRenderer::new(4, 4).is_err() {
        eprintln!("no GPU adapter");
        return;
    }
    for round in 0..4 {
        let start = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    let mut gpu = GpuRenderer::new(32, 32).expect("a renderer");
                    let mut scene = Scene::new(32., 32.);
                    let square = scene.append(
                        scene.root(),
                        NodeKind::Rect(Color(255, 0, 0, 255)),
                        Style {
                            width: Some(8.),
                            height: Some(8.),
                            ..Default::default()
                        },
                    );
                    // Frames each changing something, each read back: a
                    // command buffer a frame, named where names are on.
                    for frame in 0..20u8 {
                        scene.set_kind(square, NodeKind::Rect(Color(frame * 12, i * 30, 0, 255)));
                        let damage = scene.flush().damage;
                        gpu.render(&scene, &damage).expect("a frame");
                        let pixels = gpu.readback().expect("its pixels");
                        assert_eq!(pixels.len(), 32 * 32 * 4);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread
                .join()
                .unwrap_or_else(|_| panic!("round {round}: a thread failed"));
        }
    }
}
