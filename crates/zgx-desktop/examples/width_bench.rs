//! A chat page whose width changes as a side panel's edge is dragged, one
//! width a frame: what Zeron's interface lays out while a person drags its
//! panel open, offscreen. The page nests as the app's does: a shell column
//! under a band its insets place (a title bar), a row growing into the rest
//! of it, a screen column and a row taking the rest of that beside the
//! sidebar, the page beside the panel, a layer inset under the title bar
//! (`absolute`, in a `relative` page), transitions' one-cell grids
//! (`grid_cols(1)`) and wrappers filling their parents (`w_full`, `h_full`),
//! `WIDTH_DEPTH` (6) levels of them, down to a scrolled conversation:
//! `WIDTH_ROWS` (8) messages of wrapping paragraphs, a list and a code
//! block, each kept to a centred 660-point column. A composer floats over
//! the page's foot, and beside the page is a panel of 40 rows of text.
//!
//! The window is 2024 by 1102.4 points (a 1.25 display's 2530 by 1378
//! pixels). The panel's width steps `WIDTH_STEPS` (40) times by
//! `WIDTH_STEP` (15) points from 500, a step a frame, then back, `ROUNDS`
//! (5) times: wider while the page is wider than its column (only the
//! column moves), then past it (its lines break again). Each frame's layout
//! (`prepare_frame` and `flush`) is timed; text is measured as a renderer
//! would, nothing is drawn.
//!
//! ```sh
//! cargo run --release -p zgx-desktop --example width_bench
//! ```
//!
//! Prints the median, p95 and mean layout time a width change takes, and
//! the nodes placed. `ZGX_PROFILE=1` also reports each phase on stderr.
use std::time::{Duration, Instant};
use zgx::compose::prelude::*;
use zgx::reactive::Signal;
use zgx::scene::Color;
use zgx::widgets::Ui;
use zgx_gpu::TextServices;

#[path = "support/headless_font.rs"]
mod headless_font;

fn setting(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

const PARAGRAPH: &str = "Layout answers each width a person drags past: the conversation's \
column keeps its measure while the page is wider, so only where it stands moves, and its \
lines break again once the page is narrower than it.";

/// One message: paragraphs, a bulleted list and a code block, in a centred
/// column of at most 660 points.
fn message(index: u32) -> View {
    let bullets = (0..4).map(move |item| {
        row().gap(8.).child(text("•").text_size(14.)).child(
            text(format!("item {item} of message {index}: {PARAGRAPH}"))
                .text_size(14.)
                .text_wrap(true)
                .flex_grow(1.)
                .flex_shrink(1.)
                .min_w(0.),
        )
    });
    let code = column()
        .p(8.)
        .bg(Color(240, 240, 244, 255))
        .children((0..6).map(move |line| {
            text(format!("let line_{line} = message_{index}.layout(width);")).text_size(12.)
        }));
    row()
        .w_full()
        .justify_center()
        .px(20.)
        .pt(16.)
        .relative()
        .child(
            column()
                .w_full()
                .max_w(660.)
                .min_w(0.)
                .gap(8.)
                .children((0..3).map(move |p| {
                    text(format!("{p}. {PARAGRAPH} {PARAGRAPH}"))
                        .text_size(14.)
                        .text_wrap(true)
                }))
                .child(column().gap(4.).children(bullets))
                .child(code),
        )
}

/// A transition's wrapper (Zeron's `ui.presence`): a one-cell grid filling
/// its parent, around `inner`.
fn presence(inner: View) -> View {
    column()
        .grid()
        .grid_cols(1)
        .grid_rows(1)
        .w_full()
        .h_full()
        .child(inner)
}

/// A band over its parent's top edge, as a title bar's: placed by its
/// insets, which lay its parent out by taffy.
fn band() -> View {
    div().absolute().top(0.).left(0.).right(0.).h(38.)
}

fn view(panel: Signal<f32>, offset: Signal<f32>, depth: u32, rows: u32) -> View {
    let conversation = scroll(offset)
        .w_full()
        .h_full()
        .child(column().w_full().children((0..rows).map(message)));
    let composer = row().w_full().justify_center().px(20.).child(
        column().w_full().max_w(660.).min_w(0.).p(12.).child(
            text("Ask for follow-up changes")
                .text_size(14.)
                .text_wrap(true),
        ),
    );
    // The page's layers, outermost first: a presence and a plain wrapper a
    // level, as a shell's screens nest.
    let mut content = column()
        .w_full()
        .h_full()
        .relative()
        .child(conversation)
        .child(
            div()
                .absolute()
                .left(0.)
                .right(0.)
                .bottom(16.)
                .child(composer),
        );
    for _ in 0..depth {
        content = presence(column().w_full().h_full().child(content));
    }
    let page = column()
        .flex_grow(1.)
        .min_w(0.)
        .h_full()
        .relative()
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .top(38.)
                .left(0.)
                .right(0.)
                .bottom(0.)
                .child(content),
        );
    let files = column()
        .reactive_style(move || Styles::new().w(panel.get()))
        .h_full()
        .flex_shrink(0.)
        .overflow_hidden()
        .relative()
        .p(8.)
        .children((0..40).map(|i| text(format!("crates/zgx/src/file_{i}.rs")).text_size(13.)));
    let sidebar = column()
        .w(316.)
        .h_full()
        .flex_shrink(0.)
        .p(8.)
        .children((0..30).map(|i| text(format!("Session {i}")).text_size(13.)));
    let screen = column().h_full().flex_grow(1.).min_w(0.).child(
        row()
            .h_full()
            .flex_grow(1.)
            .min_w(0.)
            .child(page)
            .child(files),
    );
    let shell = column()
        .w_full()
        .h_full()
        .relative()
        .child(
            row()
                .w_full()
                .flex_grow(1.)
                .min_h(0.)
                .child(sidebar)
                .child(screen),
        )
        .child(band());
    column()
        .w_full()
        .h_full()
        .relative()
        .child(shell)
        .child(band())
}

fn main() {
    let (depth, rows) = (setting("WIDTH_DEPTH", 6), setting("WIDTH_ROWS", 8));
    let (steps, step) = (setting("WIDTH_STEPS", 40), setting("WIDTH_STEP", 15));
    let rounds = setting("ROUNDS", 5);
    let mut ui = Ui::new(2024., 1102.4);
    let text = TextServices::new();
    headless_font::font().install(&mut text.text_system().borrow_mut());
    text.install(&mut ui.scene.borrow_mut());
    let panel = ui.signal(500_f32);
    let offset = ui.signal(0_f32);
    ui.mount(view(panel.clone(), offset, depth, rows));
    // Settled: laid out, then a few frames with nothing new.
    for _ in 0..3 {
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
    }
    let mut times = Vec::new();
    let mut nodes = 0_usize;
    for _ in 0..rounds {
        for i in (1..=steps).chain((0..steps).rev()) {
            panel.set(500. + (i * step) as f32);
            let started = Instant::now();
            ui.prepare_frame();
            let report = ui.scene.borrow_mut().flush();
            times.push(started.elapsed());
            nodes += report.layout_nodes;
            zgx::profile::frame();
        }
    }
    zgx::profile::report();
    let count = times.len().max(1);
    let total: Duration = times.iter().sum();
    times.sort();
    let at = |q: f64| times[((count as f64 * q) as usize).min(count - 1)];
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    println!(
        "{count} width changes: layout {:.3} ms median, {:.3} ms p95, {:.3} ms mean; \
         {:.0} nodes placed a change, scene nodes {}",
        ms(at(0.5)),
        ms(at(0.95)),
        ms(total) / count as f64,
        nodes as f64 / count as f64,
        ui.scene.borrow().len(),
    );
}
