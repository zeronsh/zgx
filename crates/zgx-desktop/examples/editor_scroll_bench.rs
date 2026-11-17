//! A long text area scrolled a little a frame, as a wheel or a trackpad does:
//! 2,000 lines of code, unwrapped, 12 points a step down and back, each step
//! laid out and flushed with its text shaped as a renderer would and nothing
//! drawn (`TextServices`). What scrolling an editor costs a frame: the lines
//! it shows, those coming into view shaped, and the damage it makes.
//!
//!   cargo run --release -p zgx-desktop --example editor_scroll_bench
//!
//! `FRAMES` sets the scrolled frames (300); `MOVED=1` prints the byte under
//! the view's middle each frame, to see it scroll.
#[path = "support/headless_font.rs"]
mod headless_font;
use zgx::{compose::prelude::*, input::InputEvent, widgets::Ui};
use zgx_gpu::TextServices;

const LINES: usize = 2000;

fn main() {
    let env = |k: &str, d: usize| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };
    let (frames, moved) = (env("FRAMES", 300), env("MOVED", 0) == 1);
    let text = (0..LINES)
        .map(|i| {
            format!(
                "    let value_{i:04} = compute(\"{}\", {i}) + other_call(alpha, beta, gamma);",
                "x".repeat(60)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut ui = Ui::new(900., 700.);
    let services = TextServices::new();
    headless_font::font().install(&mut services.text_system().borrow_mut());
    services.install(&mut ui.scene.borrow_mut());
    let value = ui.signal(text);
    let view = ui.mount(
        column().w_full().h_full().p(16.).child(
            text_area("Code", value)
                .id("code")
                .w(860.)
                .h(660.)
                .text_size(14.)
                .line_height(20.)
                .text_wrap(false),
        ),
    );
    let code = view.find("code").expect("the editor");
    // Settle: the first lines shaped.
    for _ in 0..10 {
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
    }
    let (x, y) = {
        let r = ui.scene.borrow().bounds(code);
        (r.x + r.width / 2., r.y + r.height / 2.)
    };
    // Down 120 steps and back up, a wheel's 12 points each.
    let editor = ui.editor(code).expect("its editor");
    // The byte under the view's middle: a line on, every 20 points.
    let report = || editor.offset_at(x, y);
    let mut last = report();
    for n in 0..frames {
        let delta_y = if (n / 120) % 2 == 0 { 12. } else { -12. };
        ui.dispatch(InputEvent::Scroll {
            x,
            y,
            delta_x: 0.,
            delta_y,
        });
        ui.prepare_frame();
        let flushed = ui.scene.borrow_mut().flush();
        if zgx_fingerprint::on() {
            zgx_fingerprint::frame(&ui.scene.borrow(), &flushed);
        }
        if moved {
            let now = report();
            eprintln!("frame {n}: {last:?} -> {now:?}");
            last = now;
        }
    }
    println!("{frames} frames, {} scene nodes", ui.scene.borrow().len());
}
