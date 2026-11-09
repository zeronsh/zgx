//! A wheel over a `ui.scroll_x` in a page. A trackpad's swipe goes the way
//! it mostly moves and stays with what it moved, drifting or not; a mouse
//! wheel, with `wheel_across`, moves the view sideways once the pointer was
//! moved onto it, and with Shift always; the page keeps everything else.
mod common;
use common::Harness;
use zgx::input::InputEvent;

const APP: &str = r#"
local page = ui.state("page", 0)
local across = ui.state("across", 0)
local cards = {}
for i = 1, 10 do
    table.insert(cards, ui.div { w = 100, h = 60, shrink = 0 })
end
return ui.column {
    ui.text { id = "state", h = 20, function()
        return `{math.floor(page:get())} {math.floor(across:get())}`
    end },
    ui.scroll { id = "page", h = 200, w = 200, offset = page, ui.column {
        ui.scroll_x { id = "cards", h = 60, w = 200, offset = across, wheel_across = true, ui.row(cards) },
        ui.div { h = 1000 },
    } },
}
"#;

/// The same page around a plain `scroll_x` (no `wheel_across`).
const PLAIN: &str = r#"
local page = ui.state("page", 0)
local across = ui.state("across", 0)
local cards = {}
for i = 1, 10 do
    table.insert(cards, ui.div { w = 100, h = 60, shrink = 0 })
end
return ui.column {
    ui.text { id = "state", h = 20, function()
        return `{math.floor(page:get())} {math.floor(across:get())}`
    end },
    ui.scroll { id = "page", h = 200, w = 200, offset = page, ui.column {
        ui.scroll_x { id = "cards", h = 60, w = 200, offset = across, ui.row(cards) },
        ui.div { h = 1000 },
    } },
}
"#;

const OVER: (f32, f32) = (100., 50.);

fn app() -> Harness {
    let h = Harness::inline(APP);
    h.frames(2);
    h
}

fn plain() -> Harness {
    let h = Harness::inline(PLAIN);
    h.frames(2);
    h
}

fn scroll(dx: f32, dy: f32) -> InputEvent {
    InputEvent::Scroll {
        x: OVER.0,
        y: OVER.1,
        delta_x: dx,
        delta_y: dy,
    }
}

fn swipe(h: &mut Harness, dx: f32, dy: f32) {
    h.ui.dispatch(scroll(dx, dy));
    h.frames(1);
}

fn wheel(h: &mut Harness, dy: f32, shift: bool) {
    let event = scroll(0., dy);
    zgx::input::mouse_wheel(shift, || h.ui.dispatch(event));
    h.frames(1);
}

/// A pause between gestures: the next scroll goes to what's under the pointer.
fn pause(h: &mut Harness) {
    h.ui.input.pause_scrolling();
}

fn point(h: &mut Harness) {
    h.ui.dispatch(InputEvent::PointerMove {
        x: OVER.0,
        y: OVER.1,
    });
    h.frames(1);
}

#[test]
fn a_swipe_mostly_sideways_stays_with_the_view_it_moved() {
    let mut h = app();
    for _ in 0..4 {
        swipe(&mut h, 10., 2.);
    }
    assert_eq!(h.text("state"), "0 40");
}

#[test]
fn a_swipe_mostly_down_is_the_pages_and_leaves_the_view_alone() {
    let mut h = app();
    for _ in 0..3 {
        swipe(&mut h, 2., 10.);
    }
    // A moment that's mostly sideways doesn't take the gesture from the page.
    swipe(&mut h, 3., 1.);
    assert_eq!(h.text("state"), "31 0");
}

#[test]
fn a_mouse_wheel_is_the_pages_until_the_pointer_moves_onto_the_view() {
    let mut h = app();
    wheel(&mut h, 40., false);
    assert_eq!(h.text("state"), "40 0");
    pause(&mut h);
    point(&mut h);
    wheel(&mut h, -40., false);
    assert_eq!(
        h.text("state"),
        "0 0",
        "at its start, back up is the page's"
    );
    pause(&mut h);
    point(&mut h);
    wheel(&mut h, 40., false);
    assert_eq!(h.text("state"), "0 40");
}

#[test]
fn a_mouse_wheel_at_the_views_end_goes_on_to_the_page_after_a_pause() {
    let mut h = app();
    point(&mut h);
    wheel(&mut h, 2000., false);
    assert_eq!(h.text("state"), "0 800");
    wheel(&mut h, 40., false);
    assert_eq!(h.text("state"), "0 800", "the gesture stays where it was");
    pause(&mut h);
    wheel(&mut h, 40., false);
    assert_eq!(h.text("state"), "40 800");
}

#[test]
fn shift_and_the_wheel_move_the_view_and_never_the_page() {
    let mut h = app();
    wheel(&mut h, 40., true);
    assert_eq!(h.text("state"), "0 40");
    wheel(&mut h, 2000., true);
    pause(&mut h);
    wheel(&mut h, 40., true);
    assert_eq!(h.text("state"), "0 800");
}

#[test]
fn a_trackpads_swipe_down_is_never_taken_for_a_wheel() {
    let mut h = app();
    point(&mut h);
    swipe(&mut h, 0., 40.);
    assert_eq!(h.text("state"), "40 0");
}

/// A turn is a distance, not one scroll's: a gesture reported in many small
/// steps (a fast display's) turns as soon as one reported in a few large
/// ones does, and drift that comes and goes never adds up to one.
#[test]
fn a_turn_is_as_far_whatever_the_rate_and_drift_never_adds_up_to_one() {
    for (steps, dy) in [(1, 9.), (9, 1.), (18, 0.5)] {
        let mut h = app();
        swipe(&mut h, 20., 0.);
        for _ in 0..steps {
            swipe(&mut h, 0., dy);
        }
        // Gone 9 points down in a row: a turn, the page's from then (one
        // scroll's 8 points each would never have turned at 1 or 0.5).
        swipe(&mut h, 0., 10.);
        let state = h.text("state");
        let (page, across) = state.split_once(' ').unwrap();
        assert!(
            page.parse::<f32>().unwrap() >= 10.,
            "{steps} steps of {dy}: {state}"
        );
        assert_eq!(across, "20", "{steps} steps of {dy}");
    }
    let mut h = app();
    swipe(&mut h, 20., 0.);
    // Down a little, then along again, many times over: still the view's.
    for _ in 0..10 {
        swipe(&mut h, 0., 5.);
        swipe(&mut h, 4., 1.);
    }
    assert_eq!(h.text("state"), "0 60");
}

/// In a page around a plain `scroll_x`, as in AppKit's nested scroll views, a
/// diagonal swipe goes the way it mostly moves: mostly sideways, the view
/// takes all of it and the page none of its drift down; mostly down, the page
/// takes the gesture.
#[test]
fn a_diagonal_swipe_in_nested_plain_scroll_views_goes_the_way_it_mostly_moves() {
    let mut h = plain();
    for _ in 0..4 {
        swipe(&mut h, 10., 4.);
    }
    assert_eq!(h.text("state"), "0 40", "the drift down is the view's");
    pause(&mut h);
    let mut h = plain();
    for _ in 0..3 {
        swipe(&mut h, 4., 10.);
    }
    let state = h.text("state");
    assert!(state.starts_with("30 "), "{state}");
}
