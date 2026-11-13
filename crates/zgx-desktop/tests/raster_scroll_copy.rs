//! The software raster moves a scroll's pixels already drawn and repaints
//! only what the move can't provide, as the GPU renderer's scroll copies do:
//! every frame equals one rasterized whole, after the same scrolls, whatever
//! the step (whole rows, partial strips, sideways), whatever paints over or
//! around the view (an overlay staying put, rounded corners), and with one
//! scroll inside another. Before, a scroll repainted its whole view, 13 ms a
//! frame for a 1280 by 800 transcript.
use zgx::{compose::prelude::*, reactive::Signal, scene::Rect, widgets::Ui};
use zgx_desktop::raster::Raster;

const W: f32 = 320.;
const H: f32 = 240.;

/// Rows of stripes and text, `n` of them, `height` tall.
fn rows(n: usize, height: f32) -> View {
    let mut page = column().w(W).gap(3.);
    for i in 0..n {
        page = page.child(
            row()
                .w(W)
                .h(height)
                .gap(6.)
                .bg(if i % 2 == 0 {
                    rgb(0x1c2430)
                } else {
                    rgb(0x2a3546)
                })
                .child(div().w(12.).h(12.).rounded(6.).bg(rgb(0x3a8adf)))
                .child(
                    text(format!("row {i}: a line of text"))
                        .text_size(12.)
                        .text_color(rgb(0xe0e6ef)),
                ),
        );
    }
    page
}

/// Draws a frame both ways: the kept raster from its damage (moving what
/// scrolled), and a fresh one whole. They must match pixel for pixel.
fn frame(ui: &mut Ui, kept: &mut Raster, label: &str) {
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    kept.render(&ui.scene.borrow(), &damage);
    let mut whole = Raster::new(W as usize, H as usize);
    whole.render(&ui.scene.borrow(), &[Rect::new(0., 0., W, H)]);
    if let Some(at) = kept
        .pixels
        .iter()
        .zip(&whole.pixels)
        .position(|(a, b)| a != b)
    {
        panic!(
            "{label}: pixel ({}, {}) is {:06x}, drawn whole {:06x}",
            at % W as usize,
            at / W as usize,
            kept.pixels[at],
            whole.pixels[at]
        );
    }
}

/// Scrolls `offset` by each of `steps` in turn, a frame each.
fn scrolled(ui: &mut Ui, kept: &mut Raster, offset: &Signal<f32>, steps: &[f32], label: &str) {
    for (i, step) in steps.iter().enumerate() {
        offset.update(|o| *o = (*o + step).max(0.));
        frame(ui, kept, &format!("{label}, step {i} ({step:+})"));
    }
}

#[test]
fn a_list_scrolled_in_whole_and_partial_strips_matches_a_frame_drawn_whole() {
    let mut ui = Ui::new(W, H);
    let offset = ui.signal(0_f32);
    ui.mount(
        scroll(offset.clone())
            .scrollbar(false)
            .w(W)
            .h(H)
            .bg(rgb(0x101820))
            .child(rows(60, 24.)),
    );
    let mut kept = Raster::new(W as usize, H as usize);
    frame(&mut ui, &mut kept, "first");
    // Whole rows, then strips cutting rows, down and back up.
    let steps = [
        27., 27., 3., 7., 13., 1., 40., -5., -11., -27., -2., 90., -60.,
    ];
    scrolled(&mut ui, &mut kept, &offset, &steps, "down and up");
    assert_eq!(
        kept.scroll_copies(),
        steps.len() as u64,
        "every move copied"
    );
}

#[test]
fn a_sideways_scroll_copies_its_columns_and_matches_a_frame_drawn_whole() {
    let mut ui = Ui::new(W, H);
    let offset = ui.signal(0_f32);
    let mut strip = row().h(H).gap(4.);
    for i in 0..40 {
        strip = strip.child(
            div()
                .w(37.)
                .h(H)
                .bg(if i % 2 == 0 {
                    rgb(0x5a7a9a)
                } else {
                    rgb(0x9a5a7a)
                })
                .child(
                    text(format!("{i}"))
                        .text_size(12.)
                        .text_color(rgb(0xffffff)),
                ),
        );
    }
    ui.mount(
        scroll_x(offset.clone())
            .scrollbar(false)
            .w(W)
            .h(H)
            .bg(rgb(0x101820))
            .child(strip),
    );
    let mut kept = Raster::new(W as usize, H as usize);
    frame(&mut ui, &mut kept, "first");
    let steps = [9., 41., 2., 17., -6., -30., 55.];
    scrolled(&mut ui, &mut kept, &offset, &steps, "sideways");
    assert_eq!(
        kept.scroll_copies(),
        steps.len() as u64,
        "every move copied"
    );
}

/// A view inset in the window, its corners rounded, under a badge that
/// stays put: the copy carries the badge's pixels and the corners'; those
/// repaint where they are and where they were carried.
#[test]
fn a_clipped_rounded_view_under_an_overlay_repaints_what_the_copy_carried() {
    let mut ui = Ui::new(W, H);
    let offset = ui.signal(0_f32);
    ui.mount(
        overlay()
            .w(W)
            .h(H)
            .bg(rgb(0x000000))
            .child(
                div()
                    .absolute()
                    .left(20.)
                    .top(16.)
                    .w(240.)
                    .h(180.)
                    .rounded(14.)
                    .overflow_hidden()
                    .child(
                        scroll(offset.clone())
                            .scrollbar(false)
                            .w(240.)
                            .h(180.)
                            .bg(rgb(0x101820))
                            .child(rows(50, 20.)),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(150.)
                    .top(60.)
                    .w(80.)
                    .h(26.)
                    .rounded(8.)
                    .bg(rgba(0xd0a040e0))
                    .child(text("badge").text_size(11.).text_color(rgb(0x101010))),
            ),
    );
    let mut kept = Raster::new(W as usize, H as usize);
    frame(&mut ui, &mut kept, "first");
    let steps = [20., 7., 33., -9., -14., 3.];
    scrolled(&mut ui, &mut kept, &offset, &steps, "under the badge");
    assert!(kept.scroll_copies() > 0, "no move copied");
}

/// A sideways scroll inside a scroll: each moves alone, then both at once.
#[test]
fn a_scroll_inside_a_scroll_matches_a_frame_drawn_whole() {
    let mut ui = Ui::new(W, H);
    let (outer, inner) = (ui.signal(0_f32), ui.signal(0_f32));
    let mut strip = row().h(60.).gap(4.);
    for i in 0..30 {
        strip = strip.child(
            div()
                .w(44.)
                .h(60.)
                .bg(if i % 2 == 0 {
                    rgb(0x4a8a5a)
                } else {
                    rgb(0x8a4a5a)
                })
                .child(
                    text(format!("c{i}"))
                        .text_size(11.)
                        .text_color(rgb(0xffffff)),
                ),
        );
    }
    ui.mount(
        scroll(outer.clone())
            .scrollbar(false)
            .w(W)
            .h(H)
            .bg(rgb(0x101820))
            .child(
                column()
                    .w(W)
                    .gap(6.)
                    .child(rows(6, 22.))
                    .child(
                        scroll_x(inner.clone())
                            .scrollbar(false)
                            .w(W)
                            .h(60.)
                            .child(strip),
                    )
                    .child(rows(30, 22.)),
            ),
    );
    let mut kept = Raster::new(W as usize, H as usize);
    frame(&mut ui, &mut kept, "first");
    scrolled(&mut ui, &mut kept, &inner, &[13., 30., -7.], "inner alone");
    scrolled(&mut ui, &mut kept, &outer, &[11., 25., -4.], "outer alone");
    for (i, (o, n)) in [(9., 17.), (-6., 5.), (21., -12.)].into_iter().enumerate() {
        outer.update(|v| *v = (*v + o).max(0.));
        inner.update(|v| *v = (*v + n).max(0.));
        frame(&mut ui, &mut kept, &format!("both, step {i}"));
    }
    assert!(kept.scroll_copies() >= 6, "copied {}", kept.scroll_copies());
}

/// A move between pixels can't be copied: it repaints, and still matches.
#[test]
fn a_move_between_pixels_repaints_and_matches_a_frame_drawn_whole() {
    let mut ui = Ui::new(W, H);
    let offset = ui.signal(0_f32);
    ui.mount(
        scroll(offset.clone())
            .scrollbar(false)
            .w(W)
            .h(H)
            .bg(rgb(0x101820))
            .child(rows(40, 24.)),
    );
    let mut kept = Raster::new(W as usize, H as usize);
    frame(&mut ui, &mut kept, "first");
    scrolled(
        &mut ui,
        &mut kept,
        &offset,
        &[2.5, 0.25, 7.75],
        "between pixels",
    );
    assert_eq!(kept.scroll_copies(), 0, "a move between pixels copied");
}
