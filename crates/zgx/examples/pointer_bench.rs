//! A pointer moving over a deep page, headless and undrawn: each move
//! dispatched (hit test, hover, listeners) and its cursor asked for, as a
//! window's host does, then the frame flushed. CI's instruction gate counts
//! it, so what a mouse move costs zgx itself is guarded.
//! Run: cargo run -p zgx --release --example pointer_bench
//! (`MOVES` sets the moves, 600 by default.)
use zgx::{
    compose::{View, button, column, row, text},
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

fn main() {
    let moves: usize = std::env::var("MOVES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let mut ui = Ui::new(1200., 800.);
    let page = column()
        .gap(4.)
        .cursor(Cursor::Default)
        .children((0..60).map(|i| {
            row()
                .gap(8.)
                .children((0..4).map(move |j| card(i * 4 + j, 8)))
        }));
    let _handle = ui.mount(page);
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    // Back and forth along a diagonal: most moves stay in the boxes they were
    // in, some cross into the next.
    let mut cursors = 0;
    for step in 0..moves {
        let t = (step % 200) as f32;
        let (x, y) = (40. + t * 3., 20. + t * 2.);
        ui.dispatch(InputEvent::PointerMove { x, y });
        let scene = ui.scene.borrow();
        cursors += usize::from(ui.input.cursor_at(&scene, x, y).is_some());
        drop(scene);
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
    }
    println!("{moves} moves, a cursor on {cursors}");
}
