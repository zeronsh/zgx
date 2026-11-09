//! A wheel gesture latches to the scroll view it began in, as macOS's do,
//! and a scroll view takes only its own axis of a diagonal swipe.
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

// A gesture's latch holds for it and its momentum, never past them: each of
// these is what a person did right after scrolling one view, and each
// scroll must reach the view now under the pointer, as AppKit's would.

use zgx::{input::ScrollGesture, reactive::Signal};

/// Two lists side by side: A at x 75, B at x 225, each 150 points across.
fn side_by_side(ui: &mut Ui) -> (Signal<f32>, Signal<f32>) {
    let (a, b) = (ui.signal(0.), ui.signal(0.));
    let list = |offset: &Signal<f32>, name: &str| {
        scroll(offset.clone())
            .w(150.)
            .h(300.)
            .child(column().children((0..40).map(|i| text(format!("{name} {i}")).h(20.))))
    };
    ui.mount(row().child(list(&a, "A")).child(list(&b, "B")));
    ui.prepare_frame();
    (a, b)
}

const A: (f32, f32) = (75., 150.);
const B: (f32, f32) = (225., 150.);

fn phase(ui: &mut Ui, phase: ScrollGesture) {
    ui.dispatch(InputEvent::ScrollGesture(phase));
}

fn point(ui: &mut Ui, (x, y): (f32, f32)) {
    ui.dispatch(InputEvent::PointerMove { x, y });
    ui.prepare_frame();
}

/// A trackpad flick over A, as the macOS host sends one: the fingers'
/// scrolls, their lift, and the coast that goes on after it, delivered at
/// where the fingers were.
fn flick(ui: &mut Ui) {
    point(ui, A);
    phase(ui, ScrollGesture::Began);
    for _ in 0..3 {
        wheel(ui, A.0, A.1, 0., 20.);
    }
    phase(ui, ScrollGesture::Ended);
    phase(ui, ScrollGesture::MomentumBegan);
    for _ in 0..3 {
        wheel(ui, A.0, A.1, 0., 5.);
    }
}

/// A new trackpad gesture over B, its fingers down while A's coast runs:
/// the host stops the coast (its momentum ends) and the gesture begins.
fn swipe_b_during_coast(ui: &mut Ui) {
    phase(ui, ScrollGesture::MomentumEnded);
    phase(ui, ScrollGesture::Began);
    for _ in 0..2 {
        wheel(ui, B.0, B.1, 0., 20.);
    }
}

#[test]
fn a_swipe_over_another_view_during_the_last_ones_coast_goes_there() {
    let mut ui = Ui::new(400., 400.);
    let (a, b) = side_by_side(&mut ui);
    flick(&mut ui);
    assert_eq!(a.get(), 75.);
    point(&mut ui, B);
    swipe_b_during_coast(&mut ui);
    assert_eq!(b.get(), 40., "the swipe over B moves B");
    assert_eq!(a.get(), 75., "and leaves A where its coast was stopped");
}

#[test]
fn a_swipe_over_another_view_right_after_a_lift_goes_there() {
    let mut ui = Ui::new(400., 400.);
    let (a, b) = side_by_side(&mut ui);
    point(&mut ui, A);
    phase(&mut ui, ScrollGesture::Began);
    wheel(&mut ui, A.0, A.1, 0., 20.);
    phase(&mut ui, ScrollGesture::Ended);
    // No pause, no coast: the pointer goes straight to B and swipes.
    point(&mut ui, B);
    phase(&mut ui, ScrollGesture::Began);
    wheel(&mut ui, B.0, B.1, 0., 20.);
    assert_eq!((a.get(), b.get()), (20., 20.));
}

#[test]
fn a_swipe_after_clicking_another_view_goes_there() {
    let mut ui = Ui::new(400., 400.);
    let (a, b) = side_by_side(&mut ui);
    flick(&mut ui);
    point(&mut ui, B);
    ui.dispatch(InputEvent::PointerDown {
        x: B.0,
        y: B.1,
        button: zgx::input::PointerButton::Primary,
    });
    ui.dispatch(InputEvent::PointerUp {
        x: B.0,
        y: B.1,
        button: zgx::input::PointerButton::Primary,
    });
    // A scroll with no phases (a host without them) right after the click.
    wheel(&mut ui, B.0, B.1, 0., 20.);
    assert_eq!((a.get(), b.get()), (75., 20.));
}

#[test]
fn a_coast_stays_with_its_view_while_the_pointer_moves_off_it() {
    let mut ui = Ui::new(400., 400.);
    let (a, b) = side_by_side(&mut ui);
    flick(&mut ui);
    // A host that delivers momentum where the pointer is now (the system's,
    // elsewhere than macOS): the momentum is still the gesture's.
    point(&mut ui, B);
    wheel(&mut ui, B.0, B.1, 0., 5.);
    assert_eq!((a.get(), b.get()), (80., 0.));
}

#[test]
fn a_swipe_over_a_nested_view_during_the_pages_coast_goes_to_it() {
    let mut ui = Ui::new(400., 400.);
    let (page, inner) = (ui.signal(0.), ui.signal(0.));
    ui.mount(
        scroll(page.clone()).w(300.).h(300.).child(
            column()
                .children((0..2).map(|i| text(format!("Above {i}")).h(20.)))
                .child(
                    scroll(inner.clone()).w(300.).h(100.).child(
                        column().children((0..10).map(|i| text(format!("Inner {i}")).h(20.))),
                    ),
                )
                .children((0..40).map(|i| text(format!("Page {i}")).h(20.))),
        ),
    );
    ui.prepare_frame();
    // The page flicked from below the inner list, which it brings up under
    // the pointer by the time the next gesture starts...
    let below = (150., 250.);
    point(&mut ui, below);
    phase(&mut ui, ScrollGesture::Began);
    wheel(&mut ui, below.0, below.1, 0., 20.);
    phase(&mut ui, ScrollGesture::Ended);
    phase(&mut ui, ScrollGesture::MomentumBegan);
    wheel(&mut ui, below.0, below.1, 0., 5.);
    assert_eq!(page.get(), 25.);
    // ...and a swipe over the inner list, there now, moves it.
    let over = (150., 40.);
    point(&mut ui, over);
    phase(&mut ui, ScrollGesture::MomentumEnded);
    phase(&mut ui, ScrollGesture::Began);
    wheel(&mut ui, over.0, over.1, 0., 20.);
    assert_eq!((page.get(), inner.get()), (25., 20.));
}
