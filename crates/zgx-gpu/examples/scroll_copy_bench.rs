//! How a chat-like transcript scrolls: paragraphs, code blocks and tables each
//! a sideways scroll fading at its edges, messages folded under a fade at their
//! foot, in a view fading at its own top and bottom. Prints the share of frames
//! that moved the view's pixels, what each frame drew, and how long a frame
//! took from render to its pixels read back (the GPU done; the same readback
//! in every frame, so compare, don't read it as a frame's cost).
//!
//! Run: cargo run --release -p zgx-gpu --example scroll_copy_bench [frames] [rounds]
use std::time::Instant;
use zgx::{compose::prelude::*, reactive::Signal, scene::Rect, widgets::Ui};
use zgx_gpu::GpuRenderer;

const WIDTH: f32 = 1114.;
const HEIGHT: f32 = 685.;
const SCALE: f32 = 2.;
const STEP: f32 = 3.;

fn line(words: &str, size: f32, color: u32) -> View {
    text(words).w_full().text_size(size).text_color(rgb(color))
}

fn paragraph(i: usize) -> View {
    line(
        &format!(
            "Paragraph {i}: a reply wraps across the view, line after line, the way an \
             answer does when it explains what it changed, why it changed it, and what \
             it measured afterwards, in words nobody needs to read twice."
        ),
        15.,
        0xd8dce4,
    )
}

/// A code block: wider than the view, fading at its right edge while it overflows.
fn code(offset: Signal<f32>, i: usize) -> View {
    let mut lines = column().w(1600.).gap(2.).p(12.);
    for n in 0..8 {
        lines = lines.child(
            text(format!(
                "let value_{n} = compute_something_rather_long(input_{i}, {n}).unwrap_or_default();"
            ))
            .w(1560.)
            .h(18.)
            .text_size(13.)
            .text_color(rgb(0xa8d0a0)),
        );
    }
    scroll_x(offset)
        .w_full()
        .h(200.)
        .rounded(8.)
        .bg(rgb(0x20242c))
        .fade_edges_x(32., 32.)
        .fade_overflow(true)
        .child(lines)
}

/// A table: its rows' cells under the table's fade, a long cell fading too.
fn table(offset: Signal<f32>) -> View {
    let mut rows = column().w(1400.);
    for r in 0..6 {
        let mut cells = row().w(1400.).h(30.).bg(if r % 2 == 0 {
            rgb(0x1c2028)
        } else {
            rgb(0x22262e)
        });
        for c in 0..7 {
            cells = cells.child(
                div()
                    .w(200.)
                    .h(30.)
                    .overflow_hidden()
                    .fade_edges_x(0., 16.)
                    .child(
                        text(format!("cell {r}:{c} holding more words than fit"))
                            .w(360.)
                            .h(30.)
                            .text_size(13.)
                            .text_color(rgb(0xc8ccd4)),
                    ),
            );
        }
        rows = rows.child(cells);
    }
    scroll_x(offset)
        .w_full()
        .h(180.)
        .rounded(8.)
        .fade_edges_x(32., 32.)
        .fade_overflow(true)
        .child(rows)
}

/// A message folded shut: its clip fades at its foot.
fn folded(i: usize) -> View {
    let mut words = column().w_full().gap(4.);
    for n in 0..10 {
        words = words.child(line(
            &format!("Folded message {i}, line {n}: pasted text the reader opened once."),
            14.,
            0xe8e8f0,
        ));
    }
    div()
        .w_full()
        .h(120.)
        .p(10.)
        .rounded(10.)
        .bg(rgb(0x2c3440))
        .overflow_hidden()
        .fade_edges(0., 36.)
        .child(words)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let frames: usize = args.next().map_or(600, |v| v.parse().expect("frames"));
    let rounds: usize = args.next().map_or(3, |v| v.parse().expect("rounds"));
    let (w, h) = ((WIDTH * SCALE) as u32, (HEIGHT * SCALE) as u32);
    let mut gpu = GpuRenderer::new(w, h)?;
    gpu.set_scale_factor(SCALE);
    gpu.debug_enable_single_pass();
    gpu.debug_allow_scroll_copies();
    let mut ui = Ui::new(WIDTH, HEIGHT);
    ui.scene.borrow_mut().set_pixel_scale(SCALE);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let offset = ui.signal(0_f32);
    let mut page = column().w_full().gap(18.).p(24.);
    for i in 0..60 {
        page = page
            .child(folded(i))
            .child(paragraph(i))
            .child(code(ui.signal(0_f32), i))
            .child(paragraph(i + 1000))
            .child(table(ui.signal(0_f32)));
    }
    ui.mount(
        overlay().w(WIDTH).h(HEIGHT).bg(rgb(0x000000)).child(
            scroll(offset.clone())
                .w(WIDTH)
                .h(HEIGHT)
                .bg(rgb(0x14181f))
                .fade_edges(24., 24.)
                .child(page),
        ),
    );
    let render = |ui: &mut Ui, gpu: &mut GpuRenderer| -> Result<_, Box<dyn std::error::Error>> {
        ui.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        let at = Instant::now();
        let stats = gpu.render(&ui.scene.borrow(), &damage)?;
        let pixels = gpu.debug_present_offscreen()?;
        std::hint::black_box(pixels);
        Ok((at.elapsed().as_secs_f64() * 1e3, stats, damage))
    };
    // Warm: shaders, glyphs, layers, once over what the rounds show.
    for frame in 0..=frames.min(120) {
        offset.set(frame as f32 * STEP);
        render(&mut ui, &mut gpu)?;
    }
    offset.set(0.);
    render(&mut ui, &mut gpu)?;
    render(&mut ui, &mut gpu)?;
    let mut times = Vec::new();
    let (mut quads, mut draws, mut copied, mut area) = (0, 0, 0, 0.);
    for round in 0..rounds {
        for frame in 0..frames {
            let at = frame as f32 * STEP;
            offset.set(if round % 2 == 0 {
                at
            } else {
                (frames as f32 * STEP) - at
            });
            let (time, stats, damage) = render(&mut ui, &mut gpu)?;
            times.push(time);
            quads += stats.instances;
            draws += stats.draw_calls;
            copied += usize::from(stats.scroll_copies > 0);
            let view = Rect::new(0., 0., WIDTH, HEIGHT);
            area += damage
                .iter()
                .filter_map(|d| d.intersection(view))
                .map(|d| d.width * d.height)
                .sum::<f32>()
                / (WIDTH * HEIGHT);
        }
    }
    let total = (frames * rounds) as f64;
    times.sort_by(f64::total_cmp);
    println!(
        "copied {:5.1}% of frames  {:7.1} quads {:5.1} draws a frame  flush damage {:5.1}%  \
         render to read back: median {:.2} ms, p90 {:.2} ms, p99 {:.2} ms",
        copied as f64 / total * 100.,
        quads as f64 / total,
        draws as f64 / total,
        area as f64 / total * 100.,
        times[times.len() / 2],
        times[times.len() * 9 / 10],
        times[times.len() * 99 / 100],
    );
    Ok(())
}
