//! Global illumination (`gi` feature): a test room built in code (a
//! Cornell-style box: white floor, ceiling and back wall, a red wall on the
//! left, a green one on the right, open toward the camera, a lamp under the
//! ceiling, a white block) lit with and without a probe volume.
//!
//! `ZGX_GI_OUT=<folder>` writes each image drawn there as a PNG.
use zgx::{compose::prelude::*, scene::Color};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

const SIZE: (u32, u32) = (160, 160);

struct Window {
    ui: zgx::widgets::Ui,
    gpu: GpuRenderer,
}
impl Window {
    fn new(world: &World) -> Option<Self> {
        // SAFETY: a test of ray-traced probes takes on wgpu's experimental
        // contract (`request_ray_queries`).
        unsafe { zgx_gpu::request_ray_queries() };
        let gpu = match GpuRenderer::new(SIZE.0, SIZE.1) {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
            }
            Err(_) => return None,
        };
        let mut ui = zgx::widgets::Ui::new(SIZE.0 as f32, SIZE.1 as f32);
        ui.mount(
            div()
                .w_full()
                .h_full()
                .bg(Color(0, 0, 0, 255))
                .child(gpu_view("World", world.view()).w_full().h_full()),
        );
        Some(Self { ui, gpu })
    }
    fn draw(&mut self) {
        self.ui.prepare_frame();
        let report = self.ui.scene.borrow_mut().flush();
        self.gpu
            .render(&self.ui.scene.borrow(), &report.damage)
            .unwrap();
    }
    fn pixels(&self) -> Vec<u8> {
        self.gpu.readback().unwrap()
    }
}

fn save(name: &str, rgba: &[u8]) {
    let Some(dir) = std::env::var_os("ZGX_GI_OUT") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(dir.join(format!("{name}.png"))).unwrap();
    let mut encoder = png::Encoder::new(file, SIZE.0, SIZE.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap();
}

/// The room's walls, block and lamp; the lamp's id.
fn room(world: &World) -> LightId {
    let white = Material {
        color: [0.73, 0.73, 0.73, 1.],
        roughness: 1.,
        ..Material::default()
    };
    let red = Material {
        color: [0.63, 0.065, 0.05, 1.],
        ..white
    };
    let green = Material {
        color: [0.14, 0.45, 0.091, 1.],
        ..white
    };
    let slab = |size: [f32; 3], at: [f32; 3], material: Material| {
        world.add(Object::mesh(Geometry::cuboid(size[0], size[1], size[2]), material).at(at));
    };
    slab([2.2, 0.1, 2.2], [0., -0.05, 0.], white); // floor
    slab([2.2, 0.1, 2.2], [0., 2.05, 0.], white); // ceiling
    slab([2.2, 2.2, 0.1], [0., 1., -1.05], white); // back
    slab([0.1, 2.2, 2.2], [-1.05, 1., 0.], red); // left
    slab([0.1, 2.2, 2.2], [1.05, 1., 0.], green); // right
    slab([0.6, 1.2, 0.6], [0.35, 0.6, -0.3], white); // block
    world.set_camera(Camera {
        position: vec3(0., 1., 3.4),
        target: vec3(0., 1., 0.),
        fov: 40f32.to_radians(),
        ..Camera::default()
    });
    world.set_background(Some(0x000000));
    world.add_light(Light::Point {
        position: vec3(0., 1.85, 0.),
        color: [1., 0.9, 0.75],
        intensity: 4.,
        range: 10.,
    })
}

fn volume() -> ProbeVolume {
    ProbeVolume::new([-0.95, 0.05, -0.95], [0.95, 1.95, 0.95], [6, 6, 6])
}

/// A pixel's RGB at `(x, y)` in a `SIZE` image, as floats 0 to 1.
fn at(rgba: &[u8], x: u32, y: u32) -> [f32; 3] {
    let i = ((y * SIZE.0 + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2]].map(|v| f32::from(v) / 255.)
}
/// The mean RGB of a `side` square about `(x, y)`.
fn around(rgba: &[u8], (x, y): (u32, u32), side: u32) -> [f32; 3] {
    let mut sum = [0.; 3];
    for dy in 0..side {
        for dx in 0..side {
            let c = at(rgba, x + dx - side / 2, y + dy - side / 2);
            for i in 0..3 {
                sum[i] += c[i];
            }
        }
    }
    sum.map(|v| v / (side * side) as f32)
}

/// Draws as a window's frame clock would while the world's view animates
/// (this window has none), until it stops (at most `most` frames): the
/// frames it drew.
fn settle(window: &mut Window, world: &World, most: u32) -> u64 {
    let start = world.frames_drawn();
    window.draw();
    for _ in 0..most {
        if !world.view().animating() {
            break;
        }
        world.view().invalidate();
        window.draw();
    }
    world.frames_drawn() - start
}

#[test]
fn probes_carry_the_walls_colours_into_the_room_and_light_what_the_lamp_misses() {
    let world = World::new();
    room(&world);
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    let direct = window.pixels();
    save("room-direct", &direct);
    let probes = world.add_probes(volume()).unwrap();
    let frames = settle(&mut window, &world, 2000);
    if !world.probes_traced() {
        eprintln!("no ray queries on this device: probes untested");
        return;
    }
    let lit = window.pixels();
    save("room-probes", &lit);
    eprintln!("settled after {frames} frames");
    // The block's face toward the camera: edge-on to the lamp, so dark by
    // direct light alone; the floor and walls light it.
    let face = (102, 110);
    let (before, after) = (around(&direct, face, 6), around(&lit, face, 6));
    eprintln!("block face {before:?} -> {after:?}");
    assert!(
        before[1] < 0.05 && after[1] > before[1] + 0.15,
        "{before:?} {after:?}"
    );
    // The floor by the red wall is redder than by the green, and the
    // other way round.
    let (left, right) = ((40, 128), (120, 128));
    let (l, r) = (around(&lit, left, 6), around(&lit, right, 6));
    let (dl, dr) = (around(&direct, left, 6), around(&direct, right, 6));
    eprintln!("floor left {dl:?} -> {l:?}, right {dr:?} -> {r:?}");
    assert!(l[0] / l[1] > dl[0] / dl[1] * 1.05, "{l:?} {dl:?}");
    assert!(r[1] / r[0] > dr[1] / dr[0] * 1.05, "{r:?} {dr:?}");
    // Removed, the room draws as it did before.
    world.remove_probes(probes);
    window.draw();
    let again = window.pixels();
    assert_eq!(again, direct);
}

#[test]
fn a_still_room_stops_drawing_once_its_probes_converge_and_starts_again_as_the_lamp_moves() {
    let world = World::new();
    let lamp = room(&world);
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    world.add_probes(volume().per_frame(216)).unwrap();
    let frames = settle(&mut window, &world, 500);
    if !world.probes_traced() {
        return;
    }
    // Every probe once a frame: a round a frame.
    assert!(frames <= 20, "{frames} frames to converge");
    let drawn = world.frames_drawn();
    for _ in 0..10 {
        window.draw();
    }
    assert_eq!(
        world.frames_drawn(),
        drawn,
        "a converged room draws nothing"
    );
    world.set_light(
        lamp,
        Light::Point {
            position: vec3(-0.5, 1.85, 0.),
            color: [1., 0.9, 0.75],
            intensity: 4.,
            range: 10.,
        },
    );
    let frames = settle(&mut window, &world, 500);
    assert!(
        (2..=20).contains(&frames),
        "{frames} frames after the lamp moved"
    );
}

#[test]
fn a_room_reached_through_changes_draws_as_a_fresh_one() {
    // Fresh: the room and its probes, drawn until settled.
    let fresh_world = World::new();
    room(&fresh_world);
    fresh_world.add_probes(volume()).unwrap();
    let Some(mut fresh) = Window::new(&fresh_world) else {
        return;
    };
    settle(&mut fresh, &fresh_world, 2000);
    if !fresh_world.probes_traced() {
        return;
    }
    let expected = fresh.pixels();
    // Retained: another volume first, the lamp elsewhere, a block added and
    // removed, the volume swapped, each settled or not before the next.
    let world = World::new();
    let lamp = room(&world);
    let mut window = Window::new(&world).unwrap();
    let first = world
        .add_probes(ProbeVolume::new(
            [-0.9, 0.1, -0.9],
            [0.9, 1.9, 0.9],
            [3, 3, 3],
        ))
        .unwrap();
    settle(&mut window, &world, 2000);
    world.set_light(
        lamp,
        Light::Point {
            position: vec3(0.6, 1.2, 0.5),
            color: [0.2, 0.4, 1.],
            intensity: 9.,
            range: 10.,
        },
    );
    settle(&mut window, &world, 10);
    let block = world.add(
        Object::mesh(Geometry::cuboid(0.5, 0.5, 0.5), Material::rgb(0xffff00))
            .at([-0.4, 0.25, 0.2]),
    );
    settle(&mut window, &world, 30);
    world.remove(block);
    world.remove_probes(first);
    world.add_probes(volume()).unwrap();
    world.set_light(
        lamp,
        Light::Point {
            position: vec3(0., 1.85, 0.),
            color: [1., 0.9, 0.75],
            intensity: 4.,
            range: 10.,
        },
    );
    settle(&mut window, &world, 2000);
    let drawn = window.pixels();
    save("room-retained", &drawn);
    let differing = drawn
        .iter()
        .zip(&expected)
        .filter(|(a, b)| a.abs_diff(**b) > 1)
        .count();
    let worst = drawn
        .iter()
        .zip(&expected)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(
        differing == 0,
        "{differing} channels differ past 1, by up to {worst}"
    );
}

/// Which goldens this adapter is compared with (as `reference.rs` names
/// them).
fn family(gpu: &GpuRenderer) -> String {
    let info = gpu.adapter_info();
    let name = info.name.to_lowercase();
    if name.contains("llvmpipe") {
        "llvmpipe".into()
    } else if name.contains("basic render") || name.contains("warp") {
        "warp".into()
    } else if info.backend == zgx_gpu::wgpu::Backend::Metal {
        "metal".into()
    } else {
        let name: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        format!("{:?}-{name}", info.backend).to_lowercase()
    }
}

#[test]
fn the_probe_lit_room_matches_its_golden() {
    let world = World::new();
    room(&world);
    world.add_probes(volume()).unwrap();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    settle(&mut window, &world, 2000);
    if !world.probes_traced() {
        return;
    }
    let actual = window.pixels();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/reference")
        .join(family(&window.gpu))
        .join("gi_room.png");
    if std::env::var_os("ZGX_BLESS").is_some_and(|v| v == "1") {
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = png::Encoder::new(file, SIZE.0, SIZE.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::High);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&actual)
            .unwrap();
        return;
    }
    if !path.exists() {
        let required = std::env::var_os("ZGX_REQUIRE_GOLDENS").is_some();
        assert!(
            !required,
            "no golden {}; bless with ZGX_BLESS=1",
            path.display()
        );
        eprintln!("no golden {}; bless with ZGX_BLESS=1", path.display());
        return;
    }
    let mut reader =
        png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()))
            .read_info()
            .unwrap();
    let mut expected = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut expected).unwrap();
    // Shading by rays differs a little from GPU to GPU of a family; three
    // levels in 1% of pixels.
    let differing = actual
        .chunks(4)
        .zip(expected.chunks(4))
        .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 3))
        .count();
    let allowed = (SIZE.0 * SIZE.1) as usize / 100;
    assert!(
        differing <= allowed,
        "{differing} pixels differ past 3 from {}",
        path.display()
    );
}

#[test]
fn turning_the_camera_between_lamps_doesnt_converge_the_probes_again() {
    // Two more lamps at x = ±0.7: the frame shades its point lights
    // nearest the camera's target first, and turning from one to the
    // other reorders them. No light changed: the probes stay converged.
    let world = World::new();
    room(&world);
    for x in [-0.7, 0.7] {
        world.add_light(Light::Point {
            position: vec3(x, 1.2, 0.3),
            color: [0.6, 0.6, 0.6],
            intensity: 1.,
            range: 10.,
        });
    }
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    world.add_probes(volume().per_frame(216)).unwrap();
    settle(&mut window, &world, 500);
    if !world.probes_traced() {
        return;
    }
    let aim = |x: f32| {
        world.set_camera(Camera {
            position: vec3(0., 1., 3.4),
            target: vec3(x, 1., 0.),
            fov: 40f32.to_radians(),
            ..Camera::default()
        })
    };
    aim(-0.6);
    settle(&mut window, &world, 500);
    aim(0.6);
    let frames = settle(&mut window, &world, 500);
    assert_eq!(frames, 1, "a turn of the camera drew {frames} frames");
}

#[test]
fn a_mesh_rays_dont_see_moving_doesnt_converge_the_probes_again() {
    // A pane light passes through is left out of the rays' scene: moving
    // it changes the world's meshes, but nothing the probes see.
    let world = World::new();
    room(&world);
    let pane = world.add(
        Object::mesh(Geometry::cuboid(0.3, 0.3, 0.02), Material::rgb(0xffffff)).at([-0.5, 1., 0.5]),
    );
    world.set_transmission(pane, Some(Transmission::new(1.)));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    world.add_probes(volume().per_frame(216)).unwrap();
    settle(&mut window, &world, 500);
    if !world.probes_traced() {
        return;
    }
    world.set_transform(pane, Transform::at([-0.4, 1., 0.5]));
    let frames = settle(&mut window, &world, 500);
    assert_eq!(
        frames, 1,
        "moving a pane rays don't see drew {frames} frames"
    );
}

#[test]
fn a_world_says_what_its_probes_did() {
    let world = World::new();
    room(&world);
    assert_eq!(world.probes_status(), ProbesStatus::NoVolumes);
    world.add_probes(volume()).unwrap();
    assert_eq!(world.probes_status(), ProbesStatus::NotDrawn);
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    let status = world.probes_status();
    assert!(
        matches!(status, ProbesStatus::Traced | ProbesStatus::NoRayQueries),
        "{status:?}"
    );
    assert_eq!(world.probes_traced(), status == ProbesStatus::Traced);
    // The device says why it traces or not.
    assert_eq!(
        status == ProbesStatus::Traced,
        zgx_gpu::ray_queries_status() == Some(zgx_gpu::RayQueries::Available)
    );
}

#[cfg(feature = "lighting")]
#[test]
fn a_world_shading_through_clusters_says_its_probes_are_off() {
    let world = World::new();
    room(&world);
    world.set_clustered_lights(true);
    world.add_probes(volume()).unwrap();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    if world.probes_status() == ProbesStatus::NoRayQueries {
        return;
    }
    assert_eq!(
        world.probes_status(),
        ProbesStatus::BesideClustersOrCascades
    );
    assert!(!world.probes_traced());
}

#[test]
fn a_ray_bringing_no_finite_light_leaves_the_probes_finite() {
    // A speck glowing without bound in a corner: rays that hit it bring
    // back infinite light. Kept out of the probes' running averages, the
    // room shades as it does without it; let in, every probe reading it
    // is ruined for good (a blend never sheds what isn't finite).
    let shade = |speck: bool| {
        let world = World::new();
        room(&world);
        if speck {
            world.add(
                Object::mesh(
                    Geometry::cuboid(0.05, 0.05, 0.05),
                    Material {
                        emissive: [f32::INFINITY; 3],
                        ..Material::default()
                    },
                )
                .at([-0.9, 0.05, -0.9]),
            );
        }
        world.add_probes(volume().per_frame(216)).unwrap();
        let mut window = Window::new(&world)?;
        settle(&mut window, &world, 500);
        world.probes_traced().then(|| window.pixels())
    };
    let (Some(clean), Some(hot)) = (shade(false), shade(true)) else {
        return;
    };
    for at in [(102, 110), (80, 60), (128, 140), (40, 128)] {
        let (a, b) = (around(&clean, at, 6), around(&hot, at, 6));
        assert!(
            a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.03),
            "at {at:?}: {a:?} without the speck, {b:?} with it"
        );
    }
}
