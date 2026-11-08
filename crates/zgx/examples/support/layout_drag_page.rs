// A page of six panes whose first pane's edge is dragged, built the same
// for `layout_drag_bench` (which the gate counts) and the layout test that
// counts its passes and measures: shared, so the two see one page.
//
// A shell under a title band its insets place, a sidebar, and six panes in
// a row: the first as wide as `split` says, the other five growing from no
// basis into the rest. Each pane lays a layer under its own band
// (`absolute`, both insets), transitions' one-cell grids and wrappers
// filling their parents, `DEPTH` levels of them, down to a scrolled column
// of messages of wrapping paragraphs; the third holds a measured list of
// wrapping rows instead.
use zgx::compose::prelude::*;
use zgx::reactive::{Runtime, Signal};

/// The transitions' wrappers each pane nests in.
pub const DEPTH: usize = 6;
/// The measured list's rows.
pub const LIST_ROWS: usize = 400;

const PARAGRAPH: &str = "A pane laid out at each width its edge is dragged past: \
its lines break again where the column narrows, and what it holds is placed anew.";

/// One message: two paragraphs and a short list, in a column of at most 560
/// points.
fn message(index: usize) -> View {
    row().w_full().justify_center().px(12.).pt(10.).child(
        column()
            .w_full()
            .max_w(560.)
            .min_w(0.)
            .gap(6.)
            .children((0..2).map(move |p| {
                text(format!("{index}.{p} {PARAGRAPH}"))
                    .text_size(14.)
                    .text_wrap(true)
            }))
            .children((0..2).map(move |item| {
                row().gap(6.).child(text("•").text_size(14.)).child(
                    text(format!("item {item}: {PARAGRAPH}"))
                        .text_size(14.)
                        .text_wrap(true)
                        .flex_grow(1.)
                        .flex_shrink(1.)
                        .min_w(0.),
                )
            })),
    )
}

/// A transition's wrapper: a one-cell grid filling its parent.
fn presence(inner: View) -> View {
    column()
        .grid()
        .grid_cols(1)
        .grid_rows(1)
        .w_full()
        .h_full()
        .child(inner)
}

/// A band over its parent's top edge, placed by its insets.
fn band() -> View {
    div().absolute().top(0.).left(0.).right(0.).h(32.)
}

/// What pane `index` scrolls: messages, or (the third and fifth) a
/// measured list.
/// Each measured list keeps its own heights: lists sized apart share none
/// (`measured_virtual_list`).
fn content(index: usize, offset: Signal<f32>, runtime: &Runtime) -> View {
    if index == 2 || index == 4 {
        return measured_virtual_list(
            offset,
            VariableHeights::new(runtime, LIST_ROWS, 60.),
            4,
            |row| row,
            |_, row, _| {
                column()
                    .w_full()
                    .p(8.)
                    .gap(4.)
                    .child(text(format!("Row {row}")).text_size(13.))
                    .child(
                        text(format!("{row}: {PARAGRAPH}"))
                            .text_size(13.)
                            .text_wrap(true),
                    )
            },
        )
        .w_full()
        .h_full();
    }
    scroll(offset).w_full().h_full().child(
        column()
            .w_full()
            .children((0..8).map(move |m| message(index * 8 + m))),
    )
}

fn pane(index: usize, split: Signal<f32>, runtime: &Runtime) -> View {
    let mut inner =
        column()
            .w_full()
            .h_full()
            .relative()
            .child(content(index, runtime.signal(0.), runtime));
    if index != 2 && index != 4 {
        inner = inner.child(
            div().absolute().left(0.).right(0.).bottom(12.).child(
                row().w_full().justify_center().px(12.).child(
                    text_area("Message", runtime.signal(String::new()))
                        .w_full()
                        .max_w(560.)
                        .h(64.),
                ),
            ),
        );
    }
    for _ in 0..DEPTH {
        inner = presence(column().w_full().h_full().child(inner));
    }
    let pane = column()
        .h_full()
        .min_w(0.)
        .relative()
        .overflow_hidden()
        .child(
            row()
                .h(32.)
                .px(8.)
                .gap(6.)
                .items_center()
                .child(text(format!("Pane {index}")).text_size(13.))
                .child(text("Split").text_size(12.)),
        )
        .child(
            div()
                .absolute()
                .top(32.)
                .left(0.)
                .right(0.)
                .bottom(0.)
                .child(inner),
        );
    match index {
        0 => pane
            .reactive_style(move || Styles::new().w(split.get()))
            .flex_shrink(0.),
        _ => pane.flex_grow(1.).flex_basis(0.),
    }
}

/// The page, its first pane `split` wide.
pub fn page(split: Signal<f32>, runtime: &Runtime) -> View {
    let sidebar = column()
        .w(220.)
        .h_full()
        .flex_shrink(0.)
        .p(8.)
        .children((0..24).map(|i| text(format!("Session {i}")).text_size(13.)));
    let panes = row()
        .h_full()
        .flex_grow(1.)
        .min_w(0.)
        .children((0..6).map(|i| pane(i, split.clone(), runtime)));
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
                .child(panes),
        )
        .child(band());
    column()
        .w_full()
        .h_full()
        .relative()
        .child(shell)
        .child(band())
}

/// The widths a drag takes, a frame each: `steps` of `step` points from
/// 320, then back.
pub fn widths(steps: usize, step: f32) -> impl Iterator<Item = f32> {
    (1..=steps)
        .chain((0..steps).rev())
        .map(move |i| 320. + i as f32 * step)
}
