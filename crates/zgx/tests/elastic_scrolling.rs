use std::time::{Duration, Instant};
use zgx::{
    compose::prelude::*,
    input::{InputEvent, ScrollGesture},
    reactive::Signal,
    scene::NodeId,
    widgets::Ui,
};

/// A 100-high scroll view over 400 of content, at the top.
fn mount(ui: &mut Ui) -> (Signal<f32>, NodeId) {
    let offset = ui.signal(0.);
    let view = ui.mount(scroll(offset.clone()).w(200.).h(100.).child(div().h(400.)));
    ui.prepare_frame();
    (offset, content(ui, view.node()))
}

/// A scroll view's content node (root, then viewport, then content).
fn content(ui: &Ui, root: NodeId) -> NodeId {
    let scene = ui.scene.borrow();
    scene.children(scene.children(root)[0])[0]
}

/// How far the content shows below where its offset puts it.
fn shift(ui: &Ui, content: NodeId, offset: &Signal<f32>) -> f32 {
    ui.scene.borrow().transform(content).y + offset.get()
}

fn scroll_by(ui: &mut Ui, delta_y: f32) {
    ui.dispatch(InputEvent::Scroll {
        x: 50.,
        y: 50.,
        delta_x: 0.,
        delta_y,
    });
    ui.prepare_frame();
}

fn phase(ui: &mut Ui, phase: ScrollGesture) {
    ui.dispatch(InputEvent::ScrollGesture(phase));
    ui.prepare_frame();
}

#[test]
fn without_elastic_scrolling_an_edge_holds() {
    let mut ui = Ui::new(300., 200.);
    let (offset, content) = mount(&mut ui);
    phase(&mut ui, ScrollGesture::Began);
    scroll_by(&mut ui, -50.);
    assert_eq!(shift(&ui, content, &offset), 0.);
    assert_eq!(ui.next_interaction_deadline(), None);
}

#[test]
fn a_trackpad_pulled_past_an_edge_stretches_it_and_letting_go_springs_back() {
    let mut ui = Ui::new(300., 200.);
    ui.set_elastic_scrolling(true);
    let (offset, content) = mount(&mut ui);
    phase(&mut ui, ScrollGesture::Began);
    scroll_by(&mut ui, -40.);
    let first = shift(&ui, content, &offset);
    assert!(
        0. < first && first < 40.,
        "stretched, with resistance: {first}"
    );
    assert_eq!(offset.get(), 0., "the offset stays in range");
    scroll_by(&mut ui, -40.);
    let second = shift(&ui, content, &offset);
    assert!(second - first < first, "each pixel further stretches less");
    // Back toward the content: the stretch goes first, then it scrolls.
    scroll_by(&mut ui, 60.);
    assert!(shift(&ui, content, &offset) < second);
    assert_eq!(offset.get(), 0.);
    scroll_by(&mut ui, 40.);
    assert_eq!(shift(&ui, content, &offset), 0.);
    assert!(offset.get() > 0., "and the rest scrolls");

    // Down at the end and let go: it springs back.
    offset.set(300.);
    ui.prepare_frame();
    scroll_by(&mut ui, 60.);
    assert!(shift(&ui, content, &offset) < 0., "the end stretches up");
    phase(&mut ui, ScrollGesture::Ended);
    let start = ui.next_interaction_deadline().expect("it springs back");
    // Momentum toward the edge meanwhile is spent in the spring: once it
    // settles, what is left of it does not bounce.
    phase(&mut ui, ScrollGesture::MomentumBegan);
    scroll_by(&mut ui, 10.);
    ui.advance_interactions(start + Duration::from_millis(100))
        .unwrap();
    let midway = shift(&ui, content, &offset);
    assert!(midway < 0. && midway > -30., "on its way: {midway}");
    ui.advance_interactions(start + Duration::from_millis(800))
        .unwrap();
    assert_eq!(shift(&ui, content, &offset), 0., "back");
    assert_eq!(offset.get(), 300.);
    scroll_by(&mut ui, 6.);
    assert_eq!(shift(&ui, content, &offset), 0., "no bounce after");
    phase(&mut ui, ScrollGesture::MomentumEnded);
    assert_eq!(
        ui.next_interaction_deadline(),
        None,
        "and nothing more to do"
    );
}

#[test]
fn a_mouse_wheel_never_stretches_and_momentum_bounces_off_an_edge() {
    let mut ui = Ui::new(300., 200.);
    ui.set_elastic_scrolling(true);
    let (offset, content) = mount(&mut ui);
    // A wheel's notches: no gesture.
    scroll_by(&mut ui, -40.);
    assert_eq!(shift(&ui, content, &offset), 0.);
    // A fling: the fingers lift, the momentum reaches the top.
    offset.set(30.);
    ui.prepare_frame();
    phase(&mut ui, ScrollGesture::Began);
    scroll_by(&mut ui, -10.);
    phase(&mut ui, ScrollGesture::Ended);
    phase(&mut ui, ScrollGesture::MomentumBegan);
    scroll_by(&mut ui, -30.);
    scroll_by(&mut ui, -20.);
    assert_eq!(offset.get(), 0.);
    assert!(
        shift(&ui, content, &offset) > 0.,
        "it bounces past the edge"
    );
    let deadline = ui.next_interaction_deadline().expect("and comes back");
    ui.advance_interactions(deadline + Duration::from_millis(800))
        .unwrap();
    ui.advance_interactions(Instant::now() + Duration::from_secs(2))
        .unwrap();
    assert_eq!(shift(&ui, content, &offset), 0.);
    // The rest of the momentum is spent at the edge: no second bounce.
    scroll_by(&mut ui, -8.);
    scroll_by(&mut ui, -4.);
    assert_eq!(shift(&ui, content, &offset), 0., "it bounces once");
    assert_eq!(ui.next_interaction_deadline(), None);
    phase(&mut ui, ScrollGesture::MomentumEnded);
    // A gesture of its own bounces again.
    phase(&mut ui, ScrollGesture::Began);
    phase(&mut ui, ScrollGesture::Ended);
    phase(&mut ui, ScrollGesture::MomentumBegan);
    scroll_by(&mut ui, -20.);
    assert!(shift(&ui, content, &offset) > 0., "a new fling bounces");
}

#[test]
fn a_nested_view_at_its_edge_lets_the_outer_one_stretch() {
    let mut ui = Ui::new(300., 300.);
    ui.set_elastic_scrolling(true);
    let (outer, inner) = (ui.signal(0.), ui.signal(0.));
    let view = ui.mount(
        scroll(outer.clone()).w(200.).h(200.).child(
            column()
                .child(
                    scroll(inner.clone())
                        .id("inner")
                        .w(200.)
                        .h(100.)
                        .child(div().h(300.)),
                )
                .child(div().h(400.)),
        ),
    );
    ui.prepare_frame();
    let outer_content = content(&ui, view.node());
    let inner_content = content(&ui, view.find("inner").unwrap());
    phase(&mut ui, ScrollGesture::Began);
    scroll_by(&mut ui, -40.);
    assert!(shift(&ui, outer_content, &outer) > 0., "the page bounces");
    assert_eq!(shift(&ui, inner_content, &inner), 0., "not the box in it");
}

/// Lists (`virtual_list`, measured lists: `ui.list`'s) stretch at their
/// edges and spring back as a scroll view does.
#[test]
fn lists_stretch_at_their_edges_and_spring_back() {
    for measured in [false, true] {
        let mut ui = Ui::new(300., 200.);
        ui.set_elastic_scrolling(true);
        let offset = ui.signal(0.);
        let list = if measured {
            let heights = VariableHeights::new(&ui.runtime, 40, 20.);
            measured_virtual_list(offset.clone(), heights, 1, |i| i, |_, _, _| div().h(20.))
        } else {
            virtual_list(offset.clone(), 20., 1, || 40, |i| i, |_, _, _| div().h(20.))
        };
        let view = ui.mount(list.w(200.).h(100.));
        ui.prepare_frame();
        let content = content(&ui, view.node());
        phase(&mut ui, ScrollGesture::Began);
        scroll_by(&mut ui, -40.);
        let pulled = shift(&ui, content, &offset);
        assert!(0. < pulled && pulled < 40., "measured {measured}: {pulled}");
        assert_eq!(offset.get(), 0.);
        phase(&mut ui, ScrollGesture::Ended);
        let start = ui.next_interaction_deadline().expect("it springs back");
        ui.advance_interactions(start + Duration::from_millis(800))
            .unwrap();
        ui.prepare_frame();
        assert_eq!(
            shift(&ui, content, &offset),
            0.,
            "measured {measured}: back"
        );
        // And the end, scrolled to, stretches up.
        offset.set(700.);
        ui.prepare_frame();
        phase(&mut ui, ScrollGesture::Began);
        scroll_by(&mut ui, 30.);
        assert!(
            shift(&ui, content, &offset) < 0.,
            "measured {measured}: the end"
        );
        assert_eq!(offset.get(), 700.);
    }
}

/// Springing back steps a refresh at a time, at the display's rate.
#[test]
fn a_spring_back_steps_every_refresh_of_the_display() {
    let mut ui = Ui::new(300., 200.);
    ui.set_elastic_scrolling(true);
    ui.set_refresh_interval(Duration::from_micros(8_333));
    let (_, _) = mount(&mut ui);
    phase(&mut ui, ScrollGesture::Began);
    scroll_by(&mut ui, -40.);
    phase(&mut ui, ScrollGesture::Ended);
    let start = ui.next_interaction_deadline().expect("it springs back");
    ui.advance_interactions(start).unwrap();
    let next = ui.next_interaction_deadline().expect("a step after");
    assert_eq!(next.duration_since(start), Duration::from_micros(8_333));
}

/// A list released past its end, and before its spring settles another
/// view taken by a gesture (sideways, over a horizontal scroller): each
/// springs back on its own. Only the gesture's latest view did; the list
/// kept its stretch for good, shifted with nothing below it.
#[test]
fn a_view_another_gesture_takes_over_from_still_springs_back() {
    let mut ui = Ui::new(300., 200.);
    ui.set_elastic_scrolling(true);
    let (down, across) = (ui.signal(0.), ui.signal(0.));
    let root = ui.mount(
        column()
            .child(scroll(down.clone()).w(200.).h(100.).child(div().h(400.)))
            .child(
                scroll_x(across.clone())
                    .w(200.)
                    .h(100.)
                    .child(div().w(400.).h(100.)),
            ),
    );
    ui.prepare_frame();
    let (list, strip) = {
        let scene = ui.scene.borrow();
        let column = scene.children(root.node());
        (column[0], column[1])
    };
    let (list, strip) = (content(&ui, list), content(&ui, strip));
    // The list, pulled past its start and let go: it starts back.
    phase(&mut ui, ScrollGesture::Began);
    scroll_by(&mut ui, -40.);
    assert!(shift(&ui, list, &down) > 0., "stretched");
    phase(&mut ui, ScrollGesture::Ended);
    // Within its spring, a sideways gesture over the strip stretches that.
    phase(&mut ui, ScrollGesture::Began);
    ui.dispatch(InputEvent::Scroll {
        x: 50.,
        y: 150.,
        delta_x: -40.,
        delta_y: 0.,
    });
    ui.prepare_frame();
    let strip_shift = ui.scene.borrow().transform(strip).x + across.get();
    assert!(strip_shift > 0., "the strip stretched: {strip_shift}");
    phase(&mut ui, ScrollGesture::Ended);
    // Both settle.
    while let Some(next) = ui.next_interaction_deadline() {
        ui.advance_interactions(next + Duration::from_secs(1))
            .unwrap();
        ui.prepare_frame();
    }
    assert_eq!(shift(&ui, list, &down), 0., "the list is back");
    assert_eq!(
        ui.scene.borrow().transform(strip).x + across.get(),
        0.,
        "the strip is back"
    );
    assert_eq!(ui.next_interaction_deadline(), None);
}

/// Stretched, then made to fit (its window grew past its content) with the
/// fingers still down: it can't scroll, so it shows no stretch, and none is
/// left once the gesture ends.
#[test]
fn a_view_made_to_fit_mid_stretch_lets_go_of_it() {
    let mut ui = Ui::new(300., 100.);
    ui.set_elastic_scrolling(true);
    let offset = ui.signal(0.);
    let view = ui.mount(scroll(offset.clone()).w(200.).h_full().child(div().h(300.)));
    ui.prepare_frame();
    let content = content(&ui, view.node());
    phase(&mut ui, ScrollGesture::Began);
    scroll_by(&mut ui, -40.);
    assert!(shift(&ui, content, &offset) > 0., "stretched");
    // The window grows taller than the content.
    ui.scene.borrow_mut().resize(300., 400.);
    ui.prepare_frame();
    assert_eq!(
        shift(&ui, content, &offset),
        0.,
        "fitting, no stretch shows"
    );
    phase(&mut ui, ScrollGesture::Ended);
    while let Some(next) = ui.next_interaction_deadline() {
        ui.advance_interactions(next + Duration::from_secs(1))
            .unwrap();
        ui.prepare_frame();
    }
    // Shrunk again so it scrolls: nothing of the old stretch comes back.
    ui.scene.borrow_mut().resize(300., 100.);
    ui.prepare_frame();
    assert_eq!(shift(&ui, content, &offset), 0., "no stretch left over");
    assert_eq!(ui.next_interaction_deadline(), None);
}

/// Scrolled to its end, then its content shrinks or its viewport grows: the
/// offset comes back within what it can scroll now, faded overflow or not,
/// so nothing shows shifted up with empty space below.
#[test]
fn an_offset_past_a_new_range_comes_back_within_it() {
    for fade in [false, true] {
        let mut ui = Ui::new(300., 100.);
        let (offset, rows) = (ui.signal(0.), ui.signal(40_usize));
        let list = rows.clone();
        let view = ui.mount(
            scroll(offset.clone())
                .w(200.)
                .h_full()
                .fade_overflow(fade)
                .child(column().child(keyed(
                    move || (0..list.get()).collect::<Vec<_>>(),
                    |_, _| div().h(20.),
                ))),
        );
        ui.prepare_frame();
        let content = content(&ui, view.node());
        offset.set(f32::MAX);
        ui.prepare_frame();
        assert_eq!(offset.get(), 700., "at its end: 800 of rows in 100");
        // Fewer rows: 300 tall, 200 to scroll.
        rows.set(15);
        ui.prepare_frame();
        assert_eq!(offset.get(), 200., "fade {fade}: within the new range");
        assert_eq!(ui.scene.borrow().transform(content).y, -200.);
        // A taller window: everything fits.
        ui.scene.borrow_mut().resize(300., 400.);
        ui.prepare_frame();
        assert_eq!(offset.get(), 0., "fade {fade}: fits, at its start");
        assert_eq!(ui.scene.borrow().transform(content).y, 0.);
    }
}
