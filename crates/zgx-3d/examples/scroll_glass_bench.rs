//! What a fast scroll costs a frame's CPU, through the whole renderer, in a
//! long transcript of deep rows: `GROUPS` open groups of `ROWS` rows (each
//! row a dozen parts, each in the column a `ui.show` makes, `DEPTH` levels
//! down), each group's rows a list built as the scroll shows them, scrolled
//! `STEP` points a frame under a frosted composer with a shadow (a band of
//! 133 points), the list fading out over its last 61 points. With
//! `OBSERVE=1` (the default) rows watch their size, bounds and visibility
//! within a margin, as an app's `on_resize`, `ref.width` and `on_visible`
//! do. Each frame paints the strip the scroll brings in and the band.
//! The gate runs it on wgpu's no-op backend (`ZGX_GPU_NOOP`), zgx-3d's wgpu
//! with that backend, as `fx_bench` is.
//! Run: ZGX_GPU_NOOP=1 cargo run --release -p zgx-3d --example scroll_glass_bench
use std::{cell::RefCell, rc::Rc};
use zgx::{
    compose::prelude::*,
    reactive::Signal,
    scene::{BoxShadow, Color, Rect, Transform},
    style::rgb,
    widgets::Ui,
};
use zgx_gpu::GpuRenderer;

#[path = "../../zgx-desktop/examples/support/headless_font.rs"]
mod headless_font;

const WIDTH: f32 = 1240.;
const HEIGHT: f32 = 800.;
/// The composer's height: with its shadow and blur, a band of 133 points
/// repaints under it (120 made 188).
const COMPOSER: f32 = 65.;

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// What the observing nodes watch, kept alive (each a signal a mounted
/// node republishes: as a Luau `on_resize`, a `ref.width` read, an
/// `on_visible` with a margin).
#[derive(Default)]
struct Watched {
    bounds: Vec<Signal<Rect>>,
    sizes: Vec<Signal<(f32, f32)>>,
    visible: Vec<Signal<bool>>,
}
type Kept = Rc<RefCell<Watched>>;

fn watch_size(view: View, observe: bool, kept: &Kept) -> View {
    if !observe {
        return view;
    }
    let kept = kept.clone();
    view.on_mount(move |handle| kept.borrow_mut().sizes.extend(handle.observe_size()))
}
fn watch_bounds(view: View, observe: bool, kept: &Kept) -> View {
    if !observe {
        return view;
    }
    let kept = kept.clone();
    view.on_mount(move |handle| kept.borrow_mut().bounds.extend(handle.observe_bounds()))
}
fn watch_visible(view: View, observe: bool, kept: &Kept) -> View {
    if !observe {
        return view;
    }
    let kept = kept.clone();
    view.on_mount(move |handle| {
        kept.borrow_mut()
            .visible
            .extend(handle.observe_visibility_within(200.))
    })
}

/// A tool row: a dozen nodes (an icon, a title, a detail clipped to fade
/// out, a status badge, a time, a dot and their boxes), its column's size
/// watched, its title's and detail clip's bounds read, its detail's
/// visibility watched within a margin.
fn tool_row(index: usize, observe: bool, kept: &Kept) -> View {
    let title = watch_bounds(
        text(format!("tool {index}: read_file")).text_size(13.),
        observe,
        kept,
    );
    let detail = watch_visible(
        text(format!("crates/zgx/src/scene_{index}.rs, lines 1 to 240"))
            .text_size(12.)
            .text_color(rgb(0xa9a9ae)),
        observe,
        kept,
    );
    let clip = watch_bounds(div().grow().child(detail), observe, kept);
    // Each part in the column a `ui.show` wraps it in, as a transcript's
    // conditional parts are.
    let shown = |view: View| column().child(view);
    let row = row()
        .h(28.)
        .px(8.)
        .gap(8.)
        .items_center()
        .child(shown(div().size(16., 16.).rounded(4.).bg(rgb(0x3b82f6))))
        .child(shown(title))
        .child(shown(clip).grow())
        .child(shown(
            div()
                .px(6.)
                .rounded(8.)
                .bg(rgb(0x1f3a2a))
                .child(text("done").text_size(11.).text_color(rgb(0x6ee7a0))),
        ))
        .child(shown(
            text("0.4 s").text_size(11.).text_color(rgb(0x8a8a90)),
        ))
        .child(shown(div().size(8., 8.).rounded(4.).bg(rgb(0x6ee7a0))));
    watch_size(column().w_full().child(row), observe, kept)
}

/// The levels a transcript nests a row in below its group's list (a
/// message, its parts, a tool group, its body...): `depth` in all, the
/// last six single-child boxes the size of what they hold.
fn nested(mut view: View, depth: usize) -> View {
    for level in 0..depth {
        view = match level < 6 {
            true => div().w_full().child(view),
            false => div().w_full().pl(1.).child(view),
        };
    }
    view
}

/// An open tool group: a header (a message's parts, its size and its
/// header's visibility watched) and its rows, a list built as the
/// transcript's scroll shows them (`virtualize_in_ancestor`).
fn group(
    group: usize,
    rows: usize,
    depth: usize,
    observe: bool,
    kept: &Kept,
    list_offset: Signal<f32>,
) -> View {
    let header = watch_visible(
        row()
            .h(30.)
            .px(12.)
            .items_center()
            .child(text(format!("Ran {rows} tools")).text_size(13.)),
        observe,
        kept,
    );
    let row_kept = kept.clone();
    let list = virtual_list(
        list_offset,
        30.,
        4,
        move || rows,
        move |index| group * 1000 + index,
        move |_, key, _| nested(tool_row(key, observe, &row_kept), depth),
    )
    .virtualize_in_ancestor()
    .w_full();
    let inner = watch_size(
        column().gap(2.).px(12.).py(4.).w_full().child(list),
        observe,
        kept,
    );
    watch_size(
        column()
            .w_full()
            .rounded(10.)
            .bg(rgb(0x16181d))
            .child(header)
            .child(inner),
        observe,
        kept,
    )
}

fn main() {
    let frames = env("FRAMES", 120);
    let groups = env("GROUPS", 60);
    let rows = env("ROWS", 20);
    let depth = env("DEPTH", 28);
    let observe = env("OBSERVE", 1) == 1;
    let step = env("STEP", 24) as f32;
    let scale = env("SCALE", 2) as f32;
    let mut ui = Ui::new(WIDTH, HEIGHT);
    let mut gpu = GpuRenderer::new((WIDTH * scale) as u32, (HEIGHT * scale) as u32).expect("GPU");
    gpu.set_scale_factor(scale);
    headless_font::font().install(&mut gpu.text_system().borrow_mut());
    gpu.install_text(&mut ui.scene.borrow_mut());
    let offset = ui.signal(0_f32);
    let kept: Kept = Rc::default();
    let mut transcript = column().w_full().gap(10.).px(24.).pt(24.).pb(160.);
    for g in 0..groups {
        transcript = transcript.child(group(g, rows, depth, observe, &kept, ui.signal(0_f32)));
    }
    let composer = div()
        .absolute()
        .ml(120.)
        .mt(HEIGHT - COMPOSER - 20.)
        .size(WIDTH - 240., COMPOSER)
        .rounded(18.)
        .bg(Color(240, 240, 245, 40))
        .backdrop_blur(if env("BLUR", 1) == 1 { 16. } else { 0. })
        .shadow(BoxShadow {
            color: Color(0, 0, 0, 90),
            offset: Transform { x: 0., y: 8. },
            blur_radius: 24.,
            spread: 0.,
        })
        .child(
            text("Ask anything")
                .text_size(14.)
                .text_color(rgb(0xa9a9ae)),
        );
    ui.mount(
        div()
            .size(WIDTH, HEIGHT)
            .bg(rgb(0x0f1115))
            .text_color(rgb(0xe8e8ea))
            .child(
                scroll(offset.clone())
                    .size(WIDTH, HEIGHT)
                    .fade_edges(0., if env("FADE", 1) == 1 { 61. } else { 0. })
                    .child(transcript),
            )
            // In an overlay the size of the window, as an app's absolute
            // layers are.
            .child(div().absolute().size(WIDTH, HEIGHT).child(composer)),
    );
    let (mut damaged, mut items) = (0_u64, 0_usize);
    let mut at = 0_f32;
    let mut down = true;
    for frame in 0..frames {
        if frame > 0 {
            // Down to the end and back, a fast scroll's speed.
            at += if down { step } else { -step };
            let max = (groups as f32) * (30. + rows as f32 * 30. + 18.) - HEIGHT;
            if at >= max || at <= 0. {
                down = !down;
            }
            offset.set(at);
        }
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        let stats = gpu
            .render(&ui.scene.borrow(), &report.damage)
            .expect("a frame");
        damaged += stats.damaged_pixels;
        items += stats.instances;
    }
    let per = frames.max(1) as f64;
    let watched = kept.borrow();
    println!(
        "scroll under glass: {frames} frames, {} scene nodes, observers made: {} bounds, {} sizes, {} visible, \
         damaged {:.0} px/frame, instances {:.0}/frame",
        ui.scene.borrow().len(),
        watched.bounds.len(),
        watched.sizes.len(),
        watched.visible.len(),
        damaged as f64 / per,
        items as f64 / per,
    );
}
