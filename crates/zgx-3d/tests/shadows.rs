//! The sun's shadow cut into cascades (`lighting` feature): it falls where
//! the single map's does, sharper near the camera, and nowhere past the
//! cascades' reach; turning them off draws today's shadow again.
#![cfg(feature = "lighting")]
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

const SIDE: u32 = 128;

struct Window {
    ui: Ui,
    gpu: GpuRenderer,
}
impl Window {
    fn new(world: &World) -> Option<Self> {
        let gpu = match GpuRenderer::new(SIDE, SIDE) {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
            }
            Err(_) => return None,
        };
        let mut ui = Ui::new(SIDE as f32, SIDE as f32);
        ui.mount(
            div()
                .w_full()
                .h_full()
                .bg(Color(0, 0, 0, 255))
                .child(gpu_view("World", world.view()).w_full().h_full()),
        );
        Some(Self { ui, gpu })
    }
    fn draw(&mut self) -> Vec<u8> {
        self.ui.prepare_frame();
        let report = self.ui.scene.borrow_mut().flush();
        self.gpu
            .render(&self.ui.scene.borrow(), &report.damage)
            .unwrap();
        self.gpu.readback().unwrap()
    }
}

/// The pixel the world shows point `p` at.
fn seen_at(world: &World, p: Vec3) -> usize {
    let camera = world.camera();
    let ndc = (camera.projection(1.) * camera.view()).project(p);
    let x = ((ndc.x * 0.5 + 0.5) * SIDE as f32) as usize;
    let y = ((0.5 - ndc.y * 0.5) * SIDE as f32) as usize;
    (y * SIDE as usize + x) * 4
}

/// A floor and a tall box a sun shines straight down beside, from above
/// at 45° travelling toward -x: its shadow falls on the floor at -x.
fn scene() -> World {
    let world = World::new();
    world.add(Object::mesh(
        Geometry::plane(60., 60.),
        Material::rgb(0xc0c0c0).with(0., 0.9),
    ));
    world
        .add(Object::mesh(Geometry::cuboid(1., 3., 1.), Material::rgb(0xc0c0c0)).at([0., 1.5, 0.]));
    world.add_light(Light::Directional {
        direction: vec3(-1., -1., 0.),
        color: [1.; 3],
        intensity: 2.,
    });
    world.add_light(Light::Hemisphere {
        sky: [0.1; 3],
        ground: [0.1; 3],
        intensity: 1.,
    });
    world.set_shadows(true);
    world.set_camera(Camera::looking_at([0., 6., 9.], [1., 0., 0.]));
    world
}

#[test]
fn cascades_cast_the_shadow_the_single_map_does() {
    let world = scene();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let single = window.draw();
    world.set_cascades(Some(Cascades::new(4).distance(40.)));
    let cascaded = window.draw();
    // In the shadow (2 m along -x, within the 3 m it reaches) and out.
    let (shadow, lit) = (
        seen_at(&world, vec3(-2., 0., 0.)),
        seen_at(&world, vec3(2., 0., 0.)),
    );
    for (name, pixels) in [("single", &single), ("cascaded", &cascaded)] {
        assert!(
            pixels[shadow] + 40 < pixels[lit],
            "{name}: shadow {} lit {}",
            pixels[shadow],
            pixels[lit]
        );
    }
    // Alike but at the shadow's edges, which the cascades draw finer.
    let differing = single
        .iter()
        .zip(&cascaded)
        .filter(|(a, b)| a.abs_diff(**b) > 8)
        .count();
    assert!(
        differing < single.len() / 50,
        "{differing} channels differ past 8"
    );
    // Off: today's shadow, exactly.
    world.set_cascades(None);
    assert!(
        window.draw() == single,
        "turned off, the single map's frame"
    );
}

#[test]
fn nothing_past_the_cascades_reach_is_in_shadow() {
    let world = scene();
    // Reaching 2 m: the box's shadow, 9 m from the camera, is past them.
    world.set_cascades(Some(Cascades::new(2).distance(2.)));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let pixels = window.draw();
    let (shadow, lit) = (
        seen_at(&world, vec3(-2., 0., 0.)),
        seen_at(&world, vec3(2., 0., 0.)),
    );
    assert!(
        pixels[shadow].abs_diff(pixels[lit]) < 6,
        "shadow {} lit {}",
        pixels[shadow],
        pixels[lit]
    );
}
