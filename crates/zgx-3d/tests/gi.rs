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
        // `ZGX_GI_NO_RAY_QUERIES=1`: as on a device without them, to see
        // these tests say they're skipped.
        if std::env::var_os("ZGX_GI_NO_RAY_QUERIES").is_none_or(|v| v != "1") {
            // SAFETY: a test of ray-traced probes takes on wgpu's
            // experimental contract (`request_ray_queries`).
            unsafe { zgx_gpu::request_ray_queries() };
        }
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

/// Whether `world`'s last frame lit its probes, by rays or by faces. Where
/// it lit none, the test fails where a GPU is required (`ZGX_REQUIRE_GPU`)
/// or a golden is being blessed, naming the reason
/// (`zgx_gpu::ray_queries_status`): a probe test that checked nothing never
/// passes as one that did. `ZGX_GI_ALLOW_SKIP=1` lets it pass as skipped (a
/// `GI-SKIPPED` line, shown with `--nocapture`), for a run on a device known
/// to have none. `ZGX_REQUIRE_RAY_QUERIES=1` requires rays: faces fail it.
fn traced(world: &World, test: &str) -> bool {
    probes_lit(world, test, false)
}
/// As `traced`, for a world whose probes are drawn by faces on purpose
/// (`World::set_probes_rasterised`): faces are what it asks for, required
/// rays or not, and rays fail it.
#[cfg_attr(not(feature = "lighting"), allow(dead_code))]
fn faced(world: &World, test: &str) -> bool {
    probes_lit(world, test, true)
}
fn probes_lit(world: &World, test: &str, faces: bool) -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|v| v == "1");
    let wanted = Wanted {
        gpu: std::env::var_os("ZGX_REQUIRE_GPU").is_some(),
        skip_allowed: set("ZGX_GI_ALLOW_SKIP"),
        rays: set("ZGX_REQUIRE_RAY_QUERIES"),
        bless: set("ZGX_BLESS"),
        faces,
    };
    let status = world.probes_status();
    match lit(status, zgx_gpu::ray_queries_status(), wanted) {
        Ok(true) => true,
        Ok(false) => {
            eprintln!(
                "GI-SKIPPED {test}: probes not traced ({status:?}; ray queries {:?})",
                zgx_gpu::ray_queries_status()
            );
            false
        }
        Err(why) => panic!("{test}: {why}"),
    }
}

/// What a run asks of the probes (`traced`'s variables).
#[derive(Clone, Copy, Default)]
struct Wanted {
    gpu: bool,
    skip_allowed: bool,
    rays: bool,
    bless: bool,
    /// The test forces faces (`World::set_probes_rasterised`).
    faces: bool,
}

/// `traced`'s decision, from the probes' status, why the device has no ray
/// queries if it hasn't, and what the run asks: lit (`Ok(true)`), skipped
/// (`Ok(false)`), or a failure saying why.
fn lit(
    status: ProbesStatus,
    rays: Option<zgx_gpu::RayQueries>,
    wanted: Wanted,
) -> Result<bool, String> {
    // An adapter whose ray queries are declined (WARP) draws faces: always.
    if rays == Some(zgx_gpu::RayQueries::DeclinedOnAdapter) && status != ProbesStatus::Rasterised {
        return Err(format!(
            "rays declined, but the probes are {status:?}, not faces"
        ));
    }
    // Forced faces: rays drawn in their place are wrong, and rays required
    // ask nothing of it.
    if wanted.faces && status == ProbesStatus::Traced {
        return Err("faces forced, but the probes were traced".to_owned());
    }
    if wanted.rays && !wanted.faces && status != ProbesStatus::Traced {
        return Err(format!(
            "ZGX_REQUIRE_RAY_QUERIES=1, but the probes are {status:?}, not traced: ray queries {rays:?}"
        ));
    }
    if matches!(status, ProbesStatus::Traced | ProbesStatus::Rasterised) {
        return Ok(true);
    }
    if (wanted.gpu && !wanted.skip_allowed) || wanted.bless {
        return Err(format!(
            "traced no probes ({status:?}; ray queries {rays:?}): set ZGX_GI_ALLOW_SKIP=1 to skip"
        ));
    }
    Ok(false)
}

#[test]
fn rays_required_fail_where_faces_were_drawn_and_say_why() {
    use zgx_gpu::RayQueries;
    let rays = Wanted {
        gpu: true,
        rays: true,
        ..Default::default()
    };
    // Faces under FXC, and on WARP: failures naming why.
    for why in [RayQueries::CompilerIsFxc, RayQueries::DeclinedOnAdapter] {
        let failed = lit(ProbesStatus::Rasterised, Some(why), rays).unwrap_err();
        assert!(failed.contains(&format!("{why:?}")), "{failed}");
        assert!(failed.contains("Rasterised"), "{failed}");
    }
    // Rays: lit.
    assert_eq!(lit(ProbesStatus::Traced, None, rays), Ok(true));
    // Rays not required: faces light the probes as well.
    let gpu = Wanted {
        gpu: true,
        ..Default::default()
    };
    let fxc = Some(RayQueries::CompilerIsFxc);
    assert_eq!(lit(ProbesStatus::Rasterised, fxc, gpu), Ok(true));
    // Neither: a failure where a GPU is required or a golden blessed, else
    // a skip, and a skip where allowed.
    let none = ProbesStatus::Unsupported;
    assert!(lit(none, fxc, gpu).is_err());
    let bless = Wanted {
        bless: true,
        ..Default::default()
    };
    assert!(lit(none, fxc, bless).is_err());
    assert_eq!(lit(none, fxc, Wanted::default()), Ok(false));
    let allowed = Wanted {
        skip_allowed: true,
        ..gpu
    };
    assert_eq!(lit(none, fxc, allowed), Ok(false));
    // WARP declines rays: its probes are faces, or the test fails.
    let warp = Some(RayQueries::DeclinedOnAdapter);
    assert!(lit(ProbesStatus::Traced, warp, gpu).is_err());
    // Faces forced, rays required and available: faces pass; rays fail.
    let forced = Wanted {
        faces: true,
        ..rays
    };
    let available = Some(RayQueries::Available);
    assert_eq!(lit(ProbesStatus::Rasterised, available, forced), Ok(true));
    assert!(lit(ProbesStatus::Traced, available, forced).is_err());
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
    if !traced(
        &world,
        "probes_carry_the_walls_colours_into_the_room_and_light_what_the_lamp_misses",
    ) {
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
    if !traced(
        &world,
        "a_still_room_stops_drawing_once_its_probes_converge_and_starts_again_as_the_lamp_moves",
    ) {
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
    if !traced(
        &fresh_world,
        "a_room_reached_through_changes_draws_as_a_fresh_one",
    ) {
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
fn software_adapters_decline_ray_queries_and_draw_faces() {
    // Asked for, ray queries are declined on lavapipe (Vulkan's llvmpipe:
    // its first acceleration structure build crashes) and WARP (it loses
    // a structure's first triangle now and then): the device says so, and
    // probes draw faces. Elsewhere this asks nothing.
    let world = World::new();
    room(&world);
    world.add_probes(volume()).unwrap();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    let info = window.gpu.adapter_info();
    let family = family(&window.gpu);
    let declined = (family == "llvmpipe" && info.backend == zgx_gpu::wgpu::Backend::Vulkan)
        || family == "warp";
    let asked = std::env::var_os("ZGX_GI_NO_RAY_QUERIES").is_none_or(|v| v != "1");
    if !declined || !asked {
        return;
    }
    assert_eq!(
        zgx_gpu::ray_queries_status(),
        Some(zgx_gpu::RayQueries::DeclinedOnAdapter),
        "{info:?}"
    );
    assert_eq!(world.probes_status(), ProbesStatus::Rasterised, "{info:?}");
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
    if !traced(&world, "the_probe_lit_room_matches_its_golden") {
        return;
    }
    let actual = window.pixels();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/reference")
        .join(family(&window.gpu))
        // Faces drawn in place of rays shade a little differently.
        .join(if world.probes_status() == ProbesStatus::Rasterised {
            "gi_room_raster.png"
        } else {
            "gi_room.png"
        });
    golden(&path, &actual);
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
    if !traced(
        &world,
        "turning_the_camera_between_lamps_doesnt_converge_the_probes_again",
    ) {
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
    if !traced(
        &world,
        "a_mesh_rays_dont_see_moving_doesnt_converge_the_probes_again",
    ) {
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
        matches!(status, ProbesStatus::Traced | ProbesStatus::Rasterised),
        "{status:?}"
    );
    traced(&world, "a_world_says_what_its_probes_did");
    assert_eq!(world.probes_traced(), status == ProbesStatus::Traced);
    // The device says why it traces or not.
    assert_eq!(
        status == ProbesStatus::Traced,
        zgx_gpu::ray_queries_status() == Some(zgx_gpu::RayQueries::Available)
    );
}

#[cfg(feature = "lighting")]
#[test]
fn probes_light_a_world_shading_through_clusters_and_cascades() {
    // The room as before, its lamp shaded through clusters and a sun (its
    // light nothing, its shadow in cascades): the block's face the lamp
    // misses is lit by the probes as without them.
    let shade = |lighting: bool| {
        let world = World::new();
        room(&world);
        if lighting {
            world.set_clustered_lights(true);
            world.set_shadows(true);
            world.set_cascades(Some(Cascades::new(2)));
            world.add_light(Light::Directional {
                direction: vec3(0.3, -1., 0.2),
                color: [1.; 3],
                intensity: 0.,
            });
        }
        world.add_probes(volume().per_frame(216)).unwrap();
        let mut window = Window::new(&world)?;
        settle(&mut window, &world, 500);
        traced(
            &world,
            "probes_light_a_world_shading_through_clusters_and_cascades",
        )
        .then(|| window.pixels())
    };
    let Some(plain) = shade(false) else {
        return;
    };
    let lit = shade(true).expect("probes traced beside clusters and cascades");
    let face = (102, 110);
    let (a, b) = (around(&plain, face, 6), around(&lit, face, 6));
    assert!(b[1] > 0.15, "the block's face: {b:?}");
    assert!(
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.04),
        "{a:?} without clusters, {b:?} with"
    );
}

#[test]
fn a_ray_bringing_no_finite_light_leaves_the_probes_finite() {
    // Every seventh ray made to bring back infinite light or no number at
    // all (what a glow without bound or a broken mesh would), divided out
    // of a zero the GPU only meets as it runs, so no compiler folds it
    // away. (A world can't be given such a glow: it keeps emission
    // finite.) Kept out of the probes' running averages, the room shades
    // as it does without; let in, the probes are ruined (a blend never
    // sheds what isn't finite), which shows the rays really brought it.
    // By rays and by faces: both write rays the same way.
    let test = "a_ray_bringing_no_finite_light_leaves_the_probes_finite";
    let shade = |faces: bool, poison: Option<bool>| {
        let world = World::new();
        room(&world);
        world.set_probes_rasterised(faces);
        if let Some(let_in) = poison {
            world.poison_probe_rays(true, let_in);
        }
        world.add_probes(volume().per_frame(216)).unwrap();
        let mut window = Window::new(&world)?;
        settle(&mut window, &world, 500);
        // (Faces forced: `faced`.)
        probes_lit(&world, test, faces).then(|| window.pixels())
    };
    for faces in [false, true] {
        let (Some(clean), Some(kept_out), Some(let_in)) = (
            shade(faces, None),
            shade(faces, Some(false)),
            shade(faces, Some(true)),
        ) else {
            return;
        };
        let path = if faces { "faces" } else { "rays" };
        save(&format!("unfinite-{path}-clean"), &clean);
        save(&format!("unfinite-{path}-kept-out"), &kept_out);
        save(&format!("unfinite-{path}-let-in"), &let_in);
        let points = [(102, 110), (80, 60), (128, 140), (40, 128)];
        for at in points {
            let (a, b) = (around(&clean, at, 6), around(&kept_out, at, 6));
            assert!(
                a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.03),
                "{path} at {at:?}: {a:?} clean, {b:?} with the rays kept out"
            );
        }
        let ruined = points
            .iter()
            .filter(|&&at| {
                let (a, b) = (around(&clean, at, 6), around(&let_in, at, 6));
                a.iter().zip(b).any(|(a, b)| (a - b).abs() > 0.1)
            })
            .count();
        assert!(
            ruined >= 3,
            "{path}: let in, the rays ruined {ruined} of {} points: they didn't bring what isn't finite",
            points.len()
        );
    }
}

#[test]
fn a_world_whose_probes_go_with_its_renderer_comes_to_rest() {
    // Probes converging hold the world drawing. Removed in the same frame
    // as the samples change (the renderer's device state is made again,
    // with no probes), the hold must go too, or the world draws for good.
    let world = World::new();
    room(&world);
    let probes = world.add_probes(volume()).unwrap();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    if !traced(
        &world,
        "a_world_whose_probes_go_with_its_renderer_comes_to_rest",
    ) {
        return;
    }
    assert!(world.view().animating(), "converging probes hold the world");
    world.remove_probes(probes);
    world.set_samples(1);
    window.draw();
    let frames = settle(&mut window, &world, 50);
    assert!(
        !world.view().animating(),
        "still drawing after {frames} frames: the probes' hold outlived them"
    );
}

/// `actual` against its golden at `path` (`ZGX_BLESS=1` writes it): three
/// levels in 1% of pixels, as shading by rays or faces differs a little
/// from GPU to GPU of a family.
/// A PNG's RGBA, where there's one at `path`.
fn read_png(path: &std::path::Path) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .unwrap();
    let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut rgba).unwrap();
    Some(rgba)
}

fn golden(path: &std::path::Path, actual: &[u8]) {
    if std::env::var_os("ZGX_BLESS").is_some_and(|v| v == "1") {
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(file, SIZE.0, SIZE.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::High);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(actual)
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
    let expected = read_png(path).unwrap();
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
fn faces_drawn_in_place_of_rays_light_the_room_alike() {
    // Where the device has no ray queries, each probe draws the scene into
    // cube faces about it instead. Forced here beside rays on one device,
    // the room's bounced light comes out alike (rays shadow the lamp at
    // what they hit; faces, as the meshes drawn directly, don't). Where the
    // device has none (D3D12 under FXC, say), beside the rays this adapter
    // traced where it had them: its committed golden of the same room.
    let shade = |faces: bool| {
        let world = World::new();
        room(&world);
        world.set_probes_rasterised(faces);
        world.add_probes(volume().per_frame(216)).unwrap();
        let mut window = Window::new(&world)?;
        let frames = settle(&mut window, &world, 500);
        Some((
            world.probes_status(),
            frames,
            window.pixels(),
            family(&window.gpu),
        ))
    };
    let Some((status, frames, faces, family)) = shade(true) else {
        return;
    };
    assert_eq!(status, ProbesStatus::Rasterised);
    assert!(frames <= 20, "{frames} frames to converge");
    save("room-faces", &faces);
    let rays = match shade(false) {
        Some((ProbesStatus::Traced, _, rays, _)) => rays,
        _ => {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/reference")
                .join(&family)
                .join("gi_room.png");
            let Some(rays) = read_png(&path) else {
                let set = |name: &str| std::env::var_os(name).is_some_and(|v| v == "1");
                let required = (std::env::var_os("ZGX_REQUIRE_GPU").is_some()
                    && !set("ZGX_GI_ALLOW_SKIP"))
                    || std::env::var_os("ZGX_REQUIRE_GOLDENS").is_some();
                assert!(
                    !required,
                    "no rays here and no traced golden {} to set the faces beside",
                    path.display()
                );
                eprintln!(
                    "GI-SKIPPED faces_drawn_in_place_of_rays_light_the_room_alike: no rays and no {}",
                    path.display()
                );
                return;
            };
            rays
        }
    };
    for at in [(102, 110), (80, 60), (40, 128), (12, 80), (150, 80)] {
        let (a, b) = (around(&rays, at, 6), around(&faces, at, 6));
        assert!(
            a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.06),
            "at {at:?}: {a:?} by rays, {b:?} by faces"
        );
    }
}

#[test]
fn a_tight_face_budget_converges_over_more_frames_to_the_same_light() {
    // Drawing faces, a frame updates as many probes as its budget of
    // triangles holds: held to a few, the room takes many more frames to
    // converge, each drawing no more, and comes to the same light.
    let test = "a_tight_face_budget_converges_over_more_frames_to_the_same_light";
    let shade = |budget: Option<u64>| {
        let world = World::new();
        room(&world);
        world.set_probes_rasterised(true);
        world.set_probe_face_budget(budget);
        world.add_probes(volume().per_frame(216)).unwrap();
        let mut window = Window::new(&world)?;
        let frames = settle(&mut window, &world, 5000);
        traced(&world, test).then(|| (frames, window.pixels()))
    };
    let (Some((free, a)), Some((tight, b))) = (shade(None), shade(Some(2_000))) else {
        return;
    };
    eprintln!("converged in {free} frames free, {tight} held to 2,000 triangles");
    assert!(free <= 20, "{free} frames");
    assert!(tight > free * 4, "{tight} frames held, {free} free");
    for at in [(102, 110), (80, 60), (40, 128), (12, 80), (150, 80)] {
        let (a, b) = (around(&a, at, 6), around(&b, at, 6));
        assert!(
            a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.03),
            "at {at:?}: {a:?} free, {b:?} held"
        );
    }
}

#[cfg(feature = "lighting")]
#[test]
fn a_roof_set_not_to_cast_leaves_the_probes_sunlit_beneath() {
    // A roof over a sunlit floor, set not to cast the sun's shadow: the
    // floor beneath it is sunlit, and so the roof's underside (lit only by
    // light the floor bounces, through the probes) is bright, by rays as
    // by faces. Rays tracing the sun's shadow through it would see the
    // floor beneath in shade.
    let test = "a_roof_set_not_to_cast_leaves_the_probes_sunlit_beneath";
    let shade = |faces: bool, casts: bool| {
        let world = World::new();
        let white = Material {
            color: [0.8, 0.8, 0.8, 1.],
            roughness: 1.,
            ..Material::default()
        };
        world.add(Object::mesh(Geometry::cuboid(12., 0.2, 12.), white).at([0., -0.1, 0.]));
        let roof =
            world.add(Object::mesh(Geometry::cuboid(4.4, 0.2, 4.4), white).at([0., 3.1, 0.]));
        world.set_casts_shadows(roof, casts);
        world.add_light(Light::Directional {
            direction: vec3(0.2, -1., 0.1),
            color: [1.; 3],
            intensity: 3.,
        });
        world.set_shadows(true);
        world.set_camera(Camera {
            position: vec3(0., 0.8, 3.5),
            target: vec3(0., 3., 0.),
            fov: 50f32.to_radians(),
            ..Camera::default()
        });
        world.set_background(Some(0x000000));
        world.set_probes_rasterised(faces);
        world
            .add_probes(
                ProbeVolume::new([-1.8, 0.4, -1.8], [1.8, 2.8, 1.8], [4, 3, 4]).per_frame(48),
            )
            .unwrap();
        let mut window = Window::new(&world)?;
        settle(&mut window, &world, 500);
        let lit = if faces {
            faced(&world, test)
        } else {
            traced(&world, test) && world.probes_status() == ProbesStatus::Traced
        };
        lit.then(|| window.pixels())
    };
    // The roof's underside, seen from below.
    let under = (SIZE.0 / 2, SIZE.1 / 4);
    let Some(faces) = shade(true, false) else {
        return;
    };
    save("roof-faces", &faces);
    let faces = around(&faces, under, 8);
    let Some(rays) = shade(false, false) else {
        eprintln!("GI-SKIPPED {test}: no rays to set beside the faces");
        return;
    };
    save("roof-rays", &rays);
    let rays = around(&rays, under, 8);
    let shaded = around(&shade(false, true).unwrap(), under, 8);
    eprintln!("underside: by faces {faces:?}, by rays {rays:?}, casting {shaded:?}");
    assert!(
        rays.iter().zip(faces).all(|(r, f)| (r - f).abs() < 0.06),
        "by rays {rays:?}, by faces {faces:?}"
    );
    assert!(
        rays[1] > shaded[1] + 0.05,
        "not casting {rays:?}, casting {shaded:?}: rays saw the floor beneath in its shade"
    );
}

#[test]
fn the_room_lit_by_faces_matches_its_golden() {
    let world = World::new();
    room(&world);
    world.set_probes_rasterised(true);
    world.add_probes(volume()).unwrap();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    settle(&mut window, &world, 2000);
    assert_eq!(world.probes_status(), ProbesStatus::Rasterised);
    let actual = window.pixels();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/reference")
        .join(family(&window.gpu))
        .join("gi_room_raster.png");
    golden(&path, &actual);
}

#[test]
#[ignore = "a measurement: cargo test --release -p zgx-3d --features gi --test gi -- --ignored --nocapture moving"]
fn moving_a_cube_costs_a_frame() {
    // The reviewer's scene: a 67,600-triangle sphere, a floor and a 0.3 m
    // cube, probes 4 by 3 by 4 at 64 a frame. A frame's wall time drawn and
    // waited for (a readback), the cube still and moving, by rays and by
    // faces: medians of 60 frames after 30.
    for faces in [false, true] {
        let world = World::new();
        let sphere = Geometry::sphere(1., 260);
        let triangles = sphere.indices().len() / 3;
        world.add(Object::mesh(sphere, Material::default()).at([0., 1., 0.]));
        world.add(Object::mesh(Geometry::plane(10., 10.), Material::default()));
        let cube = world.add(
            Object::mesh(Geometry::cuboid(0.3, 0.3, 0.3), Material::default()).at([2., 0.5, 0.]),
        );
        world.add_light(Light::Directional {
            direction: vec3(-0.4, -1., -0.3),
            color: [1.; 3],
            intensity: 3.,
        });
        world.set_probes_rasterised(faces);
        world
            .add_probes(ProbeVolume::new([-3., 0.2, -3.], [3., 2.5, 3.], [4, 3, 4]).per_frame(64))
            .unwrap();
        let Some(mut window) = Window::new(&world) else {
            return;
        };
        // A frame's wall time drawn and waited for (a readback), and its
        // CPU's alone (recorded and submitted): medians.
        let mut time = |moving: bool| {
            let (mut wall, mut cpu) = (Vec::new(), Vec::new());
            for frame in 0..90 {
                if moving {
                    world.set_transform(
                        cube,
                        Transform::at([2. + (frame as f32 * 0.1).sin(), 0.5, 0.]),
                    );
                }
                world.view().invalidate();
                let start = std::time::Instant::now();
                window.draw();
                let drawn = start.elapsed();
                let _ = window.pixels();
                if frame >= 30 {
                    wall.push(start.elapsed().as_secs_f64() * 1e3);
                    cpu.push(drawn.as_secs_f64() * 1e3);
                }
            }
            wall.sort_by(f64::total_cmp);
            cpu.sort_by(f64::total_cmp);
            (wall[wall.len() / 2], cpu[cpu.len() / 2])
        };
        let still = time(false);
        let moving = time(true);
        eprintln!(
            "{} ({triangles} triangles + floor + cube; {:?}): (wall, cpu) still {still:.2?} ms, moving {moving:.2?} ms",
            if faces { "faces" } else { "rays" },
            world.probes_status()
        );
    }
}

#[cfg(feature = "lighting")]
#[test]
fn a_closed_box_under_the_sun_stays_dark_inside_on_the_face_path() {
    // A box closed on every side under a strong sun, its shadow in
    // cascades, its probes drawn as faces: no light gets in, so its inside
    // (lit by its probes alone) stays dark. Faces seeing the sun unshadowed
    // would light its floor and bring that in.
    let world = World::new();
    let white = Material {
        color: [0.8, 0.8, 0.8, 1.],
        roughness: 1.,
        ..Material::default()
    };
    let slab = |size: [f32; 3], at: [f32; 3]| {
        world.add(Object::mesh(Geometry::cuboid(size[0], size[1], size[2]), white).at(at));
    };
    slab([4.4, 0.2, 4.4], [0., -0.1, 0.]); // floor
    slab([4.4, 0.2, 4.4], [0., 3.1, 0.]); // roof
    slab([0.2, 3.4, 4.4], [-2.1, 1.5, 0.]);
    slab([0.2, 3.4, 4.4], [2.1, 1.5, 0.]);
    slab([4.4, 3.4, 0.2], [0., 1.5, -2.1]);
    slab([4.4, 3.4, 0.2], [0., 1.5, 2.1]);
    world.add_light(Light::Directional {
        direction: vec3(0.2, -1., 0.1),
        color: [1.; 3],
        intensity: 6.,
    });
    world.set_shadows(true);
    world.set_cascades(Some(Cascades::new(2).distance(20.)));
    world.set_camera(Camera {
        position: vec3(0., 1.5, 1.6),
        target: vec3(0., 1.2, -2.),
        fov: 60f32.to_radians(),
        near: 0.05,
        ..Camera::default()
    });
    world.set_background(Some(0x000000));
    world.set_probes_rasterised(true);
    world
        .add_probes(ProbeVolume::new([-1.8, 0.2, -1.8], [1.8, 2.8, 1.8], [4, 3, 4]).per_frame(48))
        .unwrap();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    settle(&mut window, &world, 500);
    if !faced(
        &world,
        "a_closed_box_under_the_sun_stays_dark_inside_on_the_face_path",
    ) {
        return;
    }
    let pixels = window.pixels();
    save("box-faces", &pixels);
    let wall = around(&pixels, (SIZE.0 / 2, SIZE.1 / 2), 8);
    assert!(
        wall.iter().all(|&c| c < 0.05),
        "the box's back wall inside: {wall:?}"
    );
}
