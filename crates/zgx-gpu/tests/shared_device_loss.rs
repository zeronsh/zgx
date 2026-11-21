//! On Direct3D 12 every context in the process draws with one device (a
//! GPU's while a context holds it, WARP's for the process): lost, it fails
//! every context on it, and each gets a new renderer that draws again, on a
//! new device. Elsewhere each context has a device of its own, and a loss
//! fails that context alone. A file of its own: losing the shared device
//! would fail every other test drawing beside it.
use zgx::scene::{Color, Layout, NodeKind, Rect, Scene, Style};
use zgx_gpu::{GpuContext, GpuRenderer, wgpu};

const RED: Color = Color(255, 0, 0, 255);

fn scene() -> Scene {
    let mut scene = Scene::new(64., 64.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let root = scene.root();
    scene.append(
        root,
        NodeKind::Rect(RED),
        Style {
            width: Some(32.),
            height: Some(32.),
            ..Default::default()
        },
    );
    scene.flush();
    scene
}

#[test]
fn a_shared_device_lost_fails_every_context_on_it_and_each_draws_again() {
    let Ok(first) = GpuContext::new() else {
        eprintln!("no GPU here");
        return;
    };
    let second = GpuContext::new().unwrap();
    let scene = scene();
    let full = [Rect::new(0., 0., 64., 64.)];
    let mut renderers = [
        GpuRenderer::new_with_context(64, 64, &first).unwrap(),
        GpuRenderer::new_with_context(64, 64, &second).unwrap(),
    ];
    let d3d12 = renderers[0].adapter_info().backend == wgpu::Backend::Dx12;
    assert!(!first.shares_device(&second), "two contexts");
    assert_eq!(
        first.same_device_as(&second),
        d3d12,
        "one device on Direct3D 12, one each elsewhere ({:?})",
        renderers[0].adapter_info().backend
    );
    for gpu in &mut renderers {
        gpu.render(&scene, &full).unwrap();
    }
    first.debug_lose_shared_device();
    assert!(first.lost().is_some());
    assert_eq!(second.lost().is_some(), d3d12, "the other context too");
    for (k, gpu) in renderers.iter_mut().enumerate() {
        let lost = k == 0 || d3d12;
        match gpu.render(&scene, &full) {
            Err(error) => assert!(lost && error.is_device_lost(), "{k}: {error}"),
            Ok(_) => assert!(!lost, "{k} drew on a lost device"),
        }
    }
    // Each gets a new renderer, on a new context: they draw, on a device
    // shared again (Direct3D 12) but not the one lost.
    let again: Vec<GpuRenderer> = (0..2).map(|_| GpuRenderer::new(64, 64).unwrap()).collect();
    for mut gpu in again.into_iter() {
        assert!(!gpu.context().same_device_as(&first), "not the lost device");
        assert_eq!(gpu.context().lost(), None);
        gpu.render(&scene, &full).unwrap();
        let pixels = gpu.readback().unwrap();
        assert_eq!(&pixels[..4], &[RED.0, RED.1, RED.2, 255]);
    }
    let (a, b) = (
        GpuRenderer::new(8, 8).unwrap(),
        GpuRenderer::new(8, 8).unwrap(),
    );
    assert_eq!(a.context().same_device_as(&b.context()), d3d12);
}
