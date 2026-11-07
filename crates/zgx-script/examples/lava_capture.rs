//! Plays `lava/app.luau` offscreen, frame by frame at 60 Hz, as a hand would
//! use it: a row picked up, moved, dropped into the floor. Writes raw RGBA
//! frames to stdout (or, with `STILLS=frame,frame`, raw frames under `OUT`),
//! with the pointer drawn in as a dot, and prints each frame's time on the CPU
//! and GPU (waiting for it, with a readback) and memory on stderr. No window,
//! no focus.
//! Run: cargo run --release -p zgx-script --example lava_capture > frames.rgba
use std::io::Write;
use std::time::Instant;
use zgx::input::{InputEvent, Modifiers, PointerButton};
use zgx_gpu::GpuRenderer;
use zgx_script::{ScriptHost, testing::TestApp};

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
/// Where the pointer is at `t` seconds, and whether it is down: the
/// reference video's gesture, measured by tracking its card. Row two is
/// pressed at 0.8 s, carried 221 pt down from 1.0 s to 3.6 s along a
/// smoothstep, held still, and let go at 4.28 s; the pointer then leaves.
fn hand(t: f32, width: f32, list_top: f32, row: f32, _drop_y: f32) -> (f32, f32, bool) {
    fn smooth(k: f32) -> f32 {
        let k = k.clamp(0., 1.);
        k * k * (3. - 2. * k)
    }
    let x = width * 0.52;
    let start = list_top + row * 1.5;
    let carried = start + 221.;
    match t {
        t if t < 0.8 => (
            x + 120. * (1. - smooth(t / 0.8)),
            start - 40. * (1. - smooth(t / 0.8)),
            false,
        ),
        t if t < 4.28 => (x, start + 221. * smooth((t - 1.) / 2.6), true),
        t => (
            x + 20. * smooth((t - 4.28) / 1.),
            carried - 110. * smooth((t - 4.28) / 1.2),
            false,
        ),
    }
}
/// The pointer, as a recording of a hand shows it.
fn dot(pixels: &mut [u8], width: u32, height: u32, x: f32, y: f32, radius: f32) {
    let reach = radius as i32 + 2;
    for py in (y as i32 - reach).max(0)..(y as i32 + reach).min(height as i32) {
        for px in (x as i32 - reach).max(0)..(x as i32 + reach).min(width as i32) {
            let distance = ((px as f32 - x).powi(2) + (py as f32 - y).powi(2)).sqrt();
            let cover = (radius + 0.5 - distance).clamp(0., 1.);
            let at = ((py as u32 * width + px as u32) * 4) as usize;
            for channel in &mut pixels[at..at + 3] {
                *channel = (*channel as f32 + (255. - *channel as f32) * cover) as u8;
            }
        }
    }
}
fn main() {
    let seconds: f32 = env("CAP_SECONDS", 9.);
    let scale: f32 = env("SCALE", 2.);
    let logical: (f32, f32) = (env("WIDTH", 900.), env("HEIGHT", 800.));
    let script = std::env::var("SCRIPT")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/examples/lava/app.luau").into());
    let mut app = TestApp::new(|runtime| ScriptHost::new(runtime, script));
    app.ui.scene.borrow_mut().resize(logical.0, logical.1);
    let (width, height) = ((logical.0 * scale) as u32, (logical.1 * scale) as u32);
    let mut gpu = GpuRenderer::new(width, height).expect("a renderer");
    gpu.set_scale_factor(scale);
    // Text measured as a window measures it.
    gpu.install_text(&mut app.ui.scene.borrow_mut());
    let stills: Vec<usize> = env::<String>("STILLS", String::new())
        .split(',')
        .filter_map(|frame| frame.parse().ok())
        .collect();
    let out = env::<String>("OUT", ".".into());
    let raw = stills.is_empty();
    let mut stdout = std::io::stdout().lock();
    let (mut down, mut times) = (false, Vec::new());
    for frame in 0..(seconds * 60.) as usize {
        let t = frame as f32 / 60.;
        let (x, y, pressed) = hand(
            t,
            logical.0,
            env("LIST_TOP", 148.),
            env("ROW", 78.),
            env("DROP_Y", 588.),
        );
        app.ui
            .dispatch_with_modifiers(InputEvent::PointerMove { x, y }, Modifiers::default());
        if pressed != down {
            let button = PointerButton::Primary;
            let event = match pressed {
                true => InputEvent::PointerDown { x, y, button },
                false => InputEvent::PointerUp { x, y, button },
            };
            app.ui.dispatch_with_modifiers(event, Modifiers::default());
            down = pressed;
        }
        app.frame();
        let started = Instant::now();
        {
            let mut scene = app.ui.scene.borrow_mut();
            let report = scene.flush();
            gpu.render(&scene, &report.damage).expect("a frame");
        }
        let pixels = (raw || stills.contains(&frame)).then(|| gpu.readback().expect("its pixels"));
        times.push(started.elapsed().as_secs_f64() * 1e3);
        if let Some(mut pixels) = pixels {
            dot(&mut pixels, width, height, x * scale, y * scale, 7. * scale);
            if raw {
                stdout.write_all(&pixels).expect("stdout");
            } else {
                let path = format!("{out}/f{frame:04}.rgba");
                std::fs::write(path, &pixels).expect("the still");
            }
        }
    }
    if std::env::var_os("TREE").is_some() {
        eprintln!("{}", app.tree());
    }
    times.sort_by(f64::total_cmp);
    let rss = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|out| {
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0);
    eprintln!(
        "{width}x{height} frame median {:.2} ms, p95 {:.2} ms, resident {} MB",
        times[times.len() / 2],
        times[times.len() * 95 / 100],
        rss / 1024
    );
}
