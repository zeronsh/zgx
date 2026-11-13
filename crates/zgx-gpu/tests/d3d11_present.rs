//! Direct3D 11's frame as it would be presented (`ZGX_D3D11=1`, the `d3d11`
//! feature): `debug_present_offscreen` gives what its swap chain would show,
//! which Direct3D 11 draws into directly, so a frame repainted in part (a
//! box moved over a kept frame) presents as a frame drawn afresh does.
//! `ZGX_D3D11` is read once a process, so this test is one of its own,
//! setting it first.
#[cfg(windows)]
use zgx::scene::*;
#[cfg(windows)]
use zgx_gpu::GpuRenderer;

#[cfg(windows)]
const SIZE: u32 = 160;

/// Stripes, and a box over them at `x`.
#[cfg(windows)]
fn striped(x: f32) -> (Scene, NodeId) {
    let mut scene = Scene::new(SIZE as f32, SIZE as f32);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    for i in 0..8 {
        let stripe = scene.append(
            root,
            NodeKind::Rect(if i % 2 == 0 {
                Color(30, 60, 200, 255)
            } else {
                Color(230, 130, 30, 255)
            }),
            Style {
                width: Some(SIZE as f32),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.set_transform(
            stripe,
            Transform {
                x: 0.,
                y: i as f32 * 20.,
            },
        );
    }
    let moving = scene.append(
        root,
        NodeKind::Rect(Color(240, 240, 240, 255)),
        Style {
            width: Some(40.),
            height: Some(40.),
            ..Default::default()
        },
    );
    scene.set_transform(moving, Transform { x, y: 60. });
    (scene, moving)
}

#[cfg(windows)]
fn main() {
    // SAFETY: Set before anything else runs, the test's only thread.
    unsafe { std::env::set_var("ZGX_D3D11", "1") };
    let (mut scene, moving) = striped(20.);
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    assert_eq!(gpu.adapter_info().backend, zgx_gpu::wgpu::Backend::Noop);
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    scene.set_transform(moving, Transform { x: 90., y: 60. });
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    let presented = gpu
        .debug_present_offscreen()
        .expect("Direct3D 11 presents offscreen");
    assert_eq!(
        presented,
        gpu.readback().unwrap(),
        "what it presents is its frame"
    );
    let (mut fresh_scene, _) = striped(90.);
    fresh_scene.flush();
    let mut fresh = GpuRenderer::new(SIZE, SIZE).unwrap();
    let whole = [Rect::new(0., 0., SIZE as f32, SIZE as f32)];
    fresh.render(&fresh_scene, &whole).unwrap();
    assert_eq!(
        presented,
        fresh.debug_present_offscreen().unwrap(),
        "repainted in part, it presents as a frame drawn afresh"
    );
    println!("d3d11_present: presented offscreen as drawn");
}

#[cfg(not(windows))]
fn main() {}
