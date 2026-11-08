//! A terminal streaming a build log, headless and undrawn: ten thousand
//! lines of output (16, 256 and true colours, bold, italic and underlined
//! runs, lines wrapping past the edge, a progress line rewritten in place)
//! fed in reads of 4 KB, each parsed into the screen, its damaged rows
//! worked out and their canvases laid out and measured as a renderer
//! would, a frame a read. The session replays the output on this thread
//! (`zgx_terminal::replay_terminal`): a pty and its reading thread would
//! hand it over as they please, and the count would move with them. CI's
//! instruction gate counts it, so what a terminal's output costs is
//! guarded.
//! Run: cargo run -p zgx-terminal --release --example terminal_bench
//! (`LINES` sets the lines, 10,000 by default.)
use std::fmt::Write;
use zgx::{
    compose::prelude::*,
    terminal::{Terminal, TerminalOptions, TerminalRenderer, terminal},
    text_layout::FontFamily,
    theme::{ThemeOverride, tok},
    widgets::Ui,
};

/// The log: deterministic, no two neighbouring lines alike.
fn output(lines: usize) -> Vec<u8> {
    let mut out = String::new();
    let words = [
        "Compiling",
        "Checking",
        "Finished",
        "warning:",
        "error[E0308]:",
        "note:",
        "-->",
        "src/lib.rs",
        "unused",
        "variable",
        "mismatched",
        "types",
        "expected",
        "found",
        "in",
        "crate",
        "zgx",
        "terminal",
        "release",
        "profile",
    ];
    for i in 0..lines {
        let mut line = String::new();
        // A coloured status, as cargo writes one.
        match i % 5 {
            0 => line.push_str("\x1b[1;32m   Compiling\x1b[0m "),
            1 => line.push_str("\x1b[1;33mwarning\x1b[0m\x1b[1m: "),
            2 => line.push_str("\x1b[38;5;75m  -->\x1b[0m "),
            3 => {
                let _ = write!(line, "\x1b[38;2;{};{};200m", i % 256, (i * 7) % 256);
            }
            _ => line.push_str("\x1b[3;4m"),
        }
        // Words, every seventh line long enough to wrap twice.
        let count = if i % 7 == 0 { 40 } else { 4 + i % 9 };
        for w in 0..count {
            line.push_str(words[(i * 3 + w * 5) % words.len()]);
            line.push(' ');
            if w % 6 == 5 {
                let _ = write!(line, "\x1b[3{}m", (i + w) % 8);
            }
        }
        line.push_str("\x1b[0m\r\n");
        // A progress line rewritten in place, as a build's bar is.
        if i % 50 == 49 {
            for step in 0..4 {
                let _ = write!(
                    line,
                    "\r\x1b[K\x1b[36m[{:<20}]\x1b[0m {}/{}",
                    "=".repeat(step * 5),
                    i,
                    lines
                );
            }
            line.push_str("\r\x1b[K");
        }
        out.push_str(&line);
    }
    out.into_bytes()
}

fn main() {
    let lines: usize = std::env::var("LINES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);
    let mut ui = Ui::new(800., 600.);
    let text = zgx_gpu::TextServices::new();
    let font = zgx_gpu::text::FontData::new(std::sync::Arc::<[u8]>::from(zgx_workload::FONT_BYTES))
        .expect("the bundled font");
    font.install(&mut text.text_system().borrow_mut());
    text.install(&mut ui.scene.borrow_mut());
    let term = Terminal::new(&ui.runtime, TerminalOptions::default());
    let replay = zgx_terminal::replay_terminal(&term, ui.runtime.clone());
    // The grid in the bundled font: the machine's monospaced font would
    // make the count the machine's.
    ui.mount(provide(
        TerminalRenderer::new(zgx_terminal::terminal_view),
        theme_scope_with(
            || ThemeOverride::new().with(tok::FONT_MONO, FontFamily::from(zgx_workload::FONT)),
            terminal(&term).w(800.).h(600.),
        ),
    ));
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let output = output(lines);
    let started = std::time::Instant::now();
    let mut frames = 0;
    for read in output.chunks(4096) {
        replay.feed(read);
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        frames += 1;
    }
    let state = term.state().get();
    println!(
        "{lines} lines, {} KB in {frames} reads on {}x{} cells: {:.3} ms a read",
        output.len() / 1024,
        state.columns,
        state.rows,
        started.elapsed().as_secs_f64() * 1e3 / frames as f64,
    );
}
