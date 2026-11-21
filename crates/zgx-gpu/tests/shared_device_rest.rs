//! A Direct3D 12 GPU's device is shared only while a context holds it: once
//! the last renderer and context are gone, nothing of it is held, and the
//! next context makes a device of its own. A file of its own: other tests'
//! contexts would hold the device.
use zgx::scene::{Color, NodeKind, Rect, Scene, Style};
use zgx_gpu::{GpuRenderer, d3d12_device_held, wgpu};

fn draw(gpu: &mut GpuRenderer) {
    let mut scene = Scene::new(16., 16.);
    let root = scene.root();
    scene.append(
        root,
        NodeKind::Rect(Color(0, 128, 255, 255)),
        Style {
            width: Some(16.),
            height: Some(16.),
            ..Default::default()
        },
    );
    scene.flush();
    gpu.render(&scene, &[Rect::new(0., 0., 16., 16.)]).unwrap();
    let pixels = gpu.readback().unwrap();
    assert_eq!(&pixels[..4], &[0, 128, 255, 255]);
}

#[test]
fn a_gpus_shared_device_goes_with_the_last_context_on_it() {
    let Ok(mut first) = GpuRenderer::new(16, 16) else {
        eprintln!("no GPU here");
        return;
    };
    let info = first.adapter_info();
    // A GPU's, not WARP's (kept for the process) or another backend's (a
    // device each).
    let gpu = info.backend == wgpu::Backend::Dx12 && info.device_type != wgpu::DeviceType::Cpu;
    let mut second = GpuRenderer::new(16, 16).unwrap();
    draw(&mut first);
    draw(&mut second);
    assert_eq!(d3d12_device_held(), gpu, "{info:?}");
    drop(first);
    assert_eq!(d3d12_device_held(), gpu, "the second context holds it");
    drop(second);
    assert!(!d3d12_device_held(), "nothing held at rest");
    // A new context makes one again, and draws.
    let mut third = GpuRenderer::new(16, 16).unwrap();
    draw(&mut third);
    assert_eq!(d3d12_device_held(), gpu);
    drop(third);
    assert!(!d3d12_device_held());
}
