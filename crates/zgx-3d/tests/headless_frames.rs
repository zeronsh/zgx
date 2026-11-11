//! A driver without a frame clock (a headless `GpuRenderer`: tests,
//! benchmarks) gives an animating world its frames itself. A world with
//! compiles in flight animates until they're in; drawn only when something
//! changed, as a still camera never changes anything, it never takes them
//! up (harbor_bench's still path: its sea and containers never drawn).
#![cfg(all(feature = "shaders", feature = "compute"))]
use std::time::Duration;
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

#[test]
fn a_still_world_takes_up_its_compiles_once_its_driver_gives_it_frames() {
    let world = World::new();
    world.set_environment(Some(Environment::studio()));
    let mesh = world.add(Object::mesh(
        Geometry::cuboid(1., 1., 1.),
        Material::rgb(0xffffff),
    ));
    world.set_camera(Camera::looking_at([0., 0., 3.], [0., 0., 0.]));
    let red = world
        .add_shader(MaterialShader::new(
            "#zgx material 1
             fn surface(s: SurfaceIn) -> Surface {
                 var m = surface_default(s);
                 m.base_color = vec3(1.0, 0.0, 0.0);
                 return m;
             }",
        ))
        .unwrap();
    world.set_shader(mesh, Some(red));
    let wet = world
        .add_surface_layer(MaterialShader::new(
            "#zgx material 1
             fn surface(s: SurfaceIn) -> Surface {
                 var m = surface_default(s);
                 m.roughness *= 0.5;
                 return m;
             }",
        ))
        .unwrap();
    let counts = world.add_buffer(BufferDesc::from_slice(&[0u32])).unwrap();
    let pass = world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read_write> count: array<u32>;
                 @compute @workgroup_size(1) fn step() { count[0] += 1u; }",
            )
            .buffer("count", &counts, Access::ReadWrite)
            .threads([1, 1, 1])
            .every_frame(),
        )
        .unwrap();
    let mut gpu = match GpuRenderer::new(64, 64) {
        Ok(gpu) => gpu,
        Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
            panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
        }
        Err(_) => return,
    };
    let mut ui = Ui::new(64., 64.);
    ui.mount(
        div()
            .size(64., 64.)
            .bg(Color(0, 0, 0, 255))
            .child(gpu_view("World", world.view()).size(64., 64.)),
    );
    let camera = world.camera();
    let mut frame = |clock: bool| {
        // The camera set where it is, as a still path sets it: no change.
        world.set_camera(camera);
        // A frame clock's frame: while the view animates.
        if clock && world.view().animating() {
            world.view().invalidate();
        }
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
        std::thread::sleep(Duration::from_millis(5));
    };
    let ready = |world: &World| {
        world.shader_ready(red)
            && world.shader_ready(wet)
            && world.compute_status(pass) == Some(ComputeStatus::Ready)
    };
    // Drawn only where damaged: the first frame, then none of the world's.
    for _ in 0..60 {
        frame(false);
    }
    assert!(
        world.view().animating(),
        "its compiles hold it drawing: it asks for frames"
    );
    assert!(!ready(&world), "never taken up without frames");
    // Its frames given while it asks: in, on the frames that follow.
    let mut given = 0;
    while !ready(&world) && given < 400 {
        frame(true);
        given += 1;
    }
    assert!(ready(&world), "in once given frames ({given} drawn)");
    eprintln!("ready after {given} frames given");
}
