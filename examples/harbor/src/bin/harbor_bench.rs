//! The harbor drawn offscreen along a fixed camera path: what each
//! milestone of the 3D work is judged with. It draws the app as it runs,
//! the HUD over the world, through zgx's public offscreen renderer
//! (`zgx_gpu::GpuRenderer`), and proves each run drew.
//!
//! Run: cargo run --release --bin harbor_bench (from examples/harbor)
//!
//! Settings (environment):
//! - `FRAMES` (600) measured, spread over the whole fly-through; `WARM`
//!   (30) before them, at its start.
//! - `WIDTH`, `HEIGHT` (1920, 1080) device pixels; `SCALE` (2) device
//!   pixels a point, as a Retina window.
//! - `BLOCKS` (`5x7`) the yard's blocks; `SKY=0` lights it by a hemisphere
//!   instead of the dawn sky's environment.
//! - `SHADOWS=0` turns the sun's shadows off, `CASCADES=0` its cascades,
//!   `LAMPS=0` the lamps;
//!   `SUN=degrees` sets the sun's elevation, its light's and the sky's;
//!   `RAIN=0` makes no rain;
//!   `LAMP_REACH` scales every lamp's range (0: none built); `EXPOSURE`
//!   adds stops (as the panel's slider).
//! - `HASHES=file` writes each frame's hash, a line each.
//! - `PNG=path` writes the last frame; `PNGS=dir` writes six frames along
//!   the path, for looks to be judged (`PNG_AT=i,j` those frames instead).
//!
//! Each frame is waited for by reading it back (the renderer has no other
//! public wait: GAPS.md, G5) and hashed; the readback's own cost, measured
//! apart at the end, is printed beside the frame times. Built with
//! `--features gpu-time` against the perf lane's patched wgpu-hal
//! (README.md), it also prints GPU busy time a frame, the readbacks'
//! copies included.
use std::time::Instant;
use zgx::{compose::prelude::*, widgets::Ui};
use zgx_3d::World;
use zgx_gpu::{GpuRenderer, Readback};
use zgx_harbor::{
    hud::{self, Stats},
    path, perf,
    scene::{Config, Harbor},
    settings::Settings,
    sky,
};

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Command buffers' GPU intervals since last asked (Metal, patched hal).
fn gpu_intervals() -> Option<Vec<(f64, f64)>> {
    #[cfg(all(target_os = "macos", feature = "gpu-time"))]
    return Some(zgx_gpu::wgpu::hal::metal::gpu_time::take_intervals());
    #[cfg(not(all(target_os = "macos", feature = "gpu-time")))]
    None
}

/// The union of `intervals`' lengths: each moment the GPU worked, once.
fn busy(mut intervals: Vec<(f64, f64)>) -> f64 {
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut total, mut current) = (0., None::<(f64, f64)>);
    for (start, end) in intervals {
        current = match current {
            Some((s, e)) if start <= e => Some((s, e.max(end))),
            Some((s, e)) => {
                total += e - s;
                Some((start, end))
            }
            None => Some((start, end)),
        };
    }
    total + current.map_or(0., |(s, e)| e - s)
}

fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("the PNG's file");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("a PNG header");
    writer.write_image_data(rgba).expect("the PNG's pixels");
}

fn main() {
    let frames: usize = env("FRAMES", 600).max(1);
    let warm: usize = env("WARM", 30);
    let (width, height): (u32, u32) = (env("WIDTH", 1920), env("HEIGHT", 1080));
    let scale: f32 = env("SCALE", 2.);
    let blocks = std::env::var("BLOCKS")
        .ok()
        .and_then(|b| {
            let (x, z) = b.split_once('x')?;
            Some((x.parse().ok()?, z.parse().ok()?))
        })
        .unwrap_or(Config::default().blocks);
    let defaults = Config::default();
    let config = Config {
        blocks,
        // `SUN=degrees`: the sun's elevation, its light and the sky's both.
        sun: (env("SUN", defaults.sun.0), defaults.sun.1),
        lamp_reach: env("LAMP_REACH", 1.),
        // `RAIN=0`: no rain made at all.
        rain: std::env::var("RAIN").as_deref() != Ok("0"),
        ..Config::default()
    };

    // The interface's time, and the world's: moved a fixed step a frame,
    // so what moves by itself (rain) is where the path's time says, not
    // where the wall clock does.
    let clock = zgx::clock::VirtualClock::new();
    let started = Instant::now();
    let environment = (std::env::var("SKY").as_deref() != Ok("0"))
        .then(|| sky::dawn(sky::sun_toward(config.sun.0, config.sun.1), 512));
    let sky_ms = started.elapsed().as_secs_f64() * 1e3;
    let world = World::new();
    let harbor = Harbor::build(&world, &config, environment);
    let build_ms = started.elapsed().as_secs_f64() * 1e3 - sky_ms;

    let (points_w, points_h) = (width as f32 / scale, height as f32 / scale);
    let mut ui = Ui::new(points_w, points_h);
    let settings = Settings::new(&ui.runtime, &harbor);
    let _bound = settings.bind(&ui.runtime, &world, &harbor);
    // `SHADOWS=0`: the sun casts none (an attribution arm).
    settings
        .shadows
        .set(std::env::var("SHADOWS").as_deref() != Ok("0"));
    // `CASCADES=0`: one shadow map over everything, as before cascades.
    settings
        .cascades
        .set(std::env::var("CASCADES").as_deref() != Ok("0"));
    // `EXPOSURE=stops`: the panel's exposure, over the harbor's own.
    settings.exposure.set(env("EXPOSURE", 0.));
    // `LAMPS=0`: every lamp dark (an attribution arm: the lights' cost).
    settings
        .lamps
        .set(std::env::var("LAMPS").as_deref() != Ok("0"));
    let stats = ui.runtime.signal(Stats {
        objects: harbor.objects,
        triangles: harbor.triangles,
        containers: harbor.yard.len() + harbor.deck.len(),
        lights: (harbor.lamps.len(), harbor.lamps.len()),
        ..Stats::default()
    });
    ui.mount(div().size(points_w, points_h).child(hud::view(
        gpu_view("The harbor", world.view()),
        &settings,
        stats.clone(),
    )));
    let mut gpu = GpuRenderer::new(width, height).expect("a GPU");
    gpu.set_scale_factor(scale);
    // The renderer's text services on the scene: layout measures text as
    // the renderer draws it. Without them it measures by an estimate, and
    // text sized by its own width is clipped (GAPS.md, B1).
    gpu.install_text(&mut ui.scene.borrow_mut());
    ui.refresh_text_geometry();

    let step = path::LOOP / frames as f32;
    let base = world.camera();
    let mut pixels = Vec::new();
    let mut staging = Readback::default();
    let shots: Option<std::path::PathBuf> = std::env::var_os("PNGS").map(Into::into);
    if let Some(dir) = &shots {
        std::fs::create_dir_all(dir).expect("the PNGs' folder");
    }
    let shot_every = (frames / 6).max(1);
    // `PNG_AT=i,j,...`: those frames, in place of six along the path.
    let at: Vec<usize> = std::env::var("PNG_AT")
        .map(|v| v.split(',').filter_map(|i| i.trim().parse().ok()).collect())
        .unwrap_or_default();

    // One frame: the camera where the path is at `seconds`, the HUD's
    // readout, a render, and a wait for it by reading it back.
    let mut draw = |seconds: f32, pixels: &mut Vec<u8>| -> (f64, f64, bool) {
        let begin = Instant::now();
        clock.advance(std::time::Duration::from_secs_f32(step), || {});
        let before = world.revision();
        let camera = path::camera(seconds, base);
        world.set_camera(camera);
        if let Some(rain) = harbor.rain {
            rain.follow(&world, camera.position);
        }
        stats.update(|s| {
            s.time = seconds;
            // The path's own step: the bench's frames are not timed by a
            // display, so no rate is shown.
            s.frame_ms = step * 1e3;
            s.revision = world.revision();
            // Shaded: all but those the last frame left out.
            s.lights.1 = s.lights.0.saturating_sub(world.lights_left_out() as usize);
        });
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage)
            .expect("a frame");
        let rendered = begin.elapsed().as_secs_f64() * 1e3;
        gpu.readback_into(pixels, &mut staging).expect("its pixels");
        let total = begin.elapsed().as_secs_f64() * 1e3;
        (rendered, total, world.revision() != before)
    };

    for k in 0..warm {
        draw(k as f32 * 1e-3, &mut pixels);
    }
    let _ = gpu_intervals();
    let before = perf::sample();
    let (mut totals, mut renders) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
    let (mut changed, mut hash) = (0usize, 0u64);
    let mut hashes = Vec::with_capacity(frames);
    let mut pngs = perf::Sample::default();
    let measured = Instant::now();
    for i in 0..frames {
        let seconds = i as f32 * step;
        let (rendered, total, moved) = draw(seconds, &mut pixels);
        renders.push(rendered);
        totals.push(total);
        changed += usize::from(moved);
        // Every 61st pixel: proof each frame differs, at a sliver of a
        // full hash's cost (the full one is the last frame's, below).
        let sample: Vec<u8> = pixels.chunks(4).step_by(61).flatten().copied().collect();
        let frame_hash = perf::hash(&sample);
        hash = frame_hash ^ hash.rotate_left(5);
        hashes.push(frame_hash);
        if let Some(dir) = &shots
            && (if at.is_empty() {
                i % shot_every == 0
            } else {
                at.contains(&i)
            })
        {
            // Not the frame's cost: counted apart, and taken off below.
            let before = perf::sample();
            write_png(
                &dir.join(format!("harbor-{:05.2}s.png", seconds)),
                width,
                height,
                &pixels,
            );
            let after = perf::sample();
            pngs.instructions += after.instructions - before.instructions;
            pngs.cpu_ns += after.cpu_ns - before.cpu_ns;
        }
    }
    let wall = measured.elapsed().as_secs_f64();
    let after = perf::sample();
    let gpu_busy = gpu_intervals().map(busy);

    // The readback alone: frames with nothing changed (the renderer draws
    // nothing new), read back as each measured frame was.
    let mut readback = Vec::new();
    for _ in 0..30 {
        let begin = Instant::now();
        gpu.readback_into(&mut pixels, &mut staging)
            .expect("its pixels");
        readback.push(begin.elapsed().as_secs_f64() * 1e3);
    }

    // Proof it drew: the last frame against the colours of a frame with
    // nothing drawn (the world's background and the box's).
    let clears = [rgb_of(sky::BACKGROUND), rgb_of(0x0b0e13)];
    let unlike = pixels
        .chunks(4)
        .filter(|p| {
            clears
                .iter()
                .all(|c| (0..3).any(|i| (p[i] as i32 - c[i] as i32).abs() > 6))
        })
        .count() as f64
        / (pixels.len() / 4).max(1) as f64;
    let last_hash = perf::hash(&pixels);
    // `HASHES=file`: each frame's hash a line, to find the frames two runs
    // disagree on.
    if let Ok(path) = std::env::var("HASHES") {
        let lines: String = hashes.iter().map(|h| format!("{h:016x}\n")).collect();
        std::fs::write(path, lines).expect("the hashes' file");
    }
    if let Ok(path) = std::env::var("PNG") {
        write_png(std::path::Path::new(&path), width, height, &pixels);
    }

    let n = frames as f64;
    println!(
        "harbor_bench {width}x{height} @{scale}x, {frames} frames over {:.0} s of path (+{warm} warm)",
        path::LOOP
    );
    println!(
        "scene: {} objects, {:.2} M triangles, {} containers, {} lights ({} left out of the last frame); sky {:.0} ms, build {:.0} ms",
        harbor.objects,
        harbor.triangles as f64 / 1e6,
        harbor.yard.len() + harbor.deck.len(),
        harbor.lamps.len(),
        world.lights_left_out(),
        sky_ms,
        build_ms,
    );
    println!(
        "frames drawn: {frames} rendered, {changed} with the world changed (World::revision; World::frames_drawn is hidden: GAPS.md G1)"
    );
    if perf::AVAILABLE {
        println!(
            "instructions/frame: {:.2} M; CPU ms/frame: {:.2}; footprint: {:.0} MB",
            (after.instructions - before.instructions - pngs.instructions) as f64 / n / 1e6,
            (after.cpu_ns - before.cpu_ns - pngs.cpu_ns) / n / 1e6,
            after.footprint as f64 / 1e6,
        );
    } else {
        println!("instructions/frame: not available on this platform");
    }
    let p = |v: &[f64], q| perf::percentile(v, q);
    println!(
        "frame ms (render + wait): p50 {:.2} p90 {:.2} p99 {:.2} max {:.2}; {:.1} frames/s",
        p(&totals, 50.),
        p(&totals, 90.),
        p(&totals, 99.),
        p(&totals, 100.),
        n / wall,
    );
    println!(
        "render call ms (CPU, until submitted): p50 {:.2} p90 {:.2} p99 {:.2}",
        p(&renders, 50.),
        p(&renders, 90.),
        p(&renders, 99.),
    );
    println!(
        "readback alone ms (in each frame above): p50 {:.2}",
        p(&readback, 50.)
    );
    if let Some(rain) = harbor.rain {
        println!(
            "rain: drops {:?}, splashes {:?}",
            world.emitter_status(rain.drops),
            world.emitter_status(rain.splashes),
        );
    }
    match gpu_busy {
        Some(seconds) => println!(
            "GPU busy ms/frame: {:.3} (readback copies included)",
            seconds * 1e3 / n
        ),
        None => println!("GPU busy ms/frame: not built (--features gpu-time, README.md)"),
    }
    println!(
        "proof: last frame {:016x}, {:.1}% unlike the clear colours; frames' hash {:016x}",
        last_hash,
        unlike * 100.,
        hash,
    );
    if unlike < 0.25 {
        eprintln!("harbor_bench: under a quarter of the last frame was drawn");
        std::process::exit(1);
    }
}

fn rgb_of(hex: u32) -> [u8; 3] {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}
