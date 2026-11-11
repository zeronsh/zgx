//! SCRATCH (uncommitted): which worlds of a `gi` build meet DXC's -Od crash.
//! Each case runs in a process of its own (`--exact`), since ray queries
//! are requested once per process, before the first device.
use zgx::{compose::prelude::*, scene::Color};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

fn draw(world: &World) {
    let gpu = GpuRenderer::new(64, 64).expect("renderer");
    let mut gpu = gpu;
    let mut ui = zgx::widgets::Ui::new(64., 64.);
    ui.mount(
        div()
            .w_full()
            .h_full()
            .bg(Color(0, 0, 0, 255))
            .child(gpu_view("World", world.view()).w_full().h_full()),
    );
    for _ in 0..3 {
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
        world.view().invalidate();
    }
    println!(
        "CASE ray_queries={:?} probes={:?}",
        zgx_gpu::ray_queries_status(),
        world.probes_status()
    );
}

fn plain() -> World {
    let world = World::new();
    world.add(Object::mesh(Geometry::sphere(0.5, 16), Material::rgb(0xd03030)));
    world.add_light(Light::Directional {
        direction: [-0.3, -1., -0.6].into(),
        color: [1., 1., 1.],
        intensity: 2.,
    });
    world.set_camera(Camera::looking_at([0., 0.5, 2.5], [0., 0., 0.]));
    world
}

fn with_probes() -> World {
    let world = plain();
    world
        .add_probes(ProbeVolume::new([-1., -1., -1.], [1., 1., 1.], [4, 4, 4]))
        .unwrap();
    world
}

#[test]
fn case_plain_world_no_request() {
    draw(&plain());
}

#[test]
fn case_probes_no_request() {
    draw(&with_probes());
}

#[test]
fn case_plain_world_requested() {
    // SAFETY: wgpu's experimental contract, as gi.rs takes it on.
    unsafe { zgx_gpu::request_ray_queries() };
    draw(&plain());
}

#[test]
fn case_probes_requested() {
    // SAFETY: as above.
    unsafe { zgx_gpu::request_ray_queries() };
    draw(&with_probes());
}
