//! A pointer moving over a deep page, headless and undrawn: each move
//! dispatched (hit test, hover, listeners) and its cursor asked for, as a
//! window's host does, then the frame flushed. CI's instruction gate counts
//! it, so what a mouse move costs zgx itself is guarded.
//! Run: cargo run -p zgx --release --example pointer_bench
//! (`MOVES` sets the moves, 600 by default.) `SIDEBAR=N` adds a sidebar of
//! N rows beside the page, as an app's folder tree: each with hover, a label
//! and an absolutely placed badge, one badge raised (`z`), which takes every
//! hit test by the walk keeping paint order. The pointer then moves down the
//! sidebar, across its rows, instead of over the page.
use zgx::{
    compose::{View, button, column, div, row, text},
    cursor::Cursor,
    input::InputEvent,
    style::{Styled, rgb},
    widgets::Ui,
};

/// A card nested `depth` levels deep: a column of rows, each a few boxes in.
fn card(i: usize, depth: usize) -> View {
    let mut inner = row().gap(6.).child(text(format!("Item {i}"))).child(
        button()
            .cursor(Cursor::Pointer)
            .hover(|s| s.bg(rgb(0x2a3a50)))
            .child(text("Open")),
    );
    for level in 0..depth {
        inner = column()
            .p(2.)
            .hover(move |s| s.bg(rgb(0x101820 + level as u32)))
            .child(inner);
    }
    inner
}

/// The sidebar's rows (`SIDEBAR`): row `raised` has its badge raised.
fn sidebar(rows: usize, raised: usize) -> View {
    column()
        .w(200.)
        .shrink_0()
        .children((0..rows).map(move |i| {
            let badge = div()
                .absolute()
                .top(2.)
                .right(2.)
                .w(16.)
                .h(16.)
                .bg(rgb(0x30a0f0));
            row()
                .h(24.)
                .w(200.)
                .hover(|s| s.bg(rgb(0x202830)))
                .child(text(format!("Folder {i}")))
                .child(if i == raised { badge.z(1) } else { badge })
        }))
}

fn main() {
    let moves: usize = std::env::var("MOVES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let side: usize = std::env::var("SIDEBAR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut ui = Ui::new(1200., 800.);
    let page = column()
        .gap(4.)
        .cursor(Cursor::Default)
        .children((0..60).map(|i| {
            row()
                .gap(8.)
                .children((0..4).map(move |j| card(i * 4 + j, 8)))
        }));
    // The page alone, as the cell has always measured, or beside a sidebar.
    let page = match side {
        0 => page,
        rows => row().child(sidebar(rows, rows / 2)).child(page),
    };
    let _handle = ui.mount(page);
    ui.prepare_frame();
    {
        let flushed = ui.scene.borrow_mut().flush();
        if zgx_fingerprint::on() {
            zgx_fingerprint::frame(&ui.scene.borrow(), &flushed);
        }
    }
    // Back and forth along a diagonal: most moves stay in the boxes they were
    // in, some cross into the next. Down the sidebar's rows, beside one.
    let mut cursors = 0;
    for step in 0..moves {
        let t = (step % 200) as f32;
        let (x, y) = match side {
            0 => (40. + t * 3., 20. + t * 2.),
            _ => (20. + t * 0.8, 20. + t * 3.5),
        };
        ui.dispatch(InputEvent::PointerMove { x, y });
        let scene = ui.scene.borrow();
        cursors += usize::from(ui.input.cursor_at(&scene, x, y).is_some());
        drop(scene);
        ui.prepare_frame();
        {
            let flushed = ui.scene.borrow_mut().flush();
            if zgx_fingerprint::on() {
                zgx_fingerprint::frame(&ui.scene.borrow(), &flushed);
            }
        }
    }
    println!("{moves} moves, a cursor on {cursors}");
}
