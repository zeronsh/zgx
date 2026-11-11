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
    /// Frames until nothing keeps `world` drawing (its post passes made, on
    /// the compile worker): the last's pixels.
    fn compiled(&mut self, world: &World) -> Vec<u8> {
        for _ in 0..2000 {
            world.view().invalidate();
            let pixels = self.draw();
            if !world.view().animating() {
                return pixels;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("still compiling");
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
            most <= 40 && edges * 50 < side * side,
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

/// `world` drawn at `side` pixels a side.
fn drawn_at(world: &World, side: u32) -> Option<Vec<u8>> {
    let mut gpu = match GpuRenderer::new(side, side) {
        Ok(gpu) => gpu,
        Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
            panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
        }
        Err(_) => return None,
    };
    let mut ui = Ui::new(side as f32, side as f32);
    ui.mount(gpu_view("World", world.view()).size(side as f32, side as f32));
    ui.prepare_frame();
    let report = ui.scene.borrow_mut().flush();
    gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
    Some(gpu.readback().unwrap())
}

fn decode(v: u8) -> f32 {
    let c = f32::from(v) / 255.;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn encode(c: f32) -> u8 {
    let c = c.clamp(0., 1.);
    let v = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1. / 2.4) - 0.055
    };
    (v * 255.).round() as u8
}

/// The pixels of `big` (`k` times `side` a side) averaged k by k in linear
/// light: a supersampled frame at `side`.
fn shrunk(big: &[u8], side: u32, k: u32) -> Vec<u8> {
    let (side, k) = (side as usize, k as usize);
    let mut out = vec![0; side * side * 4];
    for y in 0..side {
        for x in 0..side {
            for c in 0..4 {
                let mut sum = 0.;
                for v in 0..k {
                    for u in 0..k {
                        let at = ((y * k + v) * side * k + x * k + u) * 4 + c;
                        sum += if c == 3 {
                            f32::from(big[at]) / 255.
                        } else {
                            decode(big[at])
                        };
                    }
                }
                let mean = sum / (k * k) as f32;
                out[(y * side + x) * 4 + c] = if c == 3 {
                    (mean * 255.).round() as u8
                } else {
                    encode(mean)
                };
            }
        }
    }
    out
}

/// A crop of `pixels` (`side` a side) as a PPM image, where
/// `ZGX_POST_CROPS` names a folder (for looking at edges by eye).
fn crop(pixels: &[u8], side: u32, (x0, y0, w): (u32, u32, u32), name: &str) {
    let Some(dir) = std::env::var_os("ZGX_POST_CROPS") else {
        return;
    };
    let mut ppm = format!("P6 {w} {w} 255\n").into_bytes();
    for y in y0..y0 + w {
        for x in x0..x0 + w {
            let at = ((y * side + x) * 4) as usize;
            ppm.extend_from_slice(&pixels[at..at + 3]);
        }
    }
    std::fs::write(std::path::Path::new(&dir).join(format!("{name}.ppm")), ppm).unwrap();
}

#[test]
fn a_bright_edge_is_as_smooth_with_the_chain_as_without() {
    // A highlight's edge, slanted, over a dim wall: glowing 16 times
    // brighter than white. Both lit, so both are mapped with the chain and
    // without. (Against the background, coverage already keeps an edge
    // smooth: the chain maps a pixel's colour, then its coverage.)
    let edge = |samples| {
        let world = World::new();
        let bright = Material {
            color: [0., 0., 0., 1.],
            emissive: [16., 16., 16.],
            ..Default::default()
        };
        let wall = Material::rgb(0x404040);
        world.add(Object::mesh(Geometry::cuboid(8., 8., 0.1), wall).at([0., 0., -0.5]));
        let turned = Quat::from_axis_angle(vec3(0., 0., 1.), 0.3);
        let id = world.add(Object::mesh(Geometry::cuboid(1.5, 1.5, 0.1), bright));
        world.update(id, |o| o.transform.rotation = turned);
        world.add_light(Light::Directional {
            direction: vec3(0., 0., -1.),
            color: [1., 1., 1.],
            intensity: 1.,
        });
        world.set_background_linear(Some([0.02, 0.02, 0.02, 1.]));
        world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
        world.set_samples(samples);
        world
    };
    let side = SIZE;
    let world = edge(4);
    let Some(ldr) = drawn_at(&world, side) else {
        return;
    };
    world.set_post(Post {
        vignette: Some(Vignette { intensity: 0. }),
        ..Default::default()
    });
    let chain = drawn_at(&world, side).unwrap();
    let reference = shrunk(&drawn_at(&edge(1), side * 4).unwrap(), side, 4);
    for (pixels, name) in [(&ldr, "ldr"), (&chain, "chain"), (&reference, "reference")] {
        crop(pixels, side, (20, 20, 56), name);
    }
    // Edges: pixels of the reference unlike a neighbour.
    let s = side as usize;
    let red = |p: &[u8], x: usize, y: usize| i32::from(p[(y * s + x) * 4 + 1]);
    let mut edges = Vec::new();
    for y in 1..s - 1 {
        for x in 1..s - 1 {
            let here = red(&reference, x, y);
            if [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
                .iter()
                .any(|&(u, v)| (red(&reference, u, v) - here).abs() > 24)
            {
                edges.push((x, y));
            }
        }
    }
    assert!(edges.len() > 40, "{} edge pixels", edges.len());
    // How far from the supersampled frame, over the edges: the chain as
    // close as the path without it (measured on Metal, mean of 460 edge
    // pixels: 7.10 with, 7.11 without; resolving the samples in linear
    // light first, 23.43; weighting them by 1 / (1 + luma), 10.33).
    let off = |p: &[u8]| {
        let sum: i32 = edges
            .iter()
            .map(|&(x, y)| (red(p, x, y) - red(&reference, x, y)).abs())
            .sum();
        sum as f32 / edges.len() as f32
    };
    let (ldr_off, chain_off) = (off(&ldr), off(&chain));
    assert!(
        chain_off <= ldr_off + 0.5,
        "on edges the chain is {chain_off:.2} off the reference, without it {ldr_off:.2}"
    );
}

#[test]
fn grading_with_nothing_changed_leaves_the_frame_and_each_part_does_its_own() {
    let color = [0.2, 0.05, 0.4, 1.];
    let world = flat(color);
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    let graded = |window: &mut Window, grade: Grade| {
        world.set_post(Post {
            grade: Some(grade),
            ..Default::default()
        });
        at(&window.draw(), SIZE / 2, SIZE / 2)
    };
    // The chain on, nothing graded: the colour as without it.
    world.set_post(Post {
        vignette: Some(Vignette { intensity: 0. }),
        ..Default::default()
    });
    let plain = at(&window.draw(), SIZE / 2, SIZE / 2);
    let near = |a: [u8; 4], b: [u8; 4], by: u8| a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= by);
    let identity = graded(
        &mut window,
        Grade {
            lut: Some(Lut::identity(17)),
            ..Default::default()
        },
    );
    assert!(near(identity, plain, 1), "{identity:?} against {plain:?}");
    let [r, g, b, _] = plain;
    // A table turning each channel over, from a .cube file.
    let mut cube = String::from("LUT_3D_SIZE 2\n");
    for i in 0..8 {
        cube += &format!("{} {} {}\n", 1 - (i & 1), 1 - ((i >> 1) & 1), 1 - (i >> 2));
    }
    let inverted = graded(
        &mut window,
        Grade {
            lut: Some(Lut::from_cube(&cube).unwrap()),
            ..Default::default()
        },
    );
    assert!(
        near(inverted, [255 - r, 255 - g, 255 - b, 255], 2),
        "{inverted:?} against {plain:?}"
    );
    let grey = graded(
        &mut window,
        Grade {
            saturation: 0.,
            ..Default::default()
        },
    );
    assert!(
        grey[0].abs_diff(grey[1]) <= 1 && grey[1].abs_diff(grey[2]) <= 1,
        "{grey:?}"
    );
    let halved = graded(
        &mut window,
        Grade {
            gain: [0.5; 3],
            ..Default::default()
        },
    );
    assert!(near(halved, [r / 2, g / 2, b / 2, 255], 1), "{halved:?}");
    let lifted = graded(
        &mut window,
        Grade {
            lift: [0.5; 3],
            ..Default::default()
        },
    );
    let up = |v: u8| v / 2 + 128;
    assert!(near(lifted, [up(r), up(g), up(b), 255], 1), "{lifted:?}");
    let brighter = graded(
        &mut window,
        Grade {
            gamma: [2.; 3],
            ..Default::default()
        },
    );
    let root = |v: u8| ((f32::from(v) / 255.).sqrt() * 255.).round() as u8;
    assert!(
        near(brighter, [root(r), root(g), root(b), 255], 1),
        "{brighter:?}"
    );
    // Warmer: red up, blue down, of a grey.
    let world = flat([0.2, 0.2, 0.2, 1.]);
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    world.set_post(Post {
        grade: Some(Grade {
            temperature: 0.6,
            ..Default::default()
        }),
        ..Default::default()
    });
    let [r, g, b, _] = at(&window.draw(), SIZE / 2, SIZE / 2);
    assert!(r > g + 4 && g > b + 4, "warm: {r} {g} {b}");
}

const SCALE: &str = "#zgx post 1
fn post(p: PostIn) -> vec4<f32> {
    return color(p.uv) * params.k;
}
";

#[test]
fn the_apps_passes_run_in_their_stage_in_turn_with_their_parameters() {
    let world = flat([0.2, 0.2, 0.2, 1.]);
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    // Each frame once the passes added are made.
    let centre = |window: &mut Window| at(&window.compiled(&world), SIZE / 2, SIZE / 2)[0];
    let plain = centre(&mut window);
    let scale = || PostPass::new(SCALE).param("k", PostParam::Float);
    // One HDR pass doubling the light (before mapping: linear, so its
    // encoding by about 1.37 here).
    let double = world.add_post(scale()).unwrap();
    world.set_post_param(double, "k", 2.).unwrap();
    let doubled = centre(&mut window);
    let expected = |linear: f32| {
        let v = if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1. / 2.4) - 0.055
        };
        (v.clamp(0., 1.) * 255.).round() as u8
    };
    assert!(
        doubled.abs_diff(expected(0.4)) <= 1,
        "{doubled} for {plain}"
    );
    // A second halving it: back to the light it began with (an even count,
    // ending in the scene with no copy).
    let halve = world.add_post(scale()).unwrap();
    world.set_post_param(halve, "k", 0.5).unwrap();
    assert!(centre(&mut window).abs_diff(plain) <= 1);
    // A display pass turning what's shown over, encoded by it as the last.
    let invert = world
        .add_post(
            PostPass::new(
                "#zgx post 1
fn post(p: PostIn) -> vec4<f32> {
    let c = color(p.uv);
    return vec4(vec3(1.0) - c.rgb, 1.0);
}
",
            )
            .stage(PostStage::Display),
        )
        .unwrap();
    let inverted = centre(&mut window);
    assert!(inverted.abs_diff(expected(0.8)) <= 1, "{inverted}");
    // Errors: a parameter of another type, or not the pass's.
    assert!(matches!(
        world.set_post_param(double, "k", [1., 2.]),
        Err(PostError::Param(_))
    ));
    assert!(matches!(
        world.set_post_param(double, "j", 1.),
        Err(PostError::Param(_))
    ));
    // Out again, the frame as without them.
    for pass in [double, halve, invert] {
        world.remove_post(pass);
    }
    assert_eq!(world.set_post_param(double, "k", 1.), Err(PostError::Pass));
    assert_eq!(centre(&mut window), plain);
}

#[test]
fn an_animated_pass_keeps_its_world_drawing_until_it_goes() {
    // Its view animates (a frame every refresh while shown: the host's,
    // tested in zgx) while the pass is in, and only then.
    let world = flat([0.2, 0.2, 0.2, 1.]);
    let still = world
        .add_post(PostPass::new(SCALE).param("k", PostParam::Float))
        .unwrap();
    assert!(!world.view().animating(), "a pass not animated");
    let pulse = world
        .add_post(
            PostPass::new(
                "#zgx post 1
fn post(p: PostIn) -> vec4<f32> {
    return color(p.uv) * (1.0 + 0.5 * sin(frame.time));
}
",
            )
            .animated(),
        )
        .unwrap();
    assert!(world.view().animating());
    world.remove_post(still);
    assert!(world.view().animating(), "the animated one still in");
    world.remove_post(pulse);
    assert!(!world.view().animating());
}

#[test]
fn automatic_exposure_brings_a_dim_and_a_bright_scene_to_middle_grey_and_settles() {
    let shown = |level: f32, auto: bool| {
        let world = flat([level, level, level, 1.]);
        world.set_post(Post {
            auto_exposure: auto.then(AutoExposure::default),
            vignette: Some(Vignette { intensity: 0. }),
            ..Default::default()
        });
        let mut window = Window::new(&world)?;
        // Frames until its exposure reads back settled (the first frame
        // adapts at once; the readback comes two frames later).
        let mut frames = 0;
        loop {
            frames += 1;
            world.set_camera(Camera::looking_at(
                [0., 0., 4. + frames as f32 * 1e-4],
                [0., 0., 0.],
            ));
            let pixels = window.draw();
            if !world.view().animating() || frames == 12 {
                return Some((at(&pixels, SIZE / 2, SIZE / 2)[0], frames));
            }
        }
    };
    let Some((dim, settled)) = shown(0.01, true) else {
        return;
    };
    let (bright, _) = shown(4., true).unwrap();
    assert!(settled < 12, "settled after {settled} frames");
    // Middle grey (0.18), mapped linearly and encoded: 118. (The bright
    // scene's light past 1 survives the resolve to be measured.)
    for v in [dim, bright] {
        assert!(v.abs_diff(118) <= 3, "{dim} and {bright}");
    }
    let (dim, _) = shown(0.01, false).unwrap();
    let (bright, _) = shown(4., false).unwrap();
    assert!(dim < 40 && bright > 240, "without: {dim} and {bright}");
}

#[test]
fn a_pixel_not_finite_spreads_through_neither_bloom_nor_the_exposure() {
    // One pixel of the scene made NaN by an HDR pass (as an infinite light
    // would), against the same pixel made black: the frames alike.
    let frame = |value: &str| {
        let world = scene();
        world.set_post(Post {
            bloom: Some(Bloom { intensity: 0.2 }),
            auto_exposure: Some(AutoExposure::default()),
            ..Default::default()
        });
        world
            .add_post(PostPass::new(format!(
                "#zgx post 1
fn post(p: PostIn) -> vec4<f32> {{
    if all(floor(p.position) == vec2(48.0, 40.0)) {{
        return {value};
    }}
    return color(p.uv);
}}
"
            )))
            .unwrap();
        let mut window = Window::new(&world)?;
        Some(window.draw())
    };
    let Some(nan) = frame("vec4(bitcast<f32>(0x7fc00000u))") else {
        return;
    };
    let black = frame("vec4(0.0)").unwrap();
    let side = SIZE as usize;
    let mut most = 0;
    let mut far = 0;
    for (i, (a, b)) in nan.chunks(4).zip(black.chunks(4)).enumerate() {
        let (x, y) = (i % side, i / side);
        let by = a.iter().zip(b).map(|(p, q)| p.abs_diff(*q)).max().unwrap();
        most = most.max(by);
        if x.abs_diff(48) > 3 || y.abs_diff(40) > 3 {
            far = far.max(by);
        }
    }
    // Bloom's filtered taps touching it read as nothing whole, a little
    // more than the pixel alone: a level at most beyond its neighbours.
    // (Unguarded, the NaN takes bloom and the exposure, and the frame, with
    // it.)
    assert!(
        far <= 1,
        "beyond its neighbours, the frame as with black: by {far}"
    );
    assert!(most <= 4, "and near it, by {most}");
}

#[test]
fn a_pass_returning_less_than_nothing_is_taken_as_nothing() {
    // Over a background: a negative alpha would let more of it through
    // than there is.
    let shown = |value: &str| {
        let world = World::new();
        world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
        world.set_tone_mapping(ToneMapping::Linear);
        world.set_background_linear(Some([0.2, 0.2, 0.2, 1.]));
        let source =
            format!("#zgx post 1\nfn post(p: PostIn) -> vec4<f32> {{ return vec4({value}); }}\n");
        world.add_post(PostPass::new(source)).unwrap();
        let mut window = Window::new(&world)?;
        Some(at(&window.compiled(&world), SIZE / 2, SIZE / 2))
    };
    let Some(nothing) = shown("0.0") else {
        return;
    };
    assert_eq!(shown("-0.5"), Some(nothing));
}
