//! A wheel gesture latches to the scroll view it began in, as macOS's do,
//! and a scroll view across a diagonal swipe's main axis takes only its own
//! axis of it; one along that axis takes it whole.
use zgx::{compose::prelude::*, input::InputEvent, widgets::Ui};

fn wheel(ui: &mut Ui, x: f32, y: f32, delta_x: f32, delta_y: f32) {
    ui.dispatch(InputEvent::Scroll {
        x,
        y,
        delta_x,
        delta_y,
    });
    ui.prepare_frame();
}

#[test]
fn a_gesture_stays_with_the_view_it_began_in_until_a_pause() {
    let mut ui = Ui::new(400., 400.);
    let (outer, inner) = (ui.signal(0.), ui.signal(0.));
    ui.mount(
        scroll(outer.clone()).w(300.).h(300.).child(
            column()
                .child(
                    scroll(inner.clone()).w(300.).h(100.).child(
                        column().children((0..10).map(|i| text(format!("Inner {i}")).h(20.))),
                    ),
                )
                .children((0..40).map(|i| text(format!("Outer {i}")).h(20.))),
        ),
    );
    ui.prepare_frame();
    // Over the inner view: it scrolls to its end (100 of content 200)...
    for _ in 0..6 {
        wheel(&mut ui, 50., 50., 0., 25.);
    }
    assert_eq!(inner.get(), 100.);
    // ...and the rest of the gesture does not chain to the outer one.
    assert_eq!(outer.get(), 0., "latched to the inner view");
    // After a pause, a new gesture at its end chains outward.
    std::thread::sleep(std::time::Duration::from_millis(350));
    wheel(&mut ui, 50., 50., 0., 25.);
    assert_eq!(outer.get(), 25.);
}

#[test]
fn a_diagonal_swipe_over_a_horizontal_view_still_scrolls_the_page() {
    let mut ui = Ui::new(400., 400.);
    let (page, carousel) = (ui.signal(0.), ui.signal(0.));
    ui.mount(
        scroll(page.clone()).w(300.).h(300.).child(
            column()
                .child(scroll_x(carousel.clone()).w(300.).h(60.).child(
                    row().children((0..10).map(|i| text(format!("Card {i}")).w(100.).h(60.))),
                ))
                .children((0..40).map(|i| text(format!("Line {i}")).h(20.))),
        ),
    );
    ui.prepare_frame();
    wheel(&mut ui, 50., 30., 10., 15.);
    assert_eq!(carousel.get(), 10., "the carousel takes its axis");
    assert_eq!(page.get(), 15., "the page the other");
}

#[test]
fn sideways_jitter_on_a_swipe_over_an_inner_list_stays_with_it() {
    let mut ui = Ui::new(400., 400.);
    let (outer, inner) = (ui.signal(0.), ui.signal(0.));
    ui.mount(
        scroll(outer.clone()).w(300.).h(300.).child(
            column()
                .child(
                    virtual_list(
                        inner.clone(),
                        20.,
                        1,
                        || 10,
                        |n| n,
                        |_, n, _| text(format!("Row {n}")),
                    )
                    .w(300.)
                    .h(100.),
                )
                .children((0..40).map(|i| text(format!("Outer {i}")).h(20.))),
        ),
    );
    ui.prepare_frame();
    // A vertical swipe, a point sideways each event: the list takes it all.
    for _ in 0..6 {
        wheel(&mut ui, 50., 50., 1., 25.);
    }
    assert_eq!(inner.get(), 100.);
    assert_eq!(outer.get(), 0., "latched to the list");
}
