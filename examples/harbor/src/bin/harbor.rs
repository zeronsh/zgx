//! The harbor in a zgx window: the fly-through on the frame's time, or,
//! with "Fly-through" off, the orbit controls (drag to turn, right-drag or
//! Shift-drag to pan, scroll or pinch to zoom). The HUD's settings write
//! signals the world is bound to. Orbiting and left alone, it draws
//! nothing.
//!
//! Run: cargo run --release --bin harbor (from examples/harbor)
//! `--smoke-test` flies for two seconds, prints what it drew, and quits.
use std::{cell::RefCell, rc::Rc, time::Instant};
use zgx::{compose::prelude::*, input::InputEvent};
use zgx_3d::{Camera, Orbit, OrbitControls, World};
use zgx_desktop::{Application, ScopedTask, WindowOptions};
use zgx_harbor::{
    hud::{self, Stats},
    path,
    scene::{Config, Harbor},
    settings::Settings,
    sky,
    weather::Rain,
};

/// The orbit that keeps `camera`'s view: its target, and where it is
/// about it.
fn orbit_of(camera: Camera) -> Orbit {
    let offset = camera.position - camera.target;
    let distance = offset.length().max(1.);
    Orbit {
        target: camera.target,
        azimuth: offset.x.atan2(offset.z),
        elevation: (offset.y / distance).clamp(-1., 1.).asin(),
        distance,
        elevations: (0.02, 1.5),
        distances: (8., 1500.),
        ..Orbit::default()
    }
}

/// The camera at `camera`, the rain's box over it.
fn look(world: &World, rain: Option<Rain>, camera: Camera) {
    world.set_camera(camera);
    if let Some(rain) = rain {
        rain.follow(world, camera.position);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let smoke = std::env::args().any(|arg| arg == "--smoke-test");
    let config = Config::default();
    // 11–16 ms to prefilter, and more to make: a thread of its own while
    // the window opens.
    let sun = sky::sun_toward(config.sun.0, config.sun.1);
    let environment = std::thread::spawn(move || sky::dawn(sun, 512));
    // GPU timestamps and compressed textures where the GPU has them: asked
    // before `run`, which decides before it starts the device.
    zgx_3d::request_view_features();
    // SAFETY: the harbor takes on wgpu's experimental contract for ray
    // queries (zgx_gpu::request_ray_queries), for its probes to trace;
    // asked before the first window, and its device, are made.
    unsafe { zgx_gpu::request_ray_queries() };
    Application::new()
        .window(WindowOptions {
            title: "Harbor at dawn".into(),
            width: 1280.,
            height: 760.,
            ..Default::default()
        })
        .run(move |cx| {
            let world = World::new();
            let harbor = Harbor::build(&world, &config, environment.join().ok());
            let rain = harbor.rain;
            let settings = Settings::new(&cx.ui.runtime, &harbor);
            let mut keep = settings.bind(&cx.ui.runtime, &world, &harbor);
            let stats = cx.ui.signal(Stats {
                objects: harbor.objects,
                triangles: harbor.triangles,
                containers: harbor.yard.len() + harbor.deck.len(),
                lights: (harbor.lamps.len(), harbor.lamps.len()),
                ..Stats::default()
            });
            let controls = Rc::new(RefCell::new(OrbitControls::new(orbit_of(world.camera()))));
            let base = world.camera();
            let flown = Rc::new(std::cell::Cell::new(0_f32));

            // Flying: a task moving the camera along the path each frame,
            // by the frame's interval. Not flying: no task, the orbit
            // taking over from where the flight was.
            let flight: Rc<RefCell<Option<ScopedTask>>> = Rc::default();
            keep.push(cx.ui.runtime.effect({
                let (world, frames, tasks) = (world.clone(), cx.frames.clone(), cx.tasks.clone());
                let (fly, stats, controls, flight, flown) = (
                    settings.fly.clone(),
                    stats.clone(),
                    controls.clone(),
                    flight.clone(),
                    flown.clone(),
                );
                move || {
                    let mut flight = flight.borrow_mut();
                    if !fly.get() {
                        if flight.take().is_some() {
                            *controls.borrow_mut() = OrbitControls::new(orbit_of(world.camera()));
                        }
                        return;
                    }
                    let (world, frames, stats, flown) =
                        (world.clone(), frames.clone(), stats.clone(), flown.clone());
                    *flight = Some(tasks.spawn_scoped(async move {
                        let (mut count, mut since) = (0_u32, Instant::now());
                        loop {
                            let frame = frames.next().await;
                            let t = (flown.get() + frame.interval.as_secs_f32()) % path::LOOP;
                            flown.set(t);
                            look(&world, rain, path::camera(t, base));
                            count += 1;
                            let elapsed = since.elapsed().as_secs_f32();
                            if elapsed >= 0.25 {
                                stats.update(|s| {
                                    s.fps = count as f32 / elapsed;
                                    s.frame_ms = elapsed * 1e3 / count as f32;
                                    s.time = t;
                                    s.revision = world.revision();
                                    s.lights.1 =
                                        s.lights.0.saturating_sub(world.lights_left_out() as usize);
                                });
                                (count, since) = (0, Instant::now());
                            }
                        }
                    }));
                }
            }));

            // A flick of the orbit coasts on, a frame at a time, until it
            // stops.
            let coast: Rc<RefCell<Option<ScopedTask>>> = Rc::default();
            let viewport = gpu_view("The harbor", world.view())
                .focusable(true)
                .on_event({
                    let (world, fly, frames, tasks) = (
                        world.clone(),
                        settings.fly.clone(),
                        cx.frames.clone(),
                        cx.tasks.clone(),
                    );
                    let stats = stats.clone();
                    move |event| {
                        // The effects live as long as the view does.
                        let _ = &keep;
                        if fly.with_untracked(|f| *f) {
                            return;
                        }
                        let modifiers = match &event.event {
                            InputEvent::KeyDown { modifiers, .. } => *modifiers,
                            _ => event.pointer_modifiers(),
                        };
                        let answer =
                            controls
                                .borrow_mut()
                                .handle(&event.event, modifiers, Instant::now());
                        if answer.moved {
                            let camera = controls.borrow().orbit.camera(world.camera());
                            look(&world, rain, camera);
                            stats.update(|s| s.revision = world.revision());
                        }
                        if answer.coasting {
                            let (world, frames, controls) =
                                (world.clone(), frames.clone(), controls.clone());
                            *coast.borrow_mut() = Some(tasks.spawn_scoped(async move {
                                loop {
                                    let frame = frames.next().await;
                                    let mut c = controls.borrow_mut();
                                    let going = c.coast(frame.interval.as_secs_f32());
                                    look(&world, rain, c.orbit.camera(world.camera()));
                                    if !going {
                                        break;
                                    }
                                }
                            }));
                        }
                        if answer.taken {
                            event.prevent_default();
                        }
                    }
                });
            cx.render(hud::view(viewport, &settings, stats.clone()));

            if smoke {
                let (window, world) = (cx.window.clone(), world.clone());
                cx.tasks.spawn(async move {
                    let still = world.revision();
                    zgx::timer::sleep(std::time::Duration::from_secs(2)).await;
                    println!(
                        "HARBOR changes={} flown={:.2}s objects={}",
                        world.revision() - still,
                        flown.get(),
                        stats.with_untracked(|s| s.objects)
                    );
                    window.close();
                });
            }
        })
}
