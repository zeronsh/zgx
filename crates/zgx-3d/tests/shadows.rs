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
    fn resize(&mut self, (w, h): (u32, u32)) {
        self.gpu.resize(w, h);
        self.ui.scene.borrow_mut().resize(w as f32, h as f32);
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
    scene_with_ids().0
}
/// `scene`, with its floor's and its box's ids.
fn scene_with_ids() -> (World, ObjectId, ObjectId) {
    let world = World::new();
    let floor = world.add(Object::mesh(
        Geometry::plane(60., 60.),
        Material::rgb(0xc0c0c0).with(0., 0.9),
    ));
    let post = world
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
    (world, floor, post)
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

#[test]
fn cascades_set_out_of_bounds_are_brought_within_them() {
    let world = scene();
    // Fields set by hand past what the builders allow: a map of no
    // texels, a split past logarithmic, no distance, too many.
    let mut cascades = Cascades::new(2);
    cascades.size = 0;
    cascades.split = 5.;
    cascades.distance = f32::NAN;
    cascades.count = 9;
    world.set_cascades(Some(cascades));
    let held = world.cascades().unwrap();
    assert_eq!(held.count, 4);
    assert!(held.size >= 64 && held.size <= 4096, "{}", held.size);
    assert!((0. ..=1.).contains(&held.split), "{}", held.split);
    assert!(held.distance.is_finite() && held.distance > 0.);
    let mut huge = Cascades::new(1);
    huge.size = 100_000;
    world.set_cascades(Some(huge));
    assert!(world.cascades().unwrap().size <= 4096);
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    world.set_cascades(Some(cascades));
    let pixels = window.draw();
    let (shadow, lit) = (
        seen_at(&world, vec3(-2., 0., 0.)),
        seen_at(&world, vec3(2., 0., 0.)),
    );
    assert!(pixels[shadow] + 40 < pixels[lit], "still shadowed");
}

#[test]
fn a_camera_at_its_own_target_lets_a_world_with_cascades_rest() {
    let world = scene();
    world.set_cascades(Some(Cascades::new(4)));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    for camera in [
        Camera::looking_at([1., 2., 3.], [1., 2., 3.]),
        Camera {
            orthographic: true,
            fov: f32::NAN,
            ..Camera::looking_at([0., 6., 9.], [1., 0., 0.])
        },
        Camera {
            orthographic: true,
            fov: -3.,
            ..Camera::looking_at([0., 6., 9.], [1., 0., 0.])
        },
    ] {
        world.set_camera(camera);
        // Settled within as many frames as there are cascades. (A
        // headless window draws an animating view only when told: as a
        // frame clock would, each frame here asks for the next.)
        let mut drawn = 0;
        for _ in 0..6 {
            world.view().invalidate();
            window.draw();
            drawn += 1;
            if !world.view().animating() {
                break;
            }
        }
        assert!(
            !world.view().animating(),
            "{camera:?}: still drawing after {drawn} frames"
        );
    }
}

/// A row of posts receding from the camera, their shadows in three
/// cascades reaching 12 m (as the reference scene `shadow_cascades`).
fn posts() -> World {
    let world = World::new();
    world.add(Object::mesh(
        Geometry::plane(30., 30.),
        Material::rgb(0x9a948a).with(0., 0.8),
    ));
    for i in 0..8 {
        let z = 1.5 - i as f32 * 1.6;
        world.add(
            Object::mesh(Geometry::cuboid(0.25, 1.4, 0.25), Material::rgb(0xe0e0e0)).at([
                0.6 * (i % 2) as f32 - 0.3,
                0.7,
                z,
            ]),
        );
    }
    world.add_light(Light::Directional {
        direction: vec3(-0.7, -1., 0.2),
        color: [1., 0.97, 0.92],
        intensity: 3.,
    });
    world.set_shadows(true);
    world.set_cascades(Some(Cascades::new(3).distance(12.).size(512)));
    world.set_camera(Camera::looking_at([1.2, 1.6, 3.5], [-0.3, 0.4, -4.]));
    world
}

#[test]
fn a_view_resized_and_back_draws_its_shadows_as_a_fresh_one() {
    let fresh = posts();
    let Some(mut window) = Window::new(&fresh) else {
        return;
    };
    let expected = window.draw();
    drop(window);
    let world = posts();
    let mut window = Window::new(&world).unwrap();
    window.draw();
    // Other shapes, one frame each, then back: the cascades fit the view's
    // aspect, so each change of it fits them again.
    for size in [(96, 72), (160, 120), (SIDE, SIDE)] {
        window.resize(size);
        window.draw();
    }
    let actual = window.draw();
    let (count, largest) = actual
        .iter()
        .zip(&expected)
        .map(|(a, b)| a.abs_diff(*b))
        .fold((0, 0), |(n, most), d| (n + usize::from(d > 1), most.max(d)));
    assert_eq!(
        count, 0,
        "{count} channels differ past 1, by up to {largest}"
    );
}

/// Channels of `actual` more than 1 off `expected`, and the most.
fn differ(actual: &[u8], expected: &[u8]) -> (usize, u8) {
    actual
        .iter()
        .zip(expected)
        .map(|(a, b)| a.abs_diff(*b))
        .fold((0, 0), |(n, most), d| (n + usize::from(d > 1), most.max(d)))
}

/// `posts` and boxes among them, two at `x` and `z`.
fn posts_and_boxes(x: f32, z: f32) -> (World, ObjectId, ObjectId) {
    let world = posts();
    // One shape: one run, its instances side by side.
    let shape = Geometry::cuboid(0.5, 0.5, 0.5);
    let block =
        |at: [f32; 3]| world.add(Object::mesh(shape.clone(), Material::rgb(0xd0c0b0)).at(at));
    // One still ahead of the first: it moves between two still ones of
    // its run, which its still ones are drawn around exactly.
    block([-0.9, 0.25, -2.4]);
    let (first, second) = (block([x, 0.25, -1.2]), block([0.9, 0.25, z]));
    (world, first, second)
}

/// Frames after its last move a box counts as still (120) and a few.
const STILL: usize = 125;

/// `world` shadowed by one cascade: what it holds is what's seen (with
/// more, the nearest would hide what the farther ones hold of the boxes).
fn in_one_cascade<T>((world, a, b): (World, T, T)) -> (World, T, T) {
    world.set_cascades(Some(Cascades::new(1).distance(12.).size(512)));
    (world, a, b)
}

#[test]
fn boxes_moving_among_still_ones_shadow_as_in_a_fresh_world() {
    let place = |frame: usize| (frame as f32 * 0.37).sin() * 1.4;
    let check = |what: &str, actual: Vec<u8>, (x, z): (f32, f32)| {
        let (world, ..) = in_one_cascade(posts_and_boxes(x, z));
        let expected = Window::new(&world).unwrap().draw();
        let (count, largest) = differ(&actual, &expected);
        assert_eq!(
            count, 0,
            "{what}: {count} channels differ past 1, by up to {largest}"
        );
    };
    let (world, first, second) = in_one_cascade(posts_and_boxes(place(0), -3.));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    // Where each box is: the first's frame along its path, the second's z.
    let (mut at, mut along, mut z) = (0, 0, -3.);
    let mut frame = |window: &mut Window, moves: (bool, bool), what: Option<&str>| {
        if moves.0 {
            at += 1;
            world.set_transform(first, Transform::at([place(at), 0.25, -1.2]));
        }
        if moves.1 {
            // To and fro, in the cascade's view throughout.
            along += 1;
            z = -3. + (along as f32 * 0.3).sin() * 0.8;
            world.set_transform(second, Transform::at([0.9, 0.25, z]));
        }
        if !moves.0 && !moves.1 {
            world.view().invalidate();
        }
        let actual = window.draw();
        if let Some(what) = what {
            check(what, actual, (place(at), z));
        }
    };
    // The first moving every frame: each cascade holding it drawn from its
    // layer of still casters and the box, past the frames the others take
    // to count as still and on.
    for f in 1..=45 {
        let what = format!("first moving, frame {f}");
        frame(
            &mut window,
            (true, false),
            [1, 2, 20, 45].contains(&f).then_some(&what),
        );
    }
    // The second, in those layers, moving too: they're drawn again.
    for f in 1..=3 {
        frame(
            &mut window,
            (true, true),
            Some(&format!("both moving, frame {f}")),
        );
    }
    // The first stops, and the second moves on past the frames it takes the
    // first to count as still: it's drawn with the movers (it isn't in the
    // layers), and when it moves again they aren't drawn again.
    for f in 1..=STILL {
        frame(
            &mut window,
            (false, true),
            (f == STILL).then_some("first stopped"),
        );
    }
    for f in 1..=3 {
        frame(
            &mut window,
            (true, true),
            Some(&format!("first again, frame {f}")),
        );
    }
    // The camera turning while the second moves: the far cascades it
    // touches drawn from their layers at the fit they hold, each taking the
    // camera's in its turn. Back where it was, they catch up.
    let camera = world.camera();
    for f in 1..=6 {
        let turned = Camera::looking_at([1.2 + f as f32 * 0.4, 1.6, 3.5], [-0.3, 0.4, -4.]);
        world.set_camera(turned);
        frame(&mut window, (false, true), None);
    }
    world.set_camera(camera);
    for f in 1..=4 {
        frame(
            &mut window,
            (false, true),
            (f == 4).then_some("camera back"),
        );
    }
    // Both stopped, drawn on until they count as still (the layers go).
    for _ in 0..STILL {
        frame(&mut window, (false, false), None);
    }
    frame(&mut window, (false, false), Some("all still"));
    // Moving again, the window resting between: drawn whole, then from the
    // layers made again.
    frame(&mut window, (false, true), Some("second again"));
    window.gpu.trim();
    for f in 1..=3 {
        frame(
            &mut window,
            (false, true),
            Some(&format!("rested, frame {f}")),
        );
    }
}

#[test]
fn a_box_moving_as_the_camera_turns_shadows_as_one_placed_there() {
    // The far cascades the box is in keep the fit they hold as the camera
    // turns (each takes the camera's in its turn): one moving with it is
    // drawn there, as one already there would be.
    let turned = Camera::looking_at([2.4, 1.9, 3.], [-0.3, 0.4, -4.]);
    let (end, z) = (0.9, -3.);
    let (fresh, ..) = posts_and_boxes(end, z);
    let Some(mut window) = Window::new(&fresh) else {
        return;
    };
    window.draw();
    fresh.set_camera(turned);
    let expected = window.draw();
    drop(window);
    let (world, first, _) = posts_and_boxes(-0.9, z);
    let mut window = Window::new(&world).unwrap();
    window.draw();
    // Moving long enough that each cascade holding it draws from its layer.
    for f in 1..=4 {
        let x = -0.9 + f as f32 * 0.3;
        world.set_transform(first, Transform::at([x, 0.25, -1.2]));
        window.draw();
    }
    world.set_transform(first, Transform::at([end, 0.25, -1.2]));
    world.set_camera(turned);
    let (count, largest) = differ(&window.draw(), &expected);
    assert_eq!(
        count, 0,
        "{count} channels differ past 1, by up to {largest}"
    );
}

#[test]
fn turning_cascades_or_shadows_off_while_a_cascade_is_owed_lets_the_world_rest() {
    type Switch = (&'static str, fn(&World));
    let off: [Switch; 2] = [
        ("cascades off", |w| w.set_cascades(None)),
        ("shadows off", |w| w.set_shadows(false)),
    ];
    for (name, turn_off) in off {
        let world = scene();
        world.set_cascades(Some(Cascades::new(4).distance(40.)));
        let Some(mut window) = Window::new(&world) else {
            return;
        };
        window.draw();
        // The camera turns: the nearest cascade follows at once, the
        // others are owed a frame each.
        world.set_camera(Camera::looking_at([4., 5., 8.], [-2., 0., 1.]));
        window.draw();
        assert!(
            world.view().animating(),
            "{name}: the control, a cascade owed"
        );
        turn_off(&world);
        for _ in 0..4 {
            world.view().invalidate();
            window.draw();
        }
        assert!(
            !world.view().animating(),
            "{name}: still drawing for cascades it no longer has"
        );
    }
}

/// The floor in the box's shadow and out of it, as `scene` draws it, with
/// cascades or with the single map.
fn shadow_and_lit(world: &World, window: &mut Window) -> (u8, u8) {
    let pixels = window.draw();
    let (shadow, lit) = (
        seen_at(world, vec3(-2., 0., 0.)),
        seen_at(world, vec3(2., 0., 0.)),
    );
    (pixels[shadow], pixels[lit])
}

#[test]
fn a_mesh_set_not_to_cast_leaves_no_shadow_and_casts_again_when_set_back() {
    for cascaded in [false, true] {
        let (world, _, post) = scene_with_ids();
        if cascaded {
            world.set_cascades(Some(Cascades::new(2).distance(40.)));
        }
        let Some(mut window) = Window::new(&world) else {
            return;
        };
        let (shadow, lit) = shadow_and_lit(&world, &mut window);
        assert!(
            shadow + 40 < lit,
            "cascaded {cascaded}: the control, {shadow} {lit}"
        );
        world.set_casts_shadows(post, false);
        assert!(!world.casts_shadows(post));
        let (shadow, lit) = shadow_and_lit(&world, &mut window);
        assert!(
            shadow.abs_diff(lit) < 4,
            "cascaded {cascaded}: it still casts, {shadow} {lit}"
        );
        world.set_casts_shadows(post, true);
        let (shadow, lit) = shadow_and_lit(&world, &mut window);
        assert!(
            shadow + 40 < lit,
            "cascaded {cascaded}: set back, no shadow, {shadow} {lit}"
        );
    }
}

#[test]
fn a_mesh_set_not_to_receive_is_lit_where_a_shadow_falls_on_it() {
    for cascaded in [false, true] {
        let (world, floor, _) = scene_with_ids();
        if cascaded {
            world.set_cascades(Some(Cascades::new(2).distance(40.)));
        }
        let Some(mut window) = Window::new(&world) else {
            return;
        };
        world.set_receives_shadows(floor, false);
        assert!(!world.receives_shadows(floor));
        let (shadow, lit) = shadow_and_lit(&world, &mut window);
        assert!(
            shadow.abs_diff(lit) < 4,
            "cascaded {cascaded}: still shadowed, {shadow} {lit}"
        );
        world.set_receives_shadows(floor, true);
        let (shadow, lit) = shadow_and_lit(&world, &mut window);
        assert!(
            shadow + 40 < lit,
            "cascaded {cascaded}: set back, no shadow, {shadow} {lit}"
        );
    }
}
