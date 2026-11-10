//! The post chain (`World::set_post`): with it on and nothing mixed in, a
//! world draws as it does without; bloom spreads light without adding any;
//! its targets go as the world rests.
#![cfg(feature = "post")]
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

const SIZE: u32 = 96;

struct Window {
    ui: Ui,
    gpu: GpuRenderer,
}
impl Window {
    fn new(world: &World) -> Option<Self> {
        let gpu = match GpuRenderer::new(SIZE, SIZE) {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
            }
            Err(_) => return None,
        };
        let mut ui = Ui::new(SIZE as f32, SIZE as f32);
        ui.mount(
            div()
                .size(SIZE as f32, SIZE as f32)
                .bg(Color(0, 0, 0, 255))
                .child(gpu_view("World", world.view()).size(SIZE as f32, SIZE as f32)),
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

/// A lit sphere half in front of a lit wall, over a background.
fn scene() -> World {
    let world = World::new();
    world.add(Object::mesh(Geometry::sphere(1., 48), Material::rgb(0xcc5533)).at([0.4, 0., 0.]));
    world.add(
        Object::mesh(Geometry::cuboid(1.5, 1.5, 0.2), Material::rgb(0x88aadd)).at([-1., 0.6, -1.]),
    );
    world.add_light(Light::Directional {
        direction: vec3(-0.4, -0.6, -1.),
        color: [1., 1., 1.],
        intensity: 4.,
    });
    world.set_background(Some(0x203040));
    world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
    world
}

/// The largest difference of any channel, and how many pixels differ by
/// more than `by`.
fn differ(a: &[u8], b: &[u8], by: u8) -> (u8, usize) {
    let most = a
        .iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0);
    let pixels = a
        .chunks(4)
        .zip(b.chunks(4))
        .filter(|(p, q)| p.iter().zip(*q).any(|(x, y)| x.abs_diff(*y) > by))
        .count();
    (most, pixels)
}

#[test]
fn with_the_chain_on_and_nothing_mixed_in_a_world_draws_as_without() {
    for (tone, stops) in [
        (ToneMapping::Neutral, 0.),
        (ToneMapping::Aces, 1.),
        (ToneMapping::Linear, -0.5),
    ] {
        let world = scene();
        world.set_tone_mapping(tone);
        world.set_exposure(stops);
        let Some(mut window) = Window::new(&world) else {
            return;
        };
        let without = window.draw();
        world.set_post(Post {
            bloom: Some(Bloom { intensity: 0. }),
            ..Default::default()
        });
        let with = window.draw();
        // Where a pixel and those around it are alike, the same; mapped
        // after the samples of a pixel are averaged, not before, edges
        // between meshes may differ (bright beside dark the most).
        let side = SIZE as usize;
        let px = |p: &[u8], x: usize, y: usize| -> [u8; 4] {
            p[(y * side + x) * 4..][..4].try_into().unwrap()
        };
        let mut edges = 0;
        for y in 1..side - 1 {
            for x in 1..side - 1 {
                let here = px(&without, x, y);
                let alike = (y - 1..=y + 1).all(|v| {
                    (x - 1..=x + 1).all(|u| {
                        let there = px(&without, u, v);
                        here.iter().zip(&there).all(|(a, b)| a.abs_diff(*b) <= 3)
                    })
                });
                let by = here
                    .iter()
                    .zip(&px(&with, x, y))
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap();
                if alike {
                    assert!(by <= 1, "{tone:?}, {stops}: by {by} at {x}, {y}");
                } else if by > 1 {
                    edges += 1;
                }
            }
        }
        let (most, _) = differ(&without, &with, 1);
        assert!(
            most <= 64 && edges * 50 < side * side,
            "{tone:?}, {stops}: by {most} at most, {edges} edge pixels by more than 1"
        );
        // Off again, exactly as before.
        world.set_post(Post::default());
        assert_eq!(window.draw(), without, "{tone:?}");
    }
}

#[test]
fn bloom_spreads_light_around_what_is_bright_and_adds_none() {
    // A small bright square on black.
    let world = World::new();
    let bright = Material {
        color: [8., 8., 8., 1.],
        unlit: true,
        ..Default::default()
    };
    world.add(Object::mesh(Geometry::cuboid(0.3, 0.3, 0.3), bright));
    world.set_background_linear(Some([0., 0., 0., 1.]));
    world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
    world.set_tone_mapping(ToneMapping::Linear);
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    world.set_post(Post {
        bloom: Some(Bloom { intensity: 0. }),
        ..Default::default()
    });
    let sharp = window.draw();
    world.set_post(Post {
        bloom: Some(Bloom { intensity: 0.2 }),
        ..Default::default()
    });
    let glowing = window.draw();
    let at = |pixels: &[u8], x: u32, y: u32| pixels[((y * SIZE + x) * 4) as usize];
    let (centre, near) = (SIZE / 2, SIZE / 2 + 12);
    assert_eq!(at(&sharp, centre, near), 0, "black beside it, unbloomed");
    assert!(
        at(&glowing, centre, near) > 10,
        "light beside it: {}",
        at(&glowing, centre, near)
    );
    assert!(at(&glowing, centre, near) > at(&glowing, centre, near + 20));

    // A view filled by one colour stays it: bloom mixes, it doesn't add.
    let world = World::new();
    let flat = Material {
        color: [0.3, 0.2, 0.1, 1.],
        unlit: true,
        ..Default::default()
    };
    world.add(Object::mesh(Geometry::cuboid(20., 20., 0.1), flat));
    world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
    world.set_tone_mapping(ToneMapping::Linear);
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let plain = window.draw();
    world.set_post(Post {
        bloom: Some(Bloom { intensity: 1. }),
        ..Default::default()
    });
    let (most, _) = differ(&plain, &window.draw(), 0);
    assert!(most <= 1, "by {most}");
}

#[test]
fn the_chains_targets_go_as_the_world_rests() {
    let world = scene();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    let plain = world.temporary_bytes();
    world.set_post(Post {
        bloom: Some(Bloom::default()),
        ..Default::default()
    });
    let held = window.draw();
    // Without: four samples of colour and of depth, 4 bytes each. With, the
    // samples' colour in 8 bytes, the scene one sample of 8, and bloom's
    // steps from half its size: 4 bytes a texel where the device draws
    // into three floats packed so, else 8.
    let pixels = u64::from(SIZE * SIZE);
    assert_eq!(plain, pixels * 4 * (4 + 4));
    let steps: u64 = (0..5).map(|l| u64::from((SIZE / 2) >> l).pow(2)).sum();
    let chain = world.temporary_bytes();
    assert!(
        [4, 8].contains(&((chain - pixels * (4 * (8 + 4) + 8)) / steps)),
        "{chain}"
    );
    assert_eq!((chain - pixels * (4 * (8 + 4) + 8)) % steps, 0);
    window.gpu.trim();
    assert_eq!(world.temporary_bytes(), 0);
    // A change and its undoing: a frame, of the world as it was.
    world.set_exposure(1.);
    world.set_exposure(0.);
    assert_eq!(window.draw(), held, "the same frame after resting");
    assert_eq!(world.temporary_bytes(), chain, "drawn again");
    // Off, only what the meshes draw into.
    world.set_post(Post::default());
    window.draw();
    assert_eq!(world.temporary_bytes(), plain);
}

/// A world of one unlit colour filling the view, mapped as it is.
fn flat(color: [f32; 4]) -> World {
    let world = World::new();
    let surface = Material {
        color,
        unlit: true,
        ..Default::default()
    };
    world.add(Object::mesh(Geometry::cuboid(20., 20., 0.1), surface));
    world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
    world.set_tone_mapping(ToneMapping::Linear);
    world
}

fn at(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    pixels[((y * SIZE + x) * 4) as usize..][..4]
        .try_into()
        .unwrap()
}

#[test]
fn a_vignette_darkens_the_corners_and_leaves_the_centre() {
    let world = flat([0.5, 0.5, 0.5, 1.]);
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let plain = window.draw();
    world.set_post(Post {
        vignette: Some(Vignette { intensity: 1. }),
        ..Default::default()
    });
    let shaded = window.draw();
    let (centre, corner) = (SIZE / 2, 0);
    let by = |p: [u8; 4], q: [u8; 4]| p[0].abs_diff(q[0]);
    assert!(by(at(&plain, centre, centre), at(&shaded, centre, centre)) <= 1);
    // A quarter of the light at the corners: in sRGB, about 0.54 of the
    // centre's value.
    let ratio =
        f32::from(at(&shaded, corner, corner)[0]) / f32::from(at(&plain, corner, corner)[0]);
    assert!((0.5..0.62).contains(&ratio), "{ratio}");
    // Halfway along an edge, between the two.
    let edge = at(&shaded, corner, centre)[0];
    assert!(edge > at(&shaded, corner, corner)[0] && edge < at(&shaded, centre, centre)[0]);
}

#[test]
fn grain_is_noise_of_the_frames_index_about_the_tone() {
    let grainy = || {
        let world = flat([0.2, 0.2, 0.2, 1.]);
        world.set_post(Post {
            grain: Some(Grain { intensity: 0.2 }),
            ..Default::default()
        });
        world
    };
    let (world, again) = (grainy(), grainy());
    let (Some(mut window), Some(mut other)) = (Window::new(&world), Window::new(&again)) else {
        return;
    };
    let first = window.draw();
    // A frame of another index: other noise.
    world.set_exposure(1.);
    world.set_exposure(0.);
    let second = window.draw();
    assert_ne!(first, second);
    // The same index in another view: the same noise.
    assert_eq!(other.draw(), first);
    // About the colour without it, and varied.
    let plain = {
        let world = flat([0.2, 0.2, 0.2, 1.]);
        let Some(mut window) = Window::new(&world) else {
            return;
        };
        window.draw()[0]
    };
    let reds: Vec<u8> = first.chunks(4).map(|p| p[0]).collect();
    let mean = reds.iter().map(|&r| f32::from(r)).sum::<f32>() / reds.len() as f32;
    assert!((mean - f32::from(plain)).abs() < 2., "{mean} about {plain}");
    let (low, high) = (reds.iter().min().unwrap(), reds.iter().max().unwrap());
    assert!(high - low >= 10, "{low} to {high}");
}

#[test]
fn chromatic_aberration_parts_red_outward_and_blue_inward() {
    // A white bar right of the centre, on black.
    let world = World::new();
    let white = Material {
        color: [1., 1., 1., 1.],
        unlit: true,
        ..Default::default()
    };
    world.add(Object::mesh(Geometry::cuboid(0.6, 4., 0.1), white).at([1.2, 0., 0.]));
    world.set_background_linear(Some([0., 0., 0., 1.]));
    world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
    world.set_tone_mapping(ToneMapping::Linear);
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    world.set_post(Post {
        chromatic_aberration: Some(ChromaticAberration { intensity: 1. }),
        ..Default::default()
    });
    let parted = window.draw();
    let row = SIZE / 2;
    let lit: Vec<u32> = (0..SIZE)
        .filter(|&x| at(&parted, x, row)[1] > 128)
        .collect();
    let (left, right) = (lit[0], *lit.last().unwrap());
    assert!(left > SIZE / 2, "right of the centre: {left}");
    // Just outside its outer edge, red; just inside its inner one, blue.
    let [r, _, b, _] = at(&parted, right + 1, row);
    assert!(r > b + 20, "outer edge: {r} red, {b} blue");
    let [r, _, b, _] = at(&parted, left - 1, row);
    assert!(b > r + 20, "inner edge: {r} red, {b} blue");
}
