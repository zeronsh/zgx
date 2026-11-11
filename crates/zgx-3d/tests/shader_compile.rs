//! A custom material's pipeline compiles off the frame: a frame showing a
//! shader not yet compiled returns without waiting for the driver, its
//! mesh drawn by the plain pipeline, and a later frame draws it by its
//! shader. (Its own binary: it makes the compile worker slow.)
use std::time::{Duration, Instant};
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

const SIZE: f32 = 100.;
const SLOW: u64 = 400;

#[test]
fn a_frame_with_a_shader_compiling_returns_without_waiting_for_it() {
    // SAFETY: this binary's one test, set before any thread it starts reads
    // the environment.
    unsafe { std::env::set_var("ZGX_TEST_COMPILE_DELAY_MS", SLOW.to_string()) };
    let gpu = match GpuRenderer::new(SIZE as u32, SIZE as u32) {
        Ok(gpu) => gpu,
        Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
            panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
        }
        Err(_) => return,
    };
    let world = World::new();
    let mut material = Material::rgb(0x336699);
    material.unlit = true;
    let mesh = world.add(Object::mesh(Geometry::cuboid(1., 1., 1.), material));
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
    let mut ui = Ui::new(SIZE, SIZE);
    ui.mount(
        div().size(SIZE, SIZE).bg(Color(0, 0, 0, 255)).child(
            gpu_view("World", world.view())
                .absolute()
                .size(SIZE, SIZE)
                .bg(Color(0, 128, 0, 255)),
        ),
    );
    let mut gpu = gpu;
    let mut frame = |ui: &mut Ui| {
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
        let pixels = gpu.readback().unwrap();
        let at = (50 * SIZE as usize + 50) * 4;
        <[u8; 4]>::try_from(&pixels[at..at + 4]).unwrap()
    };
    let start = Instant::now();
    let first = frame(&mut ui);
    let took = start.elapsed();
    assert!(
        took < Duration::from_millis(SLOW / 2),
        "the frame waited: {took:?}"
    );
    assert!(world.compiling(), "its pipeline is compiling");
    assert_eq!(
        first,
        [0x33, 0x66, 0x99, 255],
        "drawn by the plain pipeline meanwhile"
    );
    // Frames go on (the world animates while it compiles) until it's drawn
    // by its shader.
    let mut last = first;
    let start = Instant::now();
    while world.compiling() && start.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(5));
        world.view().invalidate();
        last = frame(&mut ui);
    }
    assert!(!world.compiling());
    assert_eq!(last, [255, 0, 0, 255], "drawn by its shader once compiled");
    assert!(compile_threads() >= 1);
}
