//! A splitter drag: a sidebar and two panes in absolute boxes sized by
//! functions over one geometry. Dragging the divider between the panes lays
//! out the two panes, and nothing under the sidebar, whose constraints did
//! not change.
use zgx::{compose::prelude::*, widgets::Ui};

#[allow(dead_code)]
fn sidebar() -> View {
    column()
        .id("sidebar")
        .absolute()
        .left(0.)
        .top(0.)
        .w(220.)
        .h_full()
        .children((0..200).map(|i| {
            row()
                .h(24.)
                .child(text(format!("item {i}")).text_size(13.))
                .child(div().w(8.).h(8.))
        }))
}

fn pane(id: &'static str, place: impl Fn() -> (f32, f32) + 'static) -> View {
    column()
        .id(id)
        .reactive_style(move || {
            let (left, width) = place();
            Styles::new()
                .absolute()
                .top(0.)
                .left(left)
                .w(width)
                .h_full()
        })
        .children((0..20).map(|i| {
            text(format!("{id} line {i} {}", "lorem ipsum ".repeat(6)))
                .text_wrap(true)
                .w_full()
        }))
}

#[test]
fn dragging_a_splitter_lays_out_the_panes_and_not_the_sidebar() {
    let mut ui = Ui::new(1000., 700.);
    let split = ui.signal(390.);
    let (a, b) = (split.clone(), split.clone());
    ui.mount(column().w_full().h_full().children(vec![
        div().h(40.).w_full().child(text("titlebar")),
        row().flex_grow(1.).w_full().children(vec![
                column()
                    .reactive_style({
                        let s = split.clone();
                        move || {
                            let _ = s.get();
                            Styles::new().w(220.).h_full().min_w_percent(5.).p(0.)
                        }
                    })
                    .shrink_0()
                    .children((0..200).map(|i| {
                        row()
                            .h(24.)
                            .child(text(format!("item {i}")).text_size(13.))
                            .child(div().w(8.).h(8.))
                    })),
                div()
                    .relative()
                    .flex_grow(1.)
                    .h_full()
                    .children(vec![
                        pane("a", move || (0., a.get())),
                        pane("b", move || (b.get(), 780. - b.get())),
                    ]),
            ]),
    ]));
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let mut counts = Vec::new();
    for step in 1..=6 {
        split.set(390. + 6. * step as f32);
        ui.prepare_frame();
        counts.push(ui.scene.borrow_mut().flush().layout_nodes);
    }
    // The sidebar alone is 600 nodes and a titlebar: a frame that touched them is over that.
    assert!(counts.iter().all(|n| *n < 300), "{counts:?}");
}
