//! What a world's device can do, read from the device a window draws it
//! with.
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

#[test]
fn a_world_drawn_in_a_window_knows_its_devices_tier() {
    let world = World::new();
    world.add(Object::mesh(
        Geometry::cuboid(1., 1., 1.),
        Material::default(),
    ));
    let mut gpu = match GpuRenderer::new(32, 32) {
        Ok(gpu) => gpu,
        Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
            panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
        }
        Err(_) => return,
    };
    let mut ui = Ui::new(32., 32.);
    ui.mount(
        div()
            .size(32., 32.)
            .bg(Color(0, 0, 0, 255))
            .child(gpu_view("World", world.view()).size(32., 32.)),
    );
    assert_eq!(world.caps(), None);
    ui.prepare_frame();
    let report = ui.scene.borrow_mut().flush();
    gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
    assert_eq!(world.frames_drawn(), 1);
    let caps = world.caps().expect("read with the first frame");
    match caps.backend {
        // Every backend but GL is tier A wherever wgpu runs on it.
        Backend::Metal | Backend::Dx12 | Backend::Vulkan => {
            assert_eq!(caps.tier, Tier::A, "{caps:?}");
            assert!(caps.compute && caps.storage_in_vertex && caps.indirect);
            assert!(caps.max_workgroups >= 65535 && caps.max_invocations >= 256);
        }
        Backend::Gl => assert_eq!(caps.tier, Tier::B, "{caps:?}"),
        _ => {}
    }
    if cfg!(target_os = "macos") && std::env::var_os("ZGX_REQUIRE_GPU").is_some() {
        assert_eq!(caps.backend, Backend::Metal);
    }
}
