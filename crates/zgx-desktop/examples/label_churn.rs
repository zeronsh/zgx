//! Labels whose text changes every frame: LABELS texts (30) each new every
//! frame, in the bundled headless font. Measured (laid out as a renderer
//! would measure it, nothing drawn) or, with `DRAWN=1`, laid out and drawn
//! offscreen as a window draws it. `WRAP=1` makes each a paragraph wrapping
//! to several rows. Prints instructions a frame (macOS) and time.
//!   cargo run --release -p zgx-desktop --example label_churn
#[path = "support/headless_font.rs"]
mod headless_font;
use zgx::{compose::prelude::*, widgets::Ui};

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// This process's instructions retired (macOS; 0 elsewhere).
fn instructions() -> u64 {
    #[cfg(target_os = "macos")]
    {
        #[repr(C)]
        struct Usage {
            uuid: [u8; 16],
            fields: [u64; 40],
        }
        unsafe extern "C" {
            fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut Usage) -> i32;
        }
        let mut usage = Usage {
            uuid: [0; 16],
            fields: [0; 40],
        };
        // SAFETY: RUSAGE_INFO_V4 into a buffer as large as it; this process.
        unsafe { proc_pid_rusage(std::process::id() as i32, 4, &mut usage) };
        usage.fields[29]
    }
    #[cfg(not(target_os = "macos"))]
    0
}

fn main() {
    let frames = env("FRAMES", 300);
    let labels = env("LABELS", 30);
    let drawn = env("DRAWN", 0) == 1;
    let wrap = env("WRAP", 0) == 1;
    let (w, h, scale) = (800., 900., 2.);
    let mut ui = Ui::new(w, h);
    let mut gpu = drawn.then(|| {
        let mut gpu =
            zgx_gpu::GpuRenderer::new((w * scale) as u32, (h * scale) as u32).expect("GPU");
        gpu.set_scale_factor(scale);
        gpu.install_text(&mut ui.scene.borrow_mut());
        headless_font::font().install(&mut gpu.text_system().borrow_mut());
        gpu
    });
    let services = (!drawn).then(|| {
        let text = zgx_gpu::TextServices::new();
        headless_font::font().install(&mut text.text_system().borrow_mut());
        text.install(&mut ui.scene.borrow_mut());
        text
    });
    let counter = ui.signal(0_u64);
    let view = column()
        .gap(2.)
        .w(if wrap { 240. } else { w })
        .font_family(zgx_workload::FONT)
        .text_size(14.)
        .children((0..labels).map(|i| {
            let counter = counter.clone();
            text_signal(move || {
                let n = counter.get() * 7 + i as u64;
                if wrap {
                    format!(
                        "Message {i}: the build finished with {n} warnings and the deploy \
                         went out to every region except one, which retries in {} seconds",
                        n % 60
                    )
                } else {
                    format!("Label {i}: {n} items")
                }
            })
        }));
    let _handle = ui.mount(view);
    let mut frame = |ui: &mut Ui, n: usize| {
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        if let Some(gpu) = gpu.as_mut() {
            gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
            // A window presents every frame; reading one back now and then
            // keeps the GPU from falling behind (rarely: a readback's copy
            // would outweigh the labels).
            if n % 600 == 599 {
                gpu.readback().unwrap();
            }
        }
        std::hint::black_box(report.paint_nodes);
    };
    frame(&mut ui, 0);
    frame(&mut ui, 0);
    let (started, before) = (std::time::Instant::now(), instructions());
    for n in 0..frames {
        counter.set(n as u64 + 1);
        frame(&mut ui, n);
    }
    let per = started.elapsed().as_secs_f64() * 1e6 / frames.max(1) as f64;
    let instr = (instructions() - before) as f64 / frames.max(1) as f64;
    println!(
        "{labels} labels{}{}, {frames} frames: {:.3} M instructions a frame ({:.1} K a label), {per:.1} us",
        if drawn { ", drawn" } else { ", measured" },
        if wrap { ", wrapped" } else { "" },
        instr / 1e6,
        instr / 1e3 / labels as f64,
    );
    drop(services);
}
