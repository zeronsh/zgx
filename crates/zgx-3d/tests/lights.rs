//! Lights past the uniform path's eight (`lighting` feature): every one
//! lights what it reaches, and clustered shading draws what the uniform
//! path draws for the lights both shade.
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

fn floor(world: &World) {
    world.add(Object::mesh(
        Geometry::plane(20., 20.),
        Material::rgb(0xb0b0b0).with(0., 0.9),
    ));
}

#[test]
fn every_one_of_sixteen_lights_lights_the_floor_under_it() {
    let world = World::new();
    floor(&world);
    world.set_camera(Camera::looking_at([0., 10., 4.], [0., 0., 0.]));
    // A 4 by 4 grid, 2 apart, each reaching 1: lit under each, dark
    // between.
    let under: Vec<Vec3> = (0..16)
        .map(|i| vec3((i % 4) as f32 * 2. - 3., 0., (i / 4) as f32 * 2. - 3.))
        .collect();
    for p in &under {
        world.add_light(Light::Point {
            position: *p + vec3(0., 0.5, 0.),
            color: [1., 0.8, 0.6],
            intensity: 2.,
            range: 1.,
        });
    }
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let pixels = window.draw();
    for (i, p) in under.iter().enumerate() {
        let lit = &pixels[seen_at(&world, *p)..][..4];
        assert!(lit[0] > 100, "light {i} lights nothing under it: {lit:?}");
        // Between it and the next along x: out of every light's reach.
        let between = &pixels[seen_at(&world, *p + vec3(1., 0., 1.))..][..4];
        assert!(between[0] < 4, "lit between lights: {between:?}");
    }
}

/// A scene of eight point and spot lights of different reaches over a floor
/// and two meshes, a sun casting shadows and a hemisphere.
fn eight_lights() -> World {
    let world = World::new();
    floor(&world);
    world.add(
        Object::mesh(
            Geometry::sphere(0.6, 32),
            Material::rgb(0xd04030).with(0., 0.4),
        )
        .at([-0.8, 0.6, 0.]),
    );
    world.add(
        Object::mesh(
            Geometry::cuboid(0.9, 0.9, 0.9),
            Material::rgb(0x3060c0).with(0.8, 0.3),
        )
        .at([0.9, 0.45, -0.4]),
    );
    world.add_light(Light::Directional {
        direction: vec3(-0.4, -1., -0.3),
        color: [1., 0.97, 0.92],
        intensity: 1.,
    });
    world.add_light(Light::Hemisphere {
        sky: [0.2, 0.25, 0.3],
        ground: [0.05, 0.04, 0.03],
        intensity: 1.,
    });
    world.set_shadows(true);
    for i in 0..8 {
        let a = i as f32 * 0.785;
        let light = world.add_light(Light::Point {
            position: vec3(a.cos() * 2.2, 0.4 + (i % 3) as f32 * 0.5, a.sin() * 2.2),
            color: [1., 0.5 + 0.06 * i as f32, 0.9 - 0.08 * i as f32],
            intensity: 2. + i as f32,
            range: 1.5 + (i % 4) as f32 * 1.1,
        });
        if i % 3 == 0 {
            world.set_spot(light, Some(Spot::new([-a.cos(), -1., -a.sin()], 0.3, 0.6)));
        }
    }
    world.set_background(Some(0x203040));
    world
}

#[test]
fn clustered_lights_draw_what_the_uniform_path_draws() {
    let world = eight_lights();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let cameras = [
        Camera::looking_at([0.5, 2.5, 5.], [0., 0.3, 0.]),
        // Low, along the floor, through many slices.
        Camera::looking_at([4., 0.7, 3.], [-2., 0.2, -2.]),
        Camera {
            orthographic: true,
            fov: 7.,
            ..Camera::looking_at([3., 4., 5.], [0., 0., 0.])
        },
    ];
    for environment in [false, true] {
        world.set_environment(environment.then(Environment::studio));
        for camera in cameras {
            world.set_camera(camera);
            world.set_clustered_lights(false);
            let uniform = window.draw();
            world.set_clustered_lights(true);
            let clustered = window.draw();
            let (mut count, mut largest) = (0, 0);
            for (a, b) in uniform.iter().zip(&clustered) {
                let d = a.abs_diff(*b);
                largest = largest.max(d);
                count += usize::from(d > 1);
            }
            assert!(
                count == 0,
                "{camera:?}, environment {environment}: {count} channels past 1, by up to {largest}"
            );
            // The control: the lights showed.
            let lit = clustered
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[0] > 120)
                .count();
            assert!(lit > 200, "{camera:?}: {lit} bright pixels");
        }
    }
}

#[test]
fn a_light_added_moved_and_removed_relights_as_a_fresh_world_draws() {
    // 40 lights: clustered whatever is asked.
    let lights = |world: &World| -> Vec<LightId> {
        (0..40)
            .map(|i| {
                let (x, z) = ((i % 8) as f32 - 3.5, (i / 8) as f32 - 2.);
                world.add_light(Light::Point {
                    position: vec3(x, 0.4, z),
                    color: [1., 0.7, 0.4],
                    intensity: 1.5,
                    range: 0.9,
                })
            })
            .collect()
    };
    let camera = Camera::looking_at([0., 6., 5.], [0., 0., 0.]);
    let fresh = World::new();
    floor(&fresh);
    lights(&fresh);
    fresh.set_camera(camera);
    let Some(mut window) = Window::new(&fresh) else {
        return;
    };
    let expected = window.draw();
    drop(window);

    let world = World::new();
    floor(&world);
    let ids = lights(&world);
    let mut window = Window::new(&world).unwrap();
    world.set_camera(Camera::looking_at([2., 3., 6.], [0., 0., 0.]));
    window.draw();
    let moved = Light::Point {
        position: vec3(0., 0.4, 0.),
        color: [0., 1., 0.],
        intensity: 4.,
        range: 3.,
    };
    world.set_light(ids[9], moved);
    let extra = world.add_light(moved);
    let changed = window.draw();
    world.set_camera(camera);
    window.draw();
    world.remove_light(extra);
    world.set_light(ids[9], {
        let (x, z) = ((9 % 8) as f32 - 3.5, (9 / 8) as f32 - 2.);
        Light::Point {
            position: vec3(x, 0.4, z),
            color: [1., 0.7, 0.4],
            intensity: 1.5,
            range: 0.9,
        }
    });
    let actual = window.draw();
    assert_ne!(changed, expected, "the control: the changes showed");
    let differing = actual
        .iter()
        .zip(&expected)
        .filter(|(a, b)| a.abs_diff(**b) > 1)
        .count();
    assert_eq!(differing, 0, "a retained world's lights went stale");
}
