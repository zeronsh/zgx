//! A sea whose waves a compute pass makes, in the harbor's terms (its wind,
//! its chop): wind waves from a spectrum summed into a float texture each
//! frame (`sea/waves.wgsl`), which the water's material reads to move its
//! grid and bend its normal (`sea/water.wgsl`). The public API alone:
//! `World::add_buffer`, `add_texture`, `add_compute` and a `MaterialShader`
//! reading the texture (`Param::GpuTexture`).
//!
//! The waves lie on the lattice of a square tile, so the tile repeats
//! without a seam over any size of sea, and their heights add up to at most
//! what the material says it may move (`displaces_by`). A still sea
//! (`Waves::set_running(false)`) runs no pass and its material isn't
//! animated: its world draws nothing until something changes.
//!
//! Run: cargo run --release -p zgx-3d --features compute,shaders --example sea
//! (`WIND=12` m/s, `CHOP=1`; `--still` starts it still; `--smoke-test`
//! shows it a moment and quits.)
//! `-- --bench`: no window; GPU frames of the sea moving, still, and flat
//! (the control) at 1080p, each arm's frames waited for in turn.
use std::time::{Duration, Instant};
use zgx_3d::*;

/// The tile the waves repeat over, in metres, and its texels a side.
const TILE: f32 = 128.;
const TEXELS: u32 = 256;
/// Waves summed: the spectrum's strongest on the tile's lattice.
const WAVES: usize = 64;
/// What the waves' heights add up to, in metres, at a chop of 1 (the
/// harbor's six reach 0.375 m): the most the surface moves.
const REACH: f32 = 0.4;

/// A sea's waves: the spectrum, the pass making them, and the texture they're in.
struct Waves {
    /// The sea's clock: seconds, how fast it runs, the tile, the waves.
    clock: GpuBuffer,
    passes: [ComputeId; 2],
    field: GpuTexture,
    /// What the material may move its vertices by, in metres.
    reach: f32,
    _waves: GpuBuffer,
}

impl Waves {
    /// Waves of the wind blowing `wind` (x, z, m/s) at `chop` times their
    /// height, made in `world`.
    fn new(world: &World, wind: [f32; 2], chop: f32) -> Result<Self, String> {
        let (spectrum, reach) = spectrum(wind, chop);
        let waves = world
            .add_buffer(BufferDesc::from_slice(&spectrum))
            .map_err(|e| e.to_string())?;
        let clock = world
            .add_buffer(BufferDesc::from_slice(&[[0., 1., TILE, WAVES as f32]]))
            .map_err(|e| e.to_string())?;
        let field = world
            .add_texture(TextureDesc::storage(TEXELS, TEXELS, Format::Rgba16Float))
            .map_err(|e| e.to_string())?;
        let source = include_str!("sea/waves.wgsl");
        // The clock first, then the field at its time (passes run in the
        // order added).
        let tick = world
            .add_compute(
                Compute::new(source)
                    .entry("tick")
                    .buffer("clock", &clock, Access::ReadWrite)
                    .every_frame(),
            )
            .map_err(|e| e.to_string())?;
        let make = world
            .add_compute(
                Compute::new(source)
                    .entry("field_at")
                    .buffer("waves", &waves, Access::Read)
                    .buffer("now", &clock, Access::Read)
                    .texture("field", &field, Access::Write)
                    .threads([TEXELS, TEXELS, 1])
                    .every_frame(),
            )
            .map_err(|e| e.to_string())?;
        Ok(Self {
            clock,
            passes: [tick, make],
            field,
            reach,
            _waves: waves,
        })
    }

    /// Moving, or still: a still sea runs no pass, so its world can rest.
    fn set_running(&self, world: &World, running: bool) {
        for pass in self.passes {
            world.set_compute_enabled(pass, running);
        }
    }

    /// Whether its passes are made, so its waves move.
    fn ready(&self, world: &World) -> bool {
        self.passes
            .iter()
            .all(|&pass| world.compute_status(pass) == Some(ComputeStatus::Ready))
    }

    /// The water's material on `meshes`, the sea `deep` beneath.
    fn water(&self, world: &World, deep: [f32; 4]) -> Result<ShaderId, String> {
        let shader = world
            .add_shader(
                MaterialShader::new(include_str!("sea/water.wgsl"))
                    .param("field", Param::GpuTexture)
                    .param("deep", Param::Color)
                    .param("tile", Param::Float)
                    .displaces_by(self.reach),
            )
            .map_err(|e| e.to_string())?;
        world
            .set_param(shader, "field", &self.field)
            .map_err(|e| e.to_string())?;
        world
            .set_param(shader, "deep", deep)
            .map_err(|e| e.to_string())?;
        world
            .set_param(shader, "tile", TILE)
            .map_err(|e| e.to_string())?;
        let _ = &self.clock;
        Ok(shader)
    }
}

/// The waves of the wind `wind` (x, z, m/s) on the tile's lattice: each
/// wave vector `2π (n, m) / TILE` weighed by a Phillips spectrum, the
/// `WAVES` strongest kept, their heights scaled to add up to `REACH`
/// times `chop`. Each: wave vector (x, z), height, phase. And that sum.
fn spectrum(wind: [f32; 2], chop: f32) -> (Vec<[f32; 4]>, f32) {
    const G: f32 = 9.81;
    let speed = (wind[0] * wind[0] + wind[1] * wind[1]).sqrt().max(0.5);
    let to = [wind[0] / speed, wind[1] / speed];
    // The longest wave the wind raises, and the shortest kept (ripples are
    // the material's own business).
    let longest = speed * speed / G;
    let shortest = 1.0f32;
    let mut all = Vec::new();
    for n in -24i32..=24 {
        for m in -24i32..=24 {
            let k = [
                std::f32::consts::TAU * n as f32 / TILE,
                std::f32::consts::TAU * m as f32 / TILE,
            ];
            let length = (k[0] * k[0] + k[1] * k[1]).sqrt();
            if length == 0. {
                continue;
            }
            let along = (k[0] * to[0] + k[1] * to[1]) / length;
            // Waves running with the wind, not against it.
            if along <= 0. {
                continue;
            }
            let kl = length * longest;
            let damp = (-(length * shortest).powi(2)).exp();
            let power = (-1. / (kl * kl)).exp() / length.powi(4) * along * along * damp;
            // A phase of its own, the same each run.
            let hash = (n.wrapping_mul(73_856_093) ^ m.wrapping_mul(19_349_663)) as u32;
            let phase = (hash % 6283) as f32 / 1000.;
            all.push([k[0], k[1], power.sqrt(), phase]);
        }
    }
    all.sort_by(|a, b| b[2].total_cmp(&a[2]));
    all.truncate(WAVES);
    let sum: f32 = all.iter().map(|w| w[2]).sum::<f32>().max(1e-6);
    let reach = REACH * chop.max(0.);
    for w in &mut all {
        w[2] *= reach / sum;
    }
    (all, reach)
}

/// A grid `size` metres square of `step` metre cells about the origin,
/// facing up: for the waves to move.
fn grid(size: f32, step: f32) -> Geometry {
    let cells = (size / step) as u32;
    let side = cells + 1;
    let half = size * 0.5;
    let mut vertices = Vec::with_capacity((side * side) as usize);
    for row in 0..side {
        for column in 0..side {
            let (u, v) = (column as f32 / cells as f32, row as f32 / cells as f32);
            vertices.push(Vertex {
                position: [u * size - half, 0., v * size - half],
                normal: [0., 1., 0.],
                uv: [u, v],
            });
        }
    }
    let mut indices = Vec::with_capacity((cells * cells * 6) as usize);
    for row in 0..cells {
        for column in 0..cells {
            let at = |c: u32, r: u32| r * side + c;
            // As `Geometry::plane` winds: facing up.
            let (a, b) = (at(column, row + 1), at(column + 1, row + 1));
            let (c, d) = (at(column + 1, row), at(column, row));
            indices.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    Geometry::new(vertices, indices).expect("a grid's indices")
}

/// A world of sea under a low sun: the grid close in, the camera at a
/// quay's height looking out over it.
fn sea_world(wind: [f32; 2], chop: f32, waves: bool) -> (World, Option<Waves>) {
    let world = World::new();
    world.set_background(Some(0x6a7f91));
    world.set_camera(Camera::looking_at([0., 6., 40.], [0., 0., -60.]));
    world.add_light(Light::Directional {
        direction: vec3(-0.4, -0.35, -1.),
        color: [1., 0.92, 0.82],
        intensity: 3.,
    });
    world.add_light(Light::Hemisphere {
        sky: [0.55, 0.65, 0.8],
        ground: [0.1, 0.12, 0.14],
        intensity: 0.8,
    });
    let deep = [0.006, 0.022, 0.026, 1.];
    let mut material = Material::default().with(0., 0.04);
    material.color = deep;
    let sea = world.add(Object::mesh(grid(256., 1.), material));
    let made = waves.then(|| {
        let made = Waves::new(&world, wind, chop).expect("the waves");
        let water = made.water(&world, deep).expect("the water");
        world.set_shader(sea, Some(water));
        made
    });
    (world, made)
}

fn setting(name: &str, default: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let wind = [0.6 * setting("WIND", 12.), -0.8 * setting("WIND", 12.)];
    let chop = setting("CHOP", 1.);
    if std::env::args().any(|arg| arg == "--bench") {
        bench(wind, chop);
        return Ok(());
    }
    let smoke = std::env::args().any(|arg| arg == "--smoke-test");
    let still = std::env::args().any(|arg| arg == "--still");
    zgx_3d::request_view_features();
    zgx_desktop::Application::new()
        .window(zgx_desktop::WindowOptions {
            title: "zgx 3D · sea".into(),
            width: 960.,
            height: 540.,
            ..Default::default()
        })
        .run(move |cx| {
            use zgx::compose::prelude::*;
            let (world, waves) = sea_world(wind, chop, true);
            if still && let Some(waves) = &waves {
                waves.set_running(&world, false);
            }
            cx.render(gpu_view("Sea", world.view()).w_full().h_full());
            // Kept for as long as the window shows them.
            std::mem::forget((world, waves));
            if smoke {
                let window = cx.window.clone();
                cx.tasks.spawn(async move {
                    zgx::timer::sleep(Duration::from_secs(2)).await;
                    window.close();
                });
            }
        })?;
    Ok(())
}

/// GPU time a frame of the sea at 1080p: frames of each arm drawn back to
/// back and waited for, arms in turn. Flat: the same grid in a plain
/// material, no pass; moving: the waves made each frame and the grid moved
/// by them; still: the passes off (a still sea's world doesn't animate, so
/// a window draws none of these frames at all: drawn here only to show
/// what one costs if something else draws it).
fn bench(wind: [f32; 2], chop: f32) {
    use zgx_gpu::{
        viewport::{Viewport, ViewportFrame},
        wgpu,
    };
    const SIZE: (u32, u32) = (1920, 1080);
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
    let rounds = setting("ROUNDS", 5.) as usize;
    let frames = setting("FRAMES", 240.) as usize;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster(instance.request_adapter(&Default::default())).expect("an adapter");
    println!("{:?} on {}", adapter.get_info().backend, adapter.get_info().name);
    let (device, queue) = pollster(adapter.request_device(&wgpu::DeviceDescriptor {
        // As a world made before its window asks (`request_view_features`).
        required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))
    .expect("a device");
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: SIZE.0,
            height: SIZE.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[FORMAT.add_srgb_suffix()],
    });
    let view = target.create_view(&Default::default());
    let srgb = target.create_view(&wgpu::TextureViewDescriptor {
        format: Some(FORMAT.add_srgb_suffix()),
        ..Default::default()
    });
    struct Arm {
        name: &'static str,
        world: World,
        waves: Option<Waves>,
        painter: Box<dyn Viewport>,
        frame: u64,
    }
    let start = Instant::now();
    let mut arms: Vec<Arm> = [("flat", false), ("moving", true), ("still", true)]
        .into_iter()
        .map(|(name, waves)| {
            let (world, waves) = sea_world(wind, chop, waves);
            Arm {
                name,
                painter: Box::new(world.viewport()),
                world,
                waves,
                frame: 0,
            }
        })
        .collect();
    let draw = |arm: &mut Arm| {
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut frame = ViewportFrame::new(&device, &queue, &mut encoder, &view, FORMAT, SIZE, 1.);
        frame.target_srgb = Some(&srgb);
        frame.time = start + Duration::from_micros(arm.frame * 16_667);
        arm.frame += 1;
        arm.painter.render(&mut frame);
        queue.submit([encoder.finish()]);
    };
    // Everything made (the passes and the material's pipelines), a second
    // drawn, then the still arm stilled.
    for arm in &mut arms {
        let began = Instant::now();
        loop {
            draw(arm);
            let ready = arm.waves.as_ref().is_none_or(|w| w.ready(&arm.world))
                && !arm.world.compiling();
            if ready || began.elapsed() > Duration::from_secs(20) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        for _ in 0..60 {
            draw(arm);
        }
        if arm.name == "still"
            && let Some(waves) = &arm.waves
        {
            waves.set_running(&arm.world, false);
            draw(arm);
        }
        println!(
            "  {}: its world {}",
            arm.name,
            if arm.world.view().animating() {
                "keeps drawing"
            } else {
                "rests"
            }
        );
    }
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); arms.len()];
    for _ in 0..rounds {
        for (at, arm) in arms.iter_mut().enumerate() {
            let began = Instant::now();
            for _ in 0..frames {
                draw(arm);
            }
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            times[at].push(began.elapsed().as_secs_f64() * 1000. / frames as f64);
        }
    }
    let median = |t: &mut Vec<f64>| {
        t.sort_by(f64::total_cmp);
        t[t.len() / 2]
    };
    let [flat, moving, still] = [0, 1, 2].map(|at| median(&mut times[at]));
    println!(
        "median a frame: flat {flat:.3} ms, moving {moving:.3} ms, still {still:.3} ms: \
         the waves cost {:.3} ms a frame moving, {:.3} ms still (and a still sea draws no frame)",
        moving - flat,
        still - flat
    );
}

fn pollster<T>(future: impl std::future::Future<Output = T>) -> T {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
