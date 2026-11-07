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

const OVER: (f32, f32) = (100., 50.);

fn app() -> Harness {
    let mut h = Harness::inline(APP);
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
    std::thread::sleep(std::time::Duration::from_millis(320));
    point(&mut h);
    wheel(&mut h, -40., false);
    assert_eq!(h.text("state"), "0 0", "at its start, back up is the page's");
    std::thread::sleep(std::time::Duration::from_millis(320));
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
    std::thread::sleep(std::time::Duration::from_millis(320));
    wheel(&mut h, 40., false);
    assert_eq!(h.text("state"), "40 800");
}

#[test]
fn shift_and_the_wheel_move_the_view_and_never_the_page() {
    let mut h = app();
    wheel(&mut h, 40., true);
    assert_eq!(h.text("state"), "0 40");
    wheel(&mut h, 2000., true);
    std::thread::sleep(std::time::Duration::from_millis(320));
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
