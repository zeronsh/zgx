//! The Bevy arm: the scene drawn by Bevy's own glTF loader and PBR renderer
//! into an offscreen image, in Bevy's default app loop (pipelined rendering,
//! multi-threaded), the camera moving along the path every update.
//!
//! Bevy's render diagnostics time the GPU only where timestamps can be
//! written inside an encoder, which Metal can't, so this arm measures it as
//! the zgx arm does: an empty pass over the frame's target holding a
//! timestamp, recorded before the camera's schedule (shadows, prepasses,
//! main passes, tone mapping) and one after it, in the same submission.
#[path = "../../common/report.rs"]
mod report;
#[path = "../../common/rusage.rs"]
mod rusage;

use bevy::{
    app::ScheduleRunnerPlugin,
    asset::RenderAssetUsages,
    camera::RenderTarget,
    core_pipeline::tonemapping::Tonemapping,
    gltf::GltfAssetLabel,
    image::Image,
    light::{
        CascadeShadowConfigBuilder, DirectionalLightShadowMap, GeneratedEnvironmentMapLight,
        ShadowFilteringMethod,
    },
    prelude::*,
    render::{
        Extract, ExtractSchedule, RenderApp, RenderPlugin, RenderSystems,
        gpu_readback::{Readback, ReadbackComplete},
        render_asset::RenderAssets,
        render_resource::{
            Extent3d, LoadOp, Operations, RenderPassColorAttachment, RenderPassDescriptor, StoreOp,
            TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
            TextureViewDimension,
        },
        renderer::{
            PendingCommandBuffers, RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue,
        },
        settings::{Backends, PowerPreference, RenderCreation, WgpuFeatures, WgpuSettings},
        texture::GpuImage,
    },
    window::ExitCondition,
    world_serialization::{WorldAsset, WorldAssetRoot},
};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use wgpu::RenderPassTimestampWrites;

/// The background every arm clears to, sRGB.
const CLEAR: [u8; 3] = [0x6b, 0x7a, 0x8f];
/// Ticks of the still camera, at 120 Hz: ten seconds, after which the
/// footprint is read again (Bevy draws on through them).
const REST_TICKS: usize = 1200;
/// Frames in a row with no pipeline compiling before warm-up starts.
const SETTLED: usize = 10;
/// Timestamps a frame: before everything, after GPU preprocessing, after
/// the shadow passes, after the main pass, after everything.
const SLOTS: usize = 5;

/// What both worlds share.
#[derive(Resource, Clone)]
struct Shared {
    image: Handle<Image>,
    /// Pipelines the render world is still compiling.
    compiling: Arc<AtomicUsize>,
    /// Frames the render world ran.
    rendered: Arc<AtomicUsize>,
    /// Each timed frame's GPU time (warm-up, then measured), filled at the
    /// end, and its parts: before shadows (preprocessing), shadows, main
    /// pass, after (tone map, blit).
    gpu_ms: Arc<Mutex<Option<Vec<f64>>>>,
    gpu_parts: Arc<Mutex<Vec<[f64; 4]>>>,
    /// Each timed frame's render-world CPU time, from its first set to the
    /// submission's last encoder.
    render_ms: Arc<Mutex<Vec<f64>>>,
    finish: Arc<AtomicBool>,
    /// Frames timed: warm-up and measured.
    frames: usize,
}

/// A cube of the generated scene that moves: its index.
#[derive(Component)]
struct Moving(usize);

#[derive(PartialEq, Clone, Copy, Debug)]
enum Phase {
    Loading,
    Settling(usize),
    Warmup(usize),
    Measuring(usize),
    Rest(usize),
    Readback,
    Done,
}

#[derive(Resource)]
struct Run {
    setup: report::Setup,
    phase: Phase,
    model: Handle<WorldAsset>,
    frame_ms: Vec<f64>,
    last: Option<Instant>,
    started: Option<(Instant, rusage::Sample)>,
    measured: Option<(f64, rusage::Sample)>,
    /// Each measured frame's submission as Metal timed it.
    spans_ms: Vec<f64>,
    /// The GPU's busy seconds over the measured frames, and the span.
    busy: (f64, f64),
    rest_from: Option<(usize, rusage::Sample)>,
    rest: Option<(u64, f64)>,
    environment_ms: f64,
    loading_from: Instant,
    model_ms: f64,
    /// From the process's start to the first frame with everything loaded
    /// and every pipeline compiled, and the footprint's peak by then.
    started_at: Instant,
    load: Option<(f64, u64)>,
    rested_footprint: u64,
    /// Meshes and triangles the camera's frustum kept, each measured frame;
    /// meshes in all.
    visible: Vec<(usize, usize)>,
    meshes: usize,
    mips_made: bool,
}

fn main() {
    let started_at = Instant::now();
    let setup = report::setup();
    let folder = Path::new(&setup.model).parent().unwrap().to_path_buf();
    let frames = setup.warmup + setup.path.len();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: folder.to_string_lossy().into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                    backends: Some(Backends::METAL),
                    power_preference: PowerPreference::HighPerformance,
                    features: WgpuFeatures::TIMESTAMP_QUERY,
                    ..default()
                })),
                ..default()
            }),
    )
    .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::ZERO))
    .insert_resource(ClearColor(Color::srgb_u8(CLEAR[0], CLEAR[1], CLEAR[2])))
    .insert_resource(DirectionalLightShadowMap { size: 2048 })
    .insert_resource(GlobalAmbientLight::NONE);

    let image = {
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let mut target = Image::new_target_texture(
            setup.width,
            setup.height,
            TextureFormat::Bgra8UnormSrgb,
            None,
        );
        target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        images.add(target)
    };
    let shared = Shared {
        image,
        compiling: Arc::default(),
        rendered: Arc::default(),
        gpu_ms: Arc::default(),
        gpu_parts: Arc::default(),
        render_ms: Arc::default(),
        finish: Arc::default(),
        frames,
    };
    app.insert_resource(shared.clone())
        .insert_resource(Run {
            setup,
            phase: Phase::Loading,
            model: Handle::default(),
            frame_ms: Vec::new(),
            last: None,
            started: None,
            measured: None,
            rest_from: None,
            rest: None,
            environment_ms: 0.,
            loading_from: Instant::now(),
            model_ms: 0.,
            started_at,
            load: None,
            rested_footprint: 0,
            visible: Vec::new(),
            spans_ms: Vec::new(),
            busy: (0., 0.),
            meshes: 0,
            mips_made: false,
        })
        .add_systems(Startup, scene)
        .add_systems(Update, (drive, mips))
        .add_systems(
            PostUpdate,
            count_visible.after(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
        )
        .add_observer(read_back);

    let render = app.sub_app_mut(RenderApp);
    {
        use bevy::core_pipeline::{Core3d, Core3dSystems};
        use bevy::pbr::{
            EARLY_SHADOW_PASS, LATE_SHADOW_PASS, early_prepass_build_indirect_parameters,
            per_view_shadow_pass, shared_shadow_pass,
        };
        render.add_systems(
            Core3d,
            (
                inner_marker::<1>
                    .after(early_prepass_build_indirect_parameters)
                    .before(per_view_shadow_pass::<EARLY_SHADOW_PASS>)
                    .before(shared_shadow_pass::<EARLY_SHADOW_PASS>),
                inner_marker::<2>
                    .after(per_view_shadow_pass::<LATE_SHADOW_PASS>)
                    .after(shared_shadow_pass::<LATE_SHADOW_PASS>)
                    .before(Core3dSystems::MainPass),
                inner_marker::<3>
                    .after(Core3dSystems::MainPass)
                    .before(Core3dSystems::EarlyPostProcess),
            ),
        );
    }
    render
        .insert_resource(shared)
        .insert_resource(Measuring(None))
        .add_systems(ExtractSchedule, extract_phase)
        .add_systems(
            bevy::render::Render,
            (
                start_render_clock.in_set(RenderSystems::ExtractCommands),
                count_compiling.in_set(RenderSystems::Cleanup),
            ),
        )
        .add_systems(
            RenderGraph,
            (
                begin_marker.in_set(RenderGraphSystems::Begin),
                end_marker
                    .after(RenderGraphSystems::Render)
                    .before(RenderGraphSystems::Submit),
                resolve_timestamps.in_set(RenderGraphSystems::Finish),
            ),
        );
    app.run();
}

fn scene(
    mut commands: Commands,
    mut run: ResMut<Run>,
    shared: Res<Shared>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let started = Instant::now();
    let cube = images.add(environment_cube(&run.setup.environment, 256));
    run.environment_ms = started.elapsed().as_secs_f64() * 1e3;
    let name = Path::new(&run.setup.model)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    run.loading_from = Instant::now();
    // `boxes:N:M`: the generated instance scene, one mesh and one material
    // shared by every cube (so Bevy batches them), the first M moving.
    if let Some((count, moving)) = report::boxes(&run.setup.model) {
        let cube = meshes.add(Cuboid::new(1., 1., 1.));
        let material = materials.add(StandardMaterial {
            base_color: Color::srgb_u8(0xc8, 0xc0, 0xb4),
            metallic: 0.,
            perceptual_roughness: 0.6,
            ..default()
        });
        for i in 0..count {
            let at = Vec3::from(report::box_at(i, count, 0));
            let mut entity = commands.spawn((
                Mesh3d(cube.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(at),
            ));
            if i < moving {
                entity.insert(Moving(i));
            }
        }
    } else {
        run.model = assets.load(GltfAssetLabel::Scene(0).from_asset(name));
        commands.spawn(WorldAssetRoot(run.model.clone()));
    }
    let sun = Vec3::from_array(run.setup.sun).normalize();
    commands.spawn((
        DirectionalLight {
            // zgx's units: radiance in lux, diffuse over π, exposure 1.
            illuminance: 3.,
            color: Color::linear_rgb(1., 0.96, 0.9),
            // `--shadows=0`: none (the A/B for the shadow pass's cost).
            shadow_maps_enabled: !std::env::args().any(|a| a == "--shadows=0"),
            ..default()
        },
        Transform::default().looking_to(sun, Vec3::Y),
        // One map of 2048², as zgx's (Bevy's default is four cascades).
        CascadeShadowConfigBuilder {
            num_cascades: 1,
            minimum_distance: 0.1,
            maximum_distance: 40.,
            ..default()
        }
        .build(),
    ));
    // `--point-lights=N`: the light scene, N shadowless point lights
    // (clustered forward shading).
    let lights: usize = std::env::args()
        .find_map(|a| {
            a.strip_prefix("--point-lights=")
                .map(|v| v.parse().unwrap())
        })
        .unwrap_or(0);
    for i in 0..lights {
        let (position, [r, g, b]) = report::light_at(i);
        commands.spawn((
            PointLight {
                intensity: 150.,
                range: 8.,
                color: Color::linear_rgb(r, g, b),
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_translation(position.into()),
        ));
    }
    let (position, target) = run.setup.path[0];
    let mut camera = commands.spawn((
        Camera3d::default(),
        RenderTarget::Image(shared.image.clone().into()),
        Projection::Perspective(PerspectiveProjection {
            fov: run.setup.fov.to_radians(),
            near: 0.1,
            ..default()
        }),
        Transform::from_translation(position.into()).looking_at(Vec3::from(target), Vec3::Y),
        if run.setup.msaa == 1 {
            Msaa::Off
        } else {
            Msaa::Sample4
        },
        Tonemapping::AcesFitted,
        // Exposure 1: ev100 = log2(1 / 1.2).
        bevy::camera::Exposure { ev100: -0.263_034 },
        ShadowFilteringMethod::Hardware2x2,
        GeneratedEnvironmentMapLight {
            environment_map: cube,
            intensity: 1.,
            ..default()
        },
    ));
    // `--no-indirect=1`: Bevy's CPU path (no GPU preprocessing, culling or
    // indirect draws), the A/B for what its GPU-driven path saves.
    if std::env::args().any(|a| a == "--no-indirect=1") {
        camera.insert(bevy::render::view::NoIndirectDrawing);
    }
}

/// The equirectangular `.hdr` as a cube (+x, −x, +y, −y, +z, −z) of
/// `side`², sampled nearest, for Bevy to prefilter on the GPU. Its centre
/// column faces −z, as zgx reads it.
fn environment_cube(path: &str, side: u32) -> Image {
    let decoded = image::ImageReader::open(path)
        .unwrap()
        .decode()
        .unwrap()
        .into_rgb32f();
    let (w, h) = decoded.dimensions();
    let mut data = Vec::with_capacity((side * side * 6 * 8) as usize);
    for face in 0..6 {
        for y in 0..side {
            for x in 0..side {
                let u = 2. * (x as f32 + 0.5) / side as f32 - 1.;
                let v = 2. * (y as f32 + 0.5) / side as f32 - 1.;
                let d = match face {
                    0 => Vec3::new(1., -v, -u),
                    1 => Vec3::new(-1., -v, u),
                    2 => Vec3::new(u, 1., v),
                    3 => Vec3::new(u, -1., -v),
                    4 => Vec3::new(u, -v, 1.),
                    _ => Vec3::new(-u, -v, -1.),
                }
                .normalize();
                let s = 0.5 + d.x.atan2(-d.z) / std::f32::consts::TAU;
                let t = d.y.clamp(-1., 1.).acos() / std::f32::consts::PI;
                let px = ((s * w as f32) as u32).min(w - 1);
                let py = ((t * h as f32) as u32).min(h - 1);
                let p = decoded.get_pixel(px, py);
                for c in [p[0], p[1], p[2], 1.] {
                    data.extend_from_slice(&half::f16::from_f32(c).to_le_bytes());
                }
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image
}

fn drive(
    mut run: ResMut<Run>,
    shared: Res<Shared>,
    assets: Res<AssetServer>,
    mut camera: Query<&mut Transform, (With<Camera3d>, Without<Moving>)>,
    mut cubes: Query<(&mut Transform, &Moving), Without<Camera3d>>,
    cameras: Query<Entity, With<GeneratedEnvironmentMapLight>>,
    mut commands: Commands,
) {
    let now = Instant::now();
    let pose = |run: &Run, i: usize| {
        let (position, target) = run.setup.path[i.min(run.setup.path.len() - 1)];
        Transform::from_translation(position.into()).looking_at(Vec3::from(target), Vec3::Y)
    };
    run.phase = match run.phase {
        Phase::Loading => {
            let generated = report::boxes(&run.setup.model).is_some();
            if generated || assets.is_loaded_with_dependencies(&run.model) {
                run.model_ms = run.loading_from.elapsed().as_secs_f64() * 1e3;
                Phase::Settling(0)
            } else {
                Phase::Loading
            }
        }
        Phase::Settling(n) => {
            if shared.compiling.load(Ordering::Relaxed) > 0 {
                Phase::Settling(0)
            } else if n >= SETTLED {
                // Filtered once: Bevy refilters a GeneratedEnvironmentMapLight
                // every frame (its realtime mode), work zgx and Filament do
                // once at load. Its EnvironmentMapLight, which points at the
                // filtered cubes, stays (`--env-realtime=1` keeps refiltering).
                if !std::env::args().any(|a| a == "--env-realtime=1") {
                    for entity in &cameras {
                        commands
                            .entity(entity)
                            .remove::<GeneratedEnvironmentMapLight>();
                    }
                }
                run.load = Some((
                    run.started_at.elapsed().as_secs_f64() * 1e3,
                    rusage::sample().peak_footprint,
                ));
                Phase::Warmup(0)
            } else {
                Phase::Settling(n + 1)
            }
        }
        Phase::Warmup(n) if n + 1 >= run.setup.warmup => {
            wgpu::hal::metal::gpu_time::take_spans();
            wgpu::hal::metal::gpu_time::take_intervals();
            run.started = Some((now, rusage::sample()));
            run.last = Some(now);
            *camera.single_mut().unwrap() = pose(&run, 0);
            Phase::Measuring(0)
        }
        Phase::Warmup(n) => Phase::Warmup(n + 1),
        Phase::Measuring(i) => {
            let last = run.last.replace(now).unwrap();
            run.frame_ms.push((now - last).as_secs_f64() * 1e3);
            if i + 1 == run.setup.path.len() {
                let (from, _) = run.started.unwrap();
                run.measured = Some(((now - from).as_secs_f64(), rusage::sample()));
                run.busy = report::busy(wgpu::hal::metal::gpu_time::take_intervals());
                run.spans_ms = wgpu::hal::metal::gpu_time::take_spans()
                    .into_iter()
                    .map(|s| s * 1e3)
                    .collect();
                run.rest_from = Some((shared.rendered.load(Ordering::Relaxed), rusage::sample()));
                Phase::Rest(0)
            } else {
                *camera.single_mut().unwrap() = pose(&run, i + 1);
                if let Some((count, _)) = report::boxes(&run.setup.model) {
                    let frame = run.setup.warmup + i + 1;
                    for (mut transform, moving) in &mut cubes {
                        transform.translation = Vec3::from(report::box_at(moving.0, count, frame));
                    }
                }
                Phase::Measuring(i + 1)
            }
        }
        Phase::Rest(n) if n >= REST_TICKS => {
            let (from, before) = run.rest_from.unwrap();
            let after = rusage::sample();
            let frames = shared.rendered.load(Ordering::Relaxed) - from;
            run.rest = Some((frames as u64, (after.cpu_ns - before.cpu_ns) / 1e6));
            run.rested_footprint = after.footprint;
            shared.finish.store(true, Ordering::Relaxed);
            commands.spawn(Readback::texture(shared.image.clone()));
            Phase::Readback
        }
        Phase::Rest(n) => {
            // A 120 Hz display's pace, nothing moving.
            std::thread::sleep(Duration::from_micros(8333));
            Phase::Rest(n + 1)
        }
        phase => phase,
    };
}

fn read_back(
    done: On<ReadbackComplete>,
    mut run: ResMut<Run>,
    shared: Res<Shared>,
    mut exit: MessageWriter<AppExit>,
) {
    if run.phase != Phase::Readback {
        return;
    }
    let Some(gpu_ms) = shared.gpu_ms.lock().unwrap().clone() else {
        return; // the timestamps aren't read yet: the next readback
    };
    run.phase = Phase::Done;
    let (width, height) = (run.setup.width, run.setup.height);
    let row = if done.data.len() as u32 >= width.div_ceil(64) * 256 * height {
        (width * 4).div_ceil(256) * 256
    } else {
        width * 4
    };
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let at = (y * row) as usize;
        for px in done.data[at..at + (width * 4) as usize].chunks_exact(4) {
            rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]); // BGRA
        }
    }
    report::write_png(&run.setup.out, width, height, &rgba);
    let (hash, drawn) = report::proof(&rgba, CLEAR);
    let (elapsed, after) = run.measured.unwrap();
    let before = run.started.unwrap().1;
    let counted = run.setup.path.len();
    let (rest_frames, rest_cpu_ms) = run.rest.unwrap();
    let render_ms = shared.render_ms.lock().unwrap().clone();
    let parts = shared.gpu_parts.lock().unwrap().clone();
    let tail = |v: &[f64]| v[v.len().saturating_sub(counted)..].to_vec();
    let n = counted as f64;
    let mut json = report::Json::default();
    json.text("arm", "bevy")
        .text("adapter", "Metal (Bevy 0.19.1, wgpu 29)")
        .int("frames", run.frame_ms.len() as u64)
        .num("environment_ms", run.environment_ms)
        .num("model_ms", run.model_ms)
        .spread("gpu_ms", &run.spans_ms)
        .int("gpu_spans", run.spans_ms.len() as u64)
        // The GPU's busy time (every command buffer's interval, united), a
        // measured frame: Bevy submits more than once a frame, and frames
        // overlap on the GPU.
        .num("gpu_ms_per_frame", run.busy.0 * 1e3 / n)
        .num("gpu_busy_fraction", run.busy.0 / run.busy.1.max(1e-9))
        .spread("gpu_marker_ms", &tail(&gpu_ms))
        .spread("frame_ms", &run.frame_ms)
        .spread("submit_ms", &tail(&render_ms))
        .spread(
            "gpu_pre_ms",
            &tail(&parts.iter().map(|p| p[0]).collect::<Vec<_>>()),
        )
        .spread(
            "gpu_shadow_ms",
            &tail(&parts.iter().map(|p| p[1]).collect::<Vec<_>>()),
        )
        .spread(
            "gpu_main_ms",
            &tail(&parts.iter().map(|p| p[2]).collect::<Vec<_>>()),
        )
        .spread(
            "gpu_post_ms",
            &tail(&parts.iter().map(|p| p[3]).collect::<Vec<_>>()),
        )
        .spread(
            "visible_meshes",
            &run.visible.iter().map(|v| v.0 as f64).collect::<Vec<_>>(),
        )
        .spread(
            "visible_triangles",
            &run.visible.iter().map(|v| v.1 as f64).collect::<Vec<_>>(),
        )
        .int("meshes", run.meshes as u64)
        .num("fps", n / elapsed)
        .num("cpu_ms_per_frame", (after.cpu_ns - before.cpu_ns) / 1e6 / n)
        .num(
            "instructions_per_frame",
            (after.instructions - before.instructions) as f64 / n,
        )
        .num(
            "cycles_per_frame",
            (after.cycles - before.cycles) as f64 / n,
        )
        .num("load_ms", run.load.map_or(f64::NAN, |l| l.0))
        .num(
            "load_peak_footprint_mb",
            run.load.map_or(f64::NAN, |l| l.1 as f64 / 1048576.),
        )
        .num("footprint_mb", after.footprint as f64 / 1048576.)
        .num(
            "rested_footprint_mb",
            run.rested_footprint as f64 / 1048576.,
        )
        .int("rest_seconds", 10)
        .num("peak_footprint_mb", after.peak_footprint as f64 / 1048576.)
        .int("hash", hash)
        .num("drawn_fraction", drawn)
        .int("rest_ticks", REST_TICKS as u64)
        .int("rest_frames", rest_frames)
        .num("rest_cpu_ms_per_tick", rest_cpu_ms / REST_TICKS as f64)
        .text("errors", "");
    println!("{}", json.line());
    exit.write(AppExit::Success);
}

// The render world.

/// The timed frame this render-world frame draws, as extracted: warm-up
/// frames first, then the measured ones.
#[derive(Resource)]
struct Measuring(Option<usize>);

#[derive(Resource)]
struct Markers {
    queries: wgpu::QuerySet,
    started: Instant,
}

fn extract_phase(run: Extract<Res<Run>>, mut measuring: ResMut<Measuring>) {
    measuring.0 = match run.phase {
        Phase::Warmup(n) => Some(n),
        Phase::Measuring(i) => Some(run.setup.warmup + i),
        _ => None,
    };
}

fn start_render_clock(
    mut commands: Commands,
    device: Res<RenderDevice>,
    shared: Res<Shared>,
    markers: Option<ResMut<Markers>>,
) {
    match markers {
        Some(mut markers) => markers.started = Instant::now(),
        None => {
            use wgpu::{QuerySetDescriptor, QueryType};
            let queries = device.wgpu_device().create_query_set(&QuerySetDescriptor {
                label: Some("Frame timestamps"),
                ty: QueryType::Timestamp,
                count: (shared.frames * SLOTS) as u32,
            });
            commands.insert_resource(Markers {
                queries,
                started: Instant::now(),
            });
        }
    }
}

fn count_compiling(cache: Res<bevy::render::render_resource::PipelineCache>, shared: Res<Shared>) {
    shared
        .compiling
        .store(cache.waiting_pipelines().count(), Ordering::Relaxed);
    shared.rendered.fetch_add(1, Ordering::Relaxed);
}

fn marker(
    device: &RenderDevice,
    images: &RenderAssets<GpuImage>,
    shared: &Shared,
    markers: &Markers,
    index: u32,
    first: bool,
) -> Option<bevy::render::render_resource::CommandEncoder> {
    let target = images.get(&shared.image)?;
    let mut encoder = device
        .wgpu_device()
        .create_command_encoder(&Default::default());
    encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("Timestamp"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: &target.texture_view,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
        })],
        timestamp_writes: Some(RenderPassTimestampWrites {
            query_set: &markers.queries,
            beginning_of_pass_write_index: first.then_some(index),
            end_of_pass_write_index: (!first).then_some(index),
        }),
        ..default()
    });
    Some(encoder)
}

/// A timestamp inside the camera's schedule, slot `S`: an empty pass over
/// the view's main texture, in the camera's own encoder.
fn inner_marker<const S: usize>(
    view: bevy::render::renderer::ViewQuery<&bevy::render::view::ViewTarget>,
    markers: Option<Res<Markers>>,
    measuring: Res<Measuring>,
    mut ctx: bevy::render::renderer::RenderContext,
) {
    let (Some(markers), Some(frame)) = (markers, measuring.0) else {
        return;
    };
    let target = view.into_inner();
    let index = (frame * SLOTS + S) as u32;
    ctx.command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("Timestamp"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target.main_texture_view(),
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            timestamp_writes: Some(RenderPassTimestampWrites {
                query_set: &markers.queries,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(index),
            }),
            ..default()
        });
}

fn begin_marker(
    device: Res<RenderDevice>,
    images: Res<RenderAssets<GpuImage>>,
    shared: Res<Shared>,
    markers: Option<Res<Markers>>,
    measuring: Res<Measuring>,
    mut pending: ResMut<PendingCommandBuffers>,
) {
    let (Some(markers), Some(frame)) = (markers, measuring.0) else {
        return;
    };
    if let Some(encoder) = marker(
        &device,
        &images,
        &shared,
        &markers,
        (frame * SLOTS) as u32,
        true,
    ) {
        pending.push_encoder(encoder);
    }
}

fn end_marker(
    device: Res<RenderDevice>,
    images: Res<RenderAssets<GpuImage>>,
    shared: Res<Shared>,
    markers: Option<Res<Markers>>,
    measuring: Res<Measuring>,
    mut pending: ResMut<PendingCommandBuffers>,
) {
    let (Some(markers), Some(frame)) = (markers, measuring.0) else {
        return;
    };
    if let Some(encoder) = marker(
        &device,
        &images,
        &shared,
        &markers,
        (frame * SLOTS + SLOTS - 1) as u32,
        false,
    ) {
        pending.push_encoder(encoder);
    }
    shared
        .render_ms
        .lock()
        .unwrap()
        .push(markers.started.elapsed().as_secs_f64() * 1e3);
}

fn resolve_timestamps(
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    shared: Res<Shared>,
    markers: Option<Res<Markers>>,
) {
    let Some(markers) = markers else { return };
    if !shared.finish.load(Ordering::Relaxed) || shared.gpu_ms.lock().unwrap().is_some() {
        return;
    }
    use bevy::render::render_resource::{BufferDescriptor, BufferUsages, MapMode, PollType};
    let device = device.wgpu_device();
    let count = (shared.frames * SLOTS) as u32;
    let bytes = count as u64 * 8;
    let resolved = device.create_buffer(&BufferDescriptor {
        label: Some("Resolved timestamps"),
        size: bytes,
        usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&BufferDescriptor {
        label: Some("Timestamps read"),
        size: bytes,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.resolve_query_set(&markers.queries, 0..count, &resolved, 0);
    encoder.copy_buffer_to_buffer(&resolved, 0, &read, 0, bytes);
    queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    slice.map_async(MapMode::Read, |r| r.expect("mapped"));
    device
        .poll(PollType::wait_indefinitely())
        .expect("the device");
    let ticks: Vec<u64> = slice
        .get_mapped_range()
        .chunks_exact(8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let period = queue.get_timestamp_period() as f64;
    let at = |f: usize, slot: usize| ticks[f * SLOTS + slot] as f64 * period / 1e6;
    let gpu_ms = (0..shared.frames)
        .map(|f| (at(f, SLOTS - 1) - at(f, 0)).max(0.))
        .collect();
    *shared.gpu_parts.lock().unwrap() = (0..shared.frames)
        .map(|f| [1, 2, 3, 4].map(|s| (at(f, s) - at(f, s - 1)).max(0.)))
        .collect();
    *shared.gpu_ms.lock().unwrap() = Some(gpu_ms);
}

/// What the frustum kept this frame (CPU visibility; Bevy's GPU preprocessing
/// culls the same set again per view).
fn count_visible(
    mut run: ResMut<Run>,
    meshes: Res<Assets<Mesh>>,
    query: Query<(&Mesh3d, &bevy::camera::visibility::ViewVisibility)>,
    mut triangles: Local<std::collections::HashMap<AssetId<Mesh>, usize>>,
) {
    if !matches!(run.phase, Phase::Measuring(_)) {
        return;
    }
    let (mut count, mut total, mut all) = (0, 0, 0);
    for (mesh, visibility) in &query {
        all += 1;
        if !visibility.get() {
            continue;
        }
        count += 1;
        total += *triangles.entry(mesh.id()).or_insert_with(|| {
            meshes.get(mesh).map_or(0, |m| {
                m.indices().map_or(m.count_vertices(), |i| i.len()) / 3
            })
        });
    }
    run.visible.push((count, total));
    run.meshes = all;
}

/// `--mips=1`: once the model's images have loaded, give each a mip chain
/// made on the CPU as zgx makes its own (each texel the mean of the 2 by 2
/// above it), so every arm samples the same kind of texture.
fn mips(mut run: ResMut<Run>, shared: Res<Shared>, mut images: ResMut<Assets<Image>>) {
    if run.mips_made
        || !matches!(run.phase, Phase::Settling(_))
        || !std::env::args().any(|a| a == "--mips=1")
    {
        return;
    }
    run.mips_made = true;
    let ids: Vec<AssetId<Image>> = images.ids().filter(|id| *id != shared.image.id()).collect();
    let mut made = 0;
    for id in ids {
        let Some(mut image) = images.get_mut(id) else {
            continue;
        };
        let d = &image.texture_descriptor;
        if d.mip_level_count != 1
            || d.size.depth_or_array_layers != 1
            || !matches!(
                d.format,
                TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm
            )
        {
            continue;
        }
        let (mut w, mut h) = (d.size.width as usize, d.size.height as usize);
        let Some(base) = image.data.clone() else {
            continue;
        };
        let mut data = base.clone();
        let mut level = base;
        let mut levels = 1;
        while w > 1 || h > 1 {
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let mut next = vec![0u8; nw * nh * 4];
            for y in 0..nh {
                for x in 0..nw {
                    for c in 0..4 {
                        let mut sum = 0u32;
                        let mut n = 0;
                        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            let (sx, sy) = ((x * 2 + dx).min(w - 1), (y * 2 + dy).min(h - 1));
                            sum += level[(sy * w + sx) * 4 + c] as u32;
                            n += 1;
                        }
                        next[(y * nw + x) * 4 + c] = ((sum + n / 2) / n) as u8;
                    }
                }
            }
            data.extend_from_slice(&next);
            level = next;
            (w, h) = (nw, nh);
            levels += 1;
        }
        image.texture_descriptor.mip_level_count = levels;
        image.data = Some(data);
        made += 1;
    }
    eprintln!("mips made for {made} images");
}
