//! Observed bounds and visibility come once a frame, at its preparation, as
//! the web delivers resize and intersection observations before paint:
//! however many scrolls dispatch between two frames, an observer of a row
//! they move hears one change, the last, and not one for each dispatch.
use std::{cell::Cell, cell::RefCell, rc::Rc};
use zgx::{
    compose::{NodeHandle, prelude::*},
    input::InputEvent,
    reactive::Signal,
    scene::Rect,
    widgets::Ui,
};

/// A row's observed bounds and visibility, kept as it mounts.
type Observed = Rc<RefCell<Option<(Signal<Rect>, Signal<bool>)>>>;

#[test]
fn scrolls_between_two_frames_move_an_observed_row_once() {
    let mut ui = Ui::new(200., 200.);
    let offset = ui.signal(0_f32);
    let observed: Observed = Rc::default();
    let keep = observed.clone();
    ui.mount(
        scroll(offset.clone()).w(200.).h(200.).child(
            column()
                .w(200.)
                .child(div().w(200.).h(300.))
                .child(div().w(100.).h(20.).on_mount(move |handle| {
                    *keep.borrow_mut() = Some((
                        handle.observe_bounds().expect("bounds"),
                        handle.observe_visibility_within(0.).expect("visibility"),
                    ));
                }))
                .child(div().w(200.).h(1000.)),
        ),
    );
    ui.prepare_frame();
    let (bounds, visible) = observed.borrow().clone().expect("mounted");
    assert_eq!(bounds.get().y, 300.);
    assert!(!visible.get(), "below the view");
    let heard = Rc::new(Cell::new(0));
    let (count, read) = (heard.clone(), bounds.clone());
    let _listener = ui.runtime.effect(move || {
        read.get();
        count.set(count.get() + 1);
    });
    let at_start = heard.get();
    for _ in 0..3 {
        ui.dispatch(InputEvent::Scroll {
            x: 100.,
            y: 100.,
            delta_x: 0.,
            delta_y: 50.,
        });
    }
    assert_eq!(heard.get(), at_start, "nothing heard while dispatching");
    ui.prepare_frame();
    assert_eq!(heard.get(), at_start + 1, "heard once, at the frame");
    assert_eq!(bounds.get().y, 150., "where the last scroll left it");
    assert!(visible.get(), "scrolled into the view");
}

/// A preparation ahead of the frame's (a host's spare-time work just before
/// it draws) lays out but leaves the observations to the frame's.
#[test]
fn a_preparation_ahead_of_the_frame_leaves_observations_to_it() {
    let mut ui = Ui::new(200., 200.);
    let offset = ui.signal(0_f32);
    let observed: Rc<RefCell<Option<Signal<Rect>>>> = Rc::default();
    let keep = observed.clone();
    ui.mount(
        scroll(offset.clone()).w(200.).h(200.).child(
            column()
                .w(200.)
                .child(div().w(200.).h(300.))
                .child(div().w(100.).h(20.).on_mount(move |handle| {
                    *keep.borrow_mut() = handle.observe_bounds();
                }))
                .child(div().w(200.).h(1000.)),
        ),
    );
    ui.prepare_frame();
    let bounds = observed.borrow().clone().expect("mounted");
    let heard = Rc::new(Cell::new(0));
    let (count, read) = (heard.clone(), bounds.clone());
    let _listener = ui.runtime.effect(move || {
        read.get();
        count.set(count.get() + 1);
    });
    let at_start = heard.get();
    ui.dispatch(InputEvent::Scroll {
        x: 100.,
        y: 100.,
        delta_x: 0.,
        delta_y: 40.,
    });
    ui.try_prepare_frame_ahead().expect("settles");
    assert_eq!(heard.get(), at_start, "left to the frame's");
    ui.prepare_frame();
    assert_eq!(heard.get(), at_start + 1);
    assert_eq!(bounds.get().y, 260.);
}

/// A row's handle and its observed bounds, kept as it mounts.
type Handled = (NodeHandle, Signal<Rect>);

/// What a handler reads of a node's geometry directly (`window_bounds`, as
/// the web's `getBoundingClientRect`) is where the latest scroll left it,
/// in the handler of a second scroll between two frames, while the node's
/// observed bounds still hold the last frame's.
#[test]
fn a_direct_read_in_the_second_scroll_sees_that_scroll() {
    let mut ui = Ui::new(200., 200.);
    let offset = ui.signal(0_f32);
    let mounted: Rc<RefCell<Option<Handled>>> = Rc::default();
    let keep = mounted.clone();
    ui.mount(
        scroll(offset.clone()).w(200.).h(200.).child(
            column()
                .w(200.)
                .child(div().w(200.).h(300.))
                .child(div().w(100.).h(20.).on_mount(move |handle| {
                    let bounds = handle.observe_bounds().expect("bounds");
                    *keep.borrow_mut() = Some((handle, bounds));
                }))
                .child(div().w(200.).h(1000.)),
        ),
    );
    ui.prepare_frame();
    let (handle, observed) = mounted.borrow().clone().expect("mounted");
    assert_eq!(handle.window_bounds().expect("shown").y, 300.);
    // Each scroll's handler: what it reads directly, and what is observed.
    let read: Rc<RefCell<Vec<(f32, f32, f32)>>> = Rc::default();
    let (seen, at) = (read.clone(), offset.clone());
    let _handler = ui.runtime.effect(move || {
        let offset = at.get();
        let direct = handle.window_bounds().expect("shown").y;
        let told = observed.with_untracked(|told| told.y);
        seen.borrow_mut().push((offset, direct, told));
    });
    read.borrow_mut().clear();
    for _ in 0..2 {
        ui.dispatch(InputEvent::Scroll {
            x: 100.,
            y: 100.,
            delta_x: 0.,
            delta_y: 50.,
        });
    }
    let reads = read.borrow().clone();
    let &(offset, direct, told) = reads.last().expect("the second scroll's handler ran");
    assert_eq!(offset, 100., "{reads:?}");
    assert_eq!(direct, 200., "fresh: where the second scroll left it");
    assert_eq!(told, 300., "observed: the last frame's");
}
