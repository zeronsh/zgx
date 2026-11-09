//! `View::paging`: which page a lift picks, measured on a real `UIScrollView`,
//! and the spring read from UIKit that takes it there.
use std::time::Duration;
use zgx::{
    clock::VirtualClock,
    compose::prelude::*,
    input::{InputEvent, ScrollGesture},
    reactive::Signal,
    scene::Direction,
    widgets::Ui,
};

const PAGE: f32 = 402.;
const PAGES: f32 = 5.;

struct Pager {
    ui: Ui,
    clock: VirtualClock,
    offset: Signal<f32>,
    /// Where the clock has run to.
    ran: Duration,
}
impl Pager {
    fn new() -> Self {
        Self::with(|_, view| view, false)
    }
    fn with(wrap: impl FnOnce(&mut Ui, View) -> View, rtl: bool) -> Self {
        let clock = VirtualClock::new();
        let mut ui = Ui::new(PAGE + 40., 400.);
        let offset = ui.signal(0_f32);
        let view = scroll_x(offset.clone())
            .w(PAGE)
            .h(200.)
            .paging(true)
            .child(div().w(PAGE * PAGES).h(200.));
        let wrapped = wrap(&mut ui, view);
        let mounted = ui.mount(wrapped);
        if rtl {
            ui.scene
                .borrow_mut()
                .set_direction(mounted.node(), Some(Direction::Rtl));
        }
        ui.prepare_frame();
        Self {
            ui,
            clock,
            offset,
            ran: Duration::ZERO,
        }
    }
    fn at(&self) -> f32 {
        self.offset.get()
    }
    fn run(&mut self, ms: u64) {
        for _ in 0..ms / 4 {
            self.ran += Duration::from_millis(4);
            self.clock.advance(self.ran, || {});
            self.ui.advance_interactions(self.clock.now()).unwrap();
            self.ui.prepare_frame();
        }
    }
    fn phase(&mut self, phase: ScrollGesture) {
        self.ui.dispatch(InputEvent::ScrollGesture(phase));
    }
    fn scroll(&mut self, delta_x: f32) {
        self.ui.dispatch(InputEvent::Scroll {
            x: 50.,
            y: 50.,
            delta_x,
            delta_y: 0.,
        });
        self.ui.prepare_frame();
    }
    /// Fingers move `delta` at `speed` points a second, and lift.
    fn swipe(&mut self, delta: f32, speed: f32) {
        self.phase(ScrollGesture::Began);
        let steps = 8;
        let dt = (delta.abs() / speed.abs() * 1000. / steps as f32).max(4.) as u64;
        for _ in 0..steps {
            self.ran += Duration::from_millis(dt);
            self.clock.advance(self.ran, || {});
            self.scroll(delta / steps as f32);
        }
        self.phase(ScrollGesture::Ended);
    }
}

#[test]
fn a_slow_lift_goes_to_the_nearest_page() {
    // As a `UIScrollView`, all under its threshold: 150 of 402 back, 250 on.
    let mut p = Pager::new();
    p.swipe(150., 180.);
    p.run(1500);
    assert_eq!(p.at(), 0.);
    drop(p);
    let mut p = Pager::new();
    p.swipe(250., 180.);
    p.run(1500);
    assert_eq!(p.at(), PAGE);
}

/// Fingers move `delta` a scroll, each `gaps` milliseconds after the last in
/// turn, as an app busy drawing takes them in batches, and lift.
fn uneven_swipe(p: &mut Pager, delta: f32, gaps: &[u64], scrolls: usize) {
    p.phase(ScrollGesture::Began);
    for gap in gaps.iter().cycle().take(scrolls) {
        p.ran += Duration::from_millis(*gap);
        p.clock.advance(p.ran, || {});
        p.scroll(delta);
    }
    p.phase(ScrollGesture::Ended);
}

#[test]
fn a_slow_lift_is_slow_however_unevenly_its_scrolls_arrive() {
    // 2 points every 7.5 ms on average, 267 points a second: under the
    // threshold, though the last gap alone says 667.
    let mut p = Pager::new();
    uneven_swipe(&mut p, 2., &[12, 3], 20);
    p.run(1500);
    assert_eq!(p.at(), 0.);
    drop(p);
    // In threes after a stall: 2 points every 10 ms.
    let mut p = Pager::new();
    uneven_swipe(&mut p, 2., &[26, 2, 2], 21);
    p.run(1500);
    assert_eq!(p.at(), 0.);
}

#[test]
fn a_quick_lift_is_quick_however_unevenly_its_scrolls_arrive() {
    // 6 points every 8 ms on average, 750 points a second.
    let mut p = Pager::new();
    uneven_swipe(&mut p, 6., &[13, 3], 8);
    p.run(1500);
    assert_eq!(p.at(), PAGE);
}

#[test]
fn a_lift_after_turning_back_goes_by_the_turn_alone() {
    // On quickly, then back slowly: the last moves are what it lifts at.
    let mut p = Pager::new();
    p.phase(ScrollGesture::Began);
    for delta in [
        12., 12., 12., 12., 12., 12., 12., 12., -2., -2., -2., -2., -2., -2.,
    ] {
        p.ran += Duration::from_millis(8);
        p.clock.advance(p.ran, || {});
        p.scroll(delta);
    }
    p.phase(ScrollGesture::Ended);
    p.run(1500);
    assert_eq!(p.at(), 0.);
}

#[test]
fn a_quick_lift_goes_on_wherever_it_is() {
    // A `UIScrollView` from 10 points in at 0.36 points a millisecond, and from
    // 50 at 0.33, went to the next page.
    for (delta, speed) in [(10., 700.), (50., 700.), (190., 700.)] {
        let mut p = Pager::new();
        p.swipe(delta, speed);
        p.run(1500);
        assert_eq!(p.at(), PAGE, "{delta} at {speed}");
    }
}

#[test]
fn a_lift_under_the_threshold_stays_even_quite_far_along() {
    // A `UIScrollView` at 190 of 402, 0.27 points a millisecond, went back.
    let mut p = Pager::new();
    p.swipe(190., 200.);
    p.run(1500);
    assert_eq!(p.at(), 0.);
}

#[test]
fn a_lift_goes_at_most_one_page_from_where_the_gesture_began() {
    // A `UIScrollView` at 12000 points a second went one page. Fingers that
    // carried it past the next page still end one page on.
    let mut p = Pager::new();
    p.swipe(240., 12_000.);
    p.run(1500);
    assert_eq!(p.at(), PAGE);
    drop(p);
    let mut p = Pager::new();
    p.swipe(700., 3_000.);
    p.run(1500);
    assert_eq!(p.at(), PAGE);
}

#[test]
fn a_quick_lift_the_other_way_goes_back() {
    // A `UIScrollView` on page 1, moved on and flicked back, went to page 0.
    let mut p = Pager::new();
    p.swipe(300., 2_000.);
    p.run(1500);
    assert_eq!(p.at(), PAGE);
    p.swipe(-50., 800.);
    p.run(1500);
    assert_eq!(p.at(), 0.);
}

#[test]
fn it_moves_on_uikits_spring() {
    // v = (v + (target - x) * pull) * friction; x += v, a step a millisecond.
    let mut p = Pager::new();
    p.ui.set_refresh_interval(Duration::from_millis(4));
    p.offset.set(100.);
    p.ui.prepare_frame();
    p.phase(ScrollGesture::Began);
    p.phase(ScrollGesture::Ended);
    let (mut x, mut v) = (100_f32, 0_f32);
    let mut checked = 0;
    for _ in 0..30 {
        for _ in 0..12 {
            v = (v + (0. - x) * 0.000_284_858) * 0.970_815;
            x += v;
        }
        p.run(12);
        if p.at() > 0. {
            assert!((p.at() - x).abs() < 0.01, "at {} against {x}", p.at());
            checked += 1;
        }
    }
    assert!(checked > 5, "only {checked} steps moved");
}

#[test]
fn momentum_moves_nothing_until_it_ends() {
    let mut p = Pager::new();
    p.swipe(250., 180.);
    p.phase(ScrollGesture::MomentumBegan);
    for _ in 0..8 {
        p.scroll(40.);
    }
    p.run(1500);
    assert_eq!(p.at(), PAGE, "on the page it lifted for");
    // Momentum that outlasts the spring moves nothing either.
    p.scroll(40.);
    p.run(100);
    assert_eq!(p.at(), PAGE);
    p.phase(ScrollGesture::MomentumEnded);
    // And a wheel afterward is a wheel.
    p.ran += Duration::from_millis(400);
    p.scroll(10.);
    p.run(1500);
    assert_eq!(p.at(), PAGE * 2.);
}

#[test]
fn fingers_catch_it_as_it_moves() {
    let mut p = Pager::new();
    p.swipe(250., 180.);
    p.run(60);
    let caught = p.at();
    assert!(caught > 0. && caught < PAGE, "moving: {caught}");
    p.phase(ScrollGesture::Began);
    p.run(200);
    assert_eq!(p.at(), caught, "held where it was caught");
    p.scroll(5.);
    assert_eq!(p.at(), caught + 5.);
}

#[test]
fn a_wheel_goes_a_page_a_notch() {
    let mut p = Pager::new();
    p.scroll(48.);
    p.run(1500);
    assert_eq!(p.at(), PAGE);
    // One notch eased over several scrolls is one page.
    for _ in 0..6 {
        p.scroll(8.);
        p.run(16);
    }
    p.run(1500);
    assert_eq!(p.at(), PAGE * 2.);
    // And back.
    p.scroll(-48.);
    p.run(1500);
    assert_eq!(p.at(), PAGE);
}

#[test]
fn at_its_last_page_a_wheel_goes_on_to_what_is_around_it() {
    let outer = std::rc::Rc::new(std::cell::RefCell::new(None));
    let held = outer.clone();
    let mut p = Pager::with(
        move |ui, pager| {
            let around = ui.signal(0_f32);
            *held.borrow_mut() = Some(around.clone());
            scroll_x(around)
                .w(PAGE)
                .h(200.)
                .child(div().w(PAGE * 2.).h(200.).child(pager))
        },
        false,
    );
    let outer = outer.borrow().clone().unwrap();
    for _ in 0..(PAGES as usize - 1) {
        p.scroll(48.);
        p.run(1500);
    }
    assert_eq!(p.at(), PAGE * (PAGES - 1.));
    assert_eq!(outer.get(), 0.);
    // (After a pause: until then the wheel's gesture stays latched to it.)
    p.ui.input.pause_scrolling();
    p.ran += Duration::from_millis(400);
    p.clock.advance(p.ran, || {});
    p.scroll(48.);
    p.ui.prepare_frame();
    assert_eq!(p.at(), PAGE * (PAGES - 1.));
    assert!(outer.get() > 0., "the view around it took the wheel");
}

#[test]
fn a_move_that_is_no_gestures_settles_on_the_nearest_page() {
    let mut p = Pager::new();
    p.offset.set(PAGE + 30.);
    p.ui.prepare_frame();
    p.run(100);
    assert_eq!(p.at(), PAGE + 30., "not while it might go on moving");
    p.run(1500);
    assert_eq!(p.at(), PAGE);
    // On a page already, nothing moves and nothing is due.
    p.offset.set(PAGE * 2.);
    p.ui.prepare_frame();
    p.run(1500);
    assert_eq!(p.at(), PAGE * 2.);
    assert_eq!(p.ui.next_interaction_deadline(), None, "idle at rest");
}

#[test]
fn resized_it_stays_on_its_page() {
    let clock = VirtualClock::new();
    let mut ui = Ui::new(400., 300.);
    let (offset, width) = (ui.signal(0_f32), ui.signal(400_f32));
    let wide = width.clone();
    let tall = width.clone();
    ui.mount(
        scroll_x(offset.clone())
            .h(200.)
            .paging(true)
            .reactive_style(move || Styles::new().w(wide.get()))
            .child(
                div()
                    .h(200.)
                    .reactive_style(move || Styles::new().w(tall.get() * PAGES)),
            ),
    );
    ui.prepare_frame();
    offset.set(800.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 800., "page 2");
    width.set(300.);
    ui.prepare_frame();
    ui.prepare_frame();
    assert_eq!(offset.get(), 600., "page 2, of the new length");
    clock.advance(Duration::from_millis(500), || {});
    ui.advance_interactions(clock.now()).unwrap();
    ui.prepare_frame();
    assert_eq!(offset.get(), 600.);
}

#[test]
fn right_to_left_it_pages_the_same_way() {
    let mut p = Pager::with(|_, view| view, true);
    // A wheel's horizontal delta is physical: leftward moves on, right to left.
    p.scroll(-48.);
    p.run(1500);
    assert_eq!(p.at(), PAGE);
}

#[test]
fn nothing_is_due_at_rest_and_a_view_that_doesnt_page_scrolls_as_ever() {
    let mut p = Pager::new();
    p.run(40);
    assert_eq!(p.ui.next_interaction_deadline(), None);
    let mut ui = Ui::new(PAGE, 400.);
    let offset = ui.signal(0_f32);
    ui.mount(
        scroll_x(offset.clone())
            .w(PAGE)
            .h(200.)
            .child(div().w(PAGE * PAGES).h(200.)),
    );
    ui.prepare_frame();
    ui.dispatch(InputEvent::Scroll {
        x: 50.,
        y: 50.,
        delta_x: 48.,
        delta_y: 0.,
    });
    ui.prepare_frame();
    assert_eq!(offset.get(), 48.);
    assert_eq!(ui.next_interaction_deadline(), None);
}

/// A touch pan tells paged views alone (`paging_gesture`): of any other view, in
/// either axis, it changes nothing, its latch included.
#[test]
fn a_touch_pans_phases_reach_no_view_that_doesnt_page() {
    for horizontal in [false, true] {
        let mut ui = Ui::new(300., 300.);
        let (outer, inner) = (ui.signal(0_f32), ui.signal(0_f32));
        let list = |offset: Signal<f32>, along: f32| {
            if horizontal {
                scroll_x(offset)
                    .w(200.)
                    .h(200.)
                    .child(div().w(along).h(200.))
            } else {
                scroll(offset).w(200.).h(200.).child(div().h(along).w(200.))
            }
        };
        let view = if horizontal {
            scroll_x(outer.clone())
                .w(200.)
                .h(200.)
                .child(div().w(800.).h(200.).child(list(inner.clone(), 400.)))
        } else {
            scroll(outer.clone())
                .w(200.)
                .h(200.)
                .child(div().h(800.).w(200.).child(list(inner.clone(), 400.)))
        };
        ui.mount(view);
        ui.prepare_frame();
        let pan = |ui: &mut Ui, delta: f32| {
            let (delta_x, delta_y) = if horizontal { (delta, 0.) } else { (0., delta) };
            ui.dispatch(InputEvent::Scroll {
                x: 50.,
                y: 50.,
                delta_x,
                delta_y,
            });
            ui.prepare_frame();
        };
        // Past the inner view's end: latched to it, the outer doesn't move. A
        // reset latch would hand the rest to the outer view.
        pan(&mut ui, 600.);
        assert_eq!(inner.get(), 200.);
        assert_eq!(outer.get(), 0.);
        for phase in [
            ScrollGesture::Began,
            ScrollGesture::Ended,
            ScrollGesture::MomentumBegan,
            ScrollGesture::MomentumEnded,
        ] {
            ui.paging_gesture(phase);
            pan(&mut ui, 30.);
            assert_eq!(outer.get(), 0., "{phase:?}, horizontal {horizontal}");
            assert_eq!(ui.next_interaction_deadline(), None);
        }
    }
}
