//! A world rests once what kept it drawing is turned off: for every reason
//! a painter holds in its frame (`Reason` in world.rs), the feature is
//! turned on until the world draws by itself, turned off, and the world
//! must stop within a few frames (or, where work finishes off the frame, a
//! few seconds), however far along it was. Idle windows doing no work is a
//! merge rule; this is its check, one row a reason. A lane adding a reason
//! adds its row here; a row failing in code another lane owns stays in as
//! `#[ignore = "<finding>: <owner>"]` until that lane fixes it.
// (With none of the features, every row is out: what they share goes unused.)
#![cfg_attr(not(feature = "gltf"), allow(dead_code, unused_imports))]
use std::time::{Duration, Instant};
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

#[cfg(feature = "gltf")]
#[path = "../examples/common/rig.rs"]
mod rig;

struct Window {
    ui: Ui,
    gpu: GpuRenderer,
}
impl Window {
    fn new(world: &World) -> Option<Self> {
        let gpu = match GpuRenderer::new(64, 64) {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
            }
            Err(_) => return None,
        };
        let mut ui = Ui::new(64., 64.);
        ui.mount(
            div()
                .size(64., 64.)
                .bg(Color(0, 0, 0, 255))
                .child(gpu_view("World", world.view()).size(64., 64.)),
        );
        Some(Self { ui, gpu })
    }
    /// A frame, as a window's frame clock draws one while the view
    /// animates (a headless window has none: it's asked for here).
    fn draw(&mut self, world: &World) {
        world.view().invalidate();
        self.ui.prepare_frame();
        let report = self.ui.scene.borrow_mut().flush();
        self.gpu
            .render(&self.ui.scene.borrow(), &report.damage)
            .unwrap();
    }
    /// Frames until `done`, `frames` at least and `cap` at most in time:
    /// whether it came.
    fn until(
        &mut self,
        world: &World,
        frames: u32,
        cap: Duration,
        done: impl Fn() -> bool,
    ) -> bool {
        let start = Instant::now();
        let mut drawn = 0;
        loop {
            self.draw(world);
            drawn += 1;
            if drawn >= frames && done() {
                return true;
            }
            if drawn >= frames && start.elapsed() > cap {
                return false;
            }
            if drawn >= frames {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

/// A floor, a box on it, a sun and a camera: what every row starts from.
fn world() -> (World, ObjectId) {
    let world = World::new();
    world.add(Object::mesh(
        Geometry::plane(20., 20.),
        Material::rgb(0x9a948a).with(0., 0.8),
    ));
    let cube = world
        .add(Object::mesh(Geometry::cuboid(1., 1., 1.), Material::rgb(0xe0e0e0)).at([0., 0.5, 0.]));
    world.add_light(Light::Directional {
        direction: vec3(-0.4, -1., -0.3),
        color: [1.; 3],
        intensity: 2.,
    });
    world.set_camera(Camera::looking_at([0., 4., 9.], [0., 0., 0.]));
    (world, cube)
}

/// What a row does: turns its feature on in `world` (its mesh given), and
/// hands back what turns it off; `None` if this device can't hold it (it
/// says why).
type Hold = fn(&World, ObjectId, &mut Window) -> Option<Box<dyn FnOnce(&World)>>;

/// Holds `hold`'s reason, turns it off, and asserts the world rests.
fn rests(name: &str, hold: Hold) {
    let (world, mesh) = world();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw(&world);
    let Some(off) = hold(&world, mesh, &mut window) else {
        eprintln!("SETTLE-SKIPPED {name}: this device can't hold it");
        return;
    };
    let held = window.until(&world, 2, Duration::from_secs(10), || {
        world.view().animating()
    });
    assert!(held, "{name}: the control, its world never drew by itself");
    off(&world);
    let rested = window.until(&world, 8, Duration::from_secs(10), || {
        !world.view().animating()
    });
    assert!(
        rested,
        "{name}: turned off, its world still draws by itself"
    );
}

#[cfg(feature = "lighting")]
#[test]
fn cascades_turned_off_while_one_is_owed() {
    rests("cascades off", |world, _, window| {
        world.set_shadows(true);
        world.set_cascades(Some(Cascades::new(4).distance(40.)));
        window.draw(world);
        // A turn of the camera: the far cascades are owed a frame each.
        world.set_camera(Camera::looking_at([5., 5., 7.], [-2., 0., 1.]));
        Some(Box::new(|world| world.set_cascades(None)))
    });
}

#[cfg(feature = "lighting")]
#[test]
fn sun_shadows_turned_off_while_a_cascade_is_owed() {
    rests("shadows off", |world, _, window| {
        world.set_shadows(true);
        world.set_cascades(Some(Cascades::new(4).distance(40.)));
        window.draw(world);
        world.set_camera(Camera::looking_at([5., 5., 7.], [-2., 0., 1.]));
        Some(Box::new(|world| world.set_shadows(false)))
    });
}

#[cfg(feature = "shaders")]
#[test]
fn an_animated_shader_taken_off_its_mesh() {
    rests("animated shader", |world, mesh, _| {
        let pulse = world
            .add_shader(
                MaterialShader::new(
                    "#zgx material 1
                     fn surface(s: SurfaceIn) -> Surface {
                         var m = surface_default(s);
                         m.base_color *= 0.5 + 0.5 * sin(frame.time);
                         return m;
                     }",
                )
                .animated(),
            )
            .unwrap();
        world.set_shader(mesh, Some(pulse));
        Some(Box::new(move |world| world.set_shader(mesh, None)))
    });
}

#[cfg(feature = "compute")]
#[test]
fn a_compute_pass_due_every_frame_removed() {
    rests("compute", |world, _, _| {
        let count = world.add_buffer(BufferDesc::storage::<u32>(1)).unwrap();
        let pass = world
            .add_compute(
                Compute::new(
                    "@group(0) @binding(0) var<storage, read_write> count: array<u32>;
                     @compute @workgroup_size(1) fn main() { count[0] += 1u; }",
                )
                .buffer("count", &count, Access::ReadWrite)
                .every_frame(),
            )
            .unwrap();
        Some(Box::new(move |world| world.remove_compute(pass)))
    });
}

#[cfg(feature = "particles")]
#[test]
fn an_emitter_removed_while_its_particles_live() {
    rests("particles", |world, _, _| {
        let rain = world
            .add_emitter(
                Emitter::new(1000)
                    .spawn(Spawn::box_([-1., 1., -1.], [1., 1., 1.]).rate(500.))
                    .lifetime(0.3..0.4)
                    .velocity([0., -5., 0.], 0.)
                    .draw(Shape::Streak { length: 0.05 }, Blend::Additive),
            )
            .unwrap();
        Some(Box::new(move |world| world.remove_emitter(rain)))
    });
}

#[cfg(feature = "post")]
#[test]
fn an_animated_post_pass_removed() {
    rests("post pass", |world, _, _| {
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
        Some(Box::new(move |world| world.remove_post(pulse)))
    });
}

#[cfg(feature = "post")]
#[test]
fn automatic_exposure_turned_off_while_it_adapts() {
    rests("exposure", |world, _, window| {
        world.set_post(Post {
            auto_exposure: Some(AutoExposure::default()),
            ..Default::default()
        });
        // Only where the device runs compute does exposure adapt.
        window.draw(world);
        window.draw(world);
        if !world.view().animating() {
            return None;
        }
        Some(Box::new(|world| world.set_post(Post::default())))
    });
}

#[cfg(feature = "post")]
#[test]
#[ignore = "TA1: graph lane (the painter's hold on gathering frames is never let go)"]
fn temporal_antialiasing_turned_to_multisampling_while_it_gathers() {
    rests("taa to msaa", |world, _, window| {
        world.set_antialiasing(Antialiasing::Taa);
        window.draw(world);
        Some(Box::new(|world| world.set_antialiasing(Antialiasing::Msaa)))
    });
}

#[cfg(feature = "post")]
#[test]
#[ignore = "TA1: graph lane (the painter's hold on gathering frames is never let go)"]
fn temporal_antialiasing_turned_off_while_it_gathers() {
    rests("taa to none", |world, _, window| {
        world.set_antialiasing(Antialiasing::Taa);
        window.draw(world);
        Some(Box::new(|world| world.set_antialiasing(Antialiasing::None)))
    });
}

#[cfg(feature = "gi")]
#[test]
fn probes_removed_while_they_converge() {
    rests("probes", |world, _, window| {
        let probes = world
            .add_probes(ProbeVolume::new([-2., 0.05, -2.], [2., 2., 2.], [6, 6, 6]).per_frame(8))
            .unwrap();
        window.draw(world);
        window.draw(world);
        if !world.probes_traced() {
            return None;
        }
        Some(Box::new(move |world| world.remove_probes(probes)))
    });
}

// Removing the last animator lets its world's own hold go outside a frame,
// and the frame then skips the step that would let its painter's go
// (src/world/animation.rs:452-457, `animation::frame` with no slots).
#[cfg(feature = "gltf")]
#[test]
#[ignore = "SETTLE-ANIM: the animation lane (removing the last animator never rests)"]
fn the_last_animator_removed_while_it_plays() {
    rests("animation", |world, _, _| {
        let model = Model::from_bytes(&rig::rig(3)).unwrap();
        let clip = model.clip("Wave").unwrap().clone();
        let placed = world.place_model(&model);
        let animator = world.add_animator(&placed);
        world.play(animator, &clip, Play::new()).unwrap();
        Some(Box::new(move |world| world.remove_animator(animator)))
    });
}
