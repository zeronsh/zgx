//! The rests-once-off rows' harness (`settle.rs`, `settle_compile.rs`):
//! a headless window, the world every row starts from, and the check.
#![allow(dead_code)]
use std::time::{Duration, Instant};
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

/// Whether the world draws by itself after a frame: where what holds it
/// finishes off the frame (a compile, a load), it may be done before a row
/// can catch it, and the row then says so and skips.
pub fn caught(world: &World, window: &mut Window) -> bool {
    window.draw(world);
    world.view().animating()
}

pub struct Window {
    ui: Ui,
    gpu: GpuRenderer,
}
impl Window {
    pub fn new(world: &World) -> Option<Self> {
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
    pub fn draw(&mut self, world: &World) {
        world.view().invalidate();
        self.ui.prepare_frame();
        let report = self.ui.scene.borrow_mut().flush();
        self.gpu
            .render(&self.ui.scene.borrow(), &report.damage)
            .unwrap();
    }
    /// Frames until `done`, `frames` at least and `cap` at most in time:
    /// whether it came.
    pub fn until(
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
pub fn world() -> (World, ObjectId) {
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
pub type Hold = fn(&World, ObjectId, &mut Window) -> Option<Box<dyn FnOnce(&World)>>;

/// Holds `hold`'s reason, turns it off, and asserts the world rests:
/// whether it could (a device that can't hold it, or a hold already over
/// before the row caught it, couldn't).
pub fn try_rests(name: &str, hold: Hold) -> bool {
    let (world, mesh) = world();
    let Some(mut window) = Window::new(&world) else {
        return false;
    };
    window.draw(&world);
    let Some(off) = hold(&world, mesh, &mut window) else {
        return false;
    };
    // Held already (a row that caught it), or once a frame or two follows.
    let held = world.view().animating()
        || window.until(&world, 2, Duration::from_secs(10), || {
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
    true
}

/// `try_rests` for a reason a device may not hold (it says so and passes).
pub fn rests(name: &str, hold: Hold) {
    if !try_rests(name, hold) {
        eprintln!("SETTLE-SKIPPED {name}: this device can't hold it");
    }
}

/// `try_rests` for a hold that ends on its own off the frame (a compile, a
/// load): tried in fresh worlds until one catches it, and failing where
/// none did: a row that tested nothing doesn't pass. (Where its worker can
/// be held, a test holds it: `settle_compile.rs`.)
pub fn rests_caught(name: &str, hold: Hold) {
    const TRIES: u32 = 20;
    if GpuRenderer::new(1, 1).is_err() && std::env::var_os("ZGX_REQUIRE_GPU").is_none() {
        return;
    }
    let caught = (0..TRIES).any(|_| try_rests(name, hold));
    assert!(
        caught,
        "{name}: its hold was over before a frame in all {TRIES} tries: it tested nothing"
    );
}
