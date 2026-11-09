//! A terminal showing pictures, through the whole renderer on wgpu's no-op
//! backend (`ZGX_GPU_NOOP`): pictures placed at the cursor (RGBA in chunks,
//! each fourth a PNG), scrolled up with lines of text, sent again under their
//! ids to replace what they show and moved where placed again; sixel images
//! placed and replaced in place; pictures deleted. Each step is a frame:
//! parsed, decoded, laid out and drawn. As `terminal_bench`, the output
//! replays on this thread (`zgx_terminal::replay_terminal`), so the count
//! moves with the code alone. CI's instruction gate counts it, so what
//! showing a picture costs is guarded, beside `terminal`, what showing none
//! does.
//! Run: ZGX_GPU_NOOP=1 cargo run -p zgx-terminal --release --example terminal_pictures_bench
//! (`ROUNDS` sets the rounds, 60 by default.)
use base64::Engine;
use std::{fmt::Write, sync::Arc};
use zgx::{
    compose::prelude::*,
    scene::NodeKind,
    terminal::{Terminal, TerminalOptions, TerminalRenderer, terminal},
    text_layout::FontFamily,
    theme::{ThemeOverride, tok},
    widgets::Ui,
};
use zgx_gpu::GpuRenderer;

const SIDE: u32 = 64;

/// A picture's RGBA pixels, different for each `seed`.
fn pixels(seed: usize) -> Vec<u8> {
    let side = SIDE as usize;
    let mut pixels = Vec::with_capacity(side * side * 4);
    for y in 0..side {
        for x in 0..side {
            pixels.extend([(x * 4 + seed) as u8, (y * 4) as u8, seed as u8, 255]);
        }
    }
    pixels
}

fn png(pixels: &[u8]) -> Vec<u8> {
    let mut file = Vec::new();
    let mut encoder = png::Encoder::new(&mut file, SIDE, SIDE);
    encoder.set_color(png::ColorType::Rgba);
    let mut writer = encoder.write_header().expect("a header");
    writer.write_image_data(pixels).expect("the pixels");
    writer.finish().expect("the end");
    file
}

/// A graphics command with `data` in chunks of 4096 characters, as programs
/// send them.
fn kitty(out: &mut Vec<u8>, keys: &str, data: &[u8]) {
    if data.is_empty() {
        out.extend_from_slice(format!("\x1b_G{keys}\x1b\\").as_bytes());
        return;
    }
    let text = base64::engine::general_purpose::STANDARD.encode(data);
    let chunks: Vec<&[u8]> = text.as_bytes().chunks(4096).collect();
    for (index, chunk) in chunks.iter().enumerate() {
        let more = u8::from(index + 1 < chunks.len());
        let keys = if index == 0 { keys } else { "" };
        out.extend_from_slice(format!("\x1b_G{keys},m={more};").as_bytes());
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
}

/// A sixel image of two bands of colour, 48 pixels wide.
fn sixel(out: &mut Vec<u8>, seed: usize) {
    let (red, green) = (seed % 100, (seed * 7) % 100);
    let mut image = format!("\x1bP0;0q\"1;1;48;36#1;2;{red};{green};50#2;2;{green};{red};20");
    for band in 0..6 {
        let _ = write!(image, "#{}!48~-", 1 + band % 2);
    }
    image.push_str("\x1b\\");
    out.extend_from_slice(image.as_bytes());
}

/// `lines` lines of coloured text, scrolling what is above them.
fn text(out: &mut Vec<u8>, seed: usize, lines: usize) {
    for line in 0..lines {
        let color = 31 + (seed + line) % 6;
        out.extend_from_slice(
            format!("\x1b[{color}mline {seed} {line}\x1b[0m of the output around them\r\n")
                .as_bytes(),
        );
    }
}

/// The output, a frame's worth to each entry.
fn steps(rounds: usize) -> Vec<Vec<u8>> {
    let mut steps = Vec::new();
    for round in 0..rounds {
        let id = 1 + round % 4;
        // Placed over 8 by 4 cells at the cursor, moving this picture's
        // placement from where it was.
        let mut step = Vec::new();
        let first = pixels(round);
        if round % 4 == 3 {
            let keys = format!("a=T,f=100,c=8,r=4,i={id},p=1,q=2");
            kitty(&mut step, &keys, &png(&first));
        } else {
            let keys = format!("a=T,f=32,s={SIDE},v={SIDE},c=8,r=4,i={id},p=1,q=2");
            kitty(&mut step, &keys, &first);
        }
        step.extend_from_slice(b"\r\n");
        steps.push(step);
        // Scrolled with the lines below it.
        for lines in 0..4 {
            let mut step = Vec::new();
            text(&mut step, round * 10 + lines, 3);
            steps.push(step);
        }
        // Replaced, where it stands.
        let mut step = Vec::new();
        let keys = format!("a=t,f=32,s={SIDE},v={SIDE},i={id},q=2");
        kitty(&mut step, &keys, &pixels(round + 1000));
        steps.push(step);
        let mut step = Vec::new();
        text(&mut step, round * 10 + 5, 2);
        steps.push(step);
        // A sixel image, then another over it, taking its cells.
        if round % 2 == 1 {
            let mut step = Vec::new();
            sixel(&mut step, round);
            steps.push(step);
            let mut step = Vec::new();
            sixel(&mut step, round + 1);
            step.extend_from_slice(b"\r\n\r\n");
            steps.push(step);
        }
        // Deleted, its picture with it.
        if round % 8 == 7 {
            let mut step = Vec::new();
            kitty(&mut step, &format!("a=d,d=I,i={id},q=2"), &[]);
            steps.push(step);
        }
    }
    steps
}

fn main() {
    let rounds: usize = std::env::var("ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let scale = 2.;
    let mut ui = Ui::new(800., 600.);
    let mut gpu = GpuRenderer::new(1600, 1200).expect("a renderer");
    gpu.set_scale_factor(scale);
    let font = zgx_gpu::text::FontData::new(Arc::<[u8]>::from(zgx_workload::FONT_BYTES))
        .expect("the bundled font");
    font.install(&mut gpu.text_system().borrow_mut());
    gpu.install_text(&mut ui.scene.borrow_mut());
    let term = Terminal::new(&ui.runtime, TerminalOptions::default());
    let replay = zgx_terminal::replay_terminal(&term, ui.runtime.clone());
    ui.mount(provide(
        TerminalRenderer::new(zgx_terminal::terminal_view),
        theme_scope_with(
            || ThemeOverride::new().with(tok::FONT_MONO, FontFamily::from(zgx_workload::FONT)),
            terminal(&term).w(800.).h(600.),
        ),
    ));
    ui.prepare_frame();
    let report = ui.scene.borrow_mut().flush();
    gpu.render(&ui.scene.borrow(), &report.damage)
        .expect("a frame");
    let steps = steps(rounds);
    let started = std::time::Instant::now();
    let mut drawn = 0;
    for step in &steps {
        for read in step.chunks(4096) {
            replay.feed(read);
        }
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        let scene = ui.scene.borrow();
        gpu.render(&scene, &report.damage).expect("a frame");
        drawn += scene
            .paint_items()
            .filter(|item| matches!(item.kind, NodeKind::Image(_)))
            .count();
    }
    // A cell measuring nothing would read as flat: every frame but a few
    // shows a picture.
    assert!(
        drawn >= steps.len(),
        "{drawn} pictures drawn in {} frames",
        steps.len()
    );
    println!(
        "{rounds} rounds, {} frames, {drawn} pictures drawn: {:.3} ms a frame",
        steps.len(),
        started.elapsed().as_secs_f64() * 1e3 / steps.len() as f64,
    );
}
