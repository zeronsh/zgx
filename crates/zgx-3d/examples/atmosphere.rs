//! A world under a sky (feature `media`): the air's light from the sun,
//! the sun's disc, clouds drifting overhead, and the light the same sky
//! gives the meshes (`Sky::sun_light`, `Sky::environment`). While the
//! clouds drift the world draws; `--still` holds them, and a still sky
//! lets the window rest.
//! Run: cargo run --release -p zgx-3d --features media --example atmosphere
//!
//! `--sun DEGREES` sets the sun's height (default 25), `--png PATH` draws
//! one frame offscreen into a PNG and quits, `--smoke-test` quits after a
//! moment.
use zgx::{compose::prelude::*, scene::Color};
use zgx_3d::*;
use zgx_desktop::{Application, WindowOptions};

/// The sun's irradiance above the air, in the world's light units.
const SUN: f32 = 4.;

fn world(elevation: f32, still: bool) -> World {
    let world = World::new();
    let up = elevation.to_radians();
    let direction = -vec3(0.35 * up.cos(), up.sin(), -0.94 * up.cos()).normalized();
    let clouds = Clouds::new(0.45).height(1800.).size(3000.);
    let sky = Sky::new().turbidity(3.).clouds(Some(if still {
        clouds
    } else {
        clouds.drifting(25., [1., 0.3])
    }));
    world.add_light(sky.sun_light(direction, SUN));
    world.set_environment(Some(sky.environment(direction, SUN)));
    world.set_sky(Some(sky));
    world.set_shadows(true);
    world.set_tone_mapping(ToneMapping::Aces);
    world.add(Object::mesh(
        Geometry::plane(60., 60.),
        Material::rgb(0x6f6a60).with(0., 0.9),
    ));
    for (i, (x, z)) in [(-2.2, 0.), (0., -1.), (2.2, 0.4)].into_iter().enumerate() {
        let material = [
            Material::rgb(0xd9a441).with(1., 0.25),
            Material::rgb(0xe8e8e8).with(0., 0.5),
            Material::rgb(0x3a6ea5).with(0., 0.1),
        ][i];
        world.add(Object::mesh(Geometry::sphere(0.8, 48), material).at([x, 0.8, z]));
    }
    world.set_camera(Camera::looking_at([0., 1.6, 7.], [0., 2.2, -10.]));
    world
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let elevation = value("--sun").and_then(|v| v.parse().ok()).unwrap_or(25.);
    let still = args.iter().any(|a| a == "--still");
    if let Some(path) = value("--png") {
        return png(&world(elevation, true), &path);
    }
    let smoke = args.iter().any(|a| a == "--smoke-test");
    zgx_3d::request_view_features();
    Application::new()
        .window(WindowOptions {
            title: "zgx 3D: atmosphere".into(),
            width: 960.,
            height: 540.,
            ..Default::default()
        })
        .run(move |cx| {
            let world = world(elevation, still);
            cx.render(
                gpu_view("A world under a sky", world.view())
                    .w_full()
                    .h_full()
                    .bg(Color(0, 0, 0, 255)),
            );
            if smoke {
                let window = cx.window.clone();
                cx.tasks.spawn(async move {
                    zgx::timer::sleep(std::time::Duration::from_millis(1500)).await;
                    window.close();
                });
            }
        })
}

/// One frame of `world`, offscreen, into a PNG at `path`.
fn png(world: &World, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (960u32, 540u32);
    let mut ui = zgx::widgets::Ui::new(width as f32, height as f32);
    ui.mount(
        div()
            .size(width as f32, height as f32)
            .child(gpu_view("World", world.view()).size(width as f32, height as f32)),
    );
    let mut gpu = zgx_gpu::GpuRenderer::new(width, height)?;
    ui.prepare_frame();
    let report = ui.scene.borrow_mut().flush();
    gpu.render(&ui.scene.borrow(), &report.damage)?;
    let rgba = gpu.readback()?;
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&rgba)?;
    Ok(())
}
