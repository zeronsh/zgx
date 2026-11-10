//! `ui.map`: a number mapped from a state in Rust, which a style follows
//! with no script run as the state changes (a scrollbar thumb's `top`
//! following an offset). Its ends may be numbers, states or functions,
//! each kept current as what it reads changes.

mod common;

use common::Harness;

const RAIL: &str = r#"
local offset = ui.state("offset", 0)
local track = ui.state("track", 100)
local tick = ui.state("tick", 0)
local calls = 0
local function finish(): number
    calls += 1
    return track:get() - 20
end
return ui.column {
    ui.button { id = "half", on_click = function() offset:set(50) end, "half" },
    ui.button { id = "far", on_click = function() offset:set(500) end, "far" },
    ui.button { id = "back", on_click = function() offset:set(-30) end, "back" },
    ui.button { id = "grow", on_click = function() track:set(140) end, "grow" },
    ui.button { id = "tick", on_click = function() tick:set(tick:peek() + 1) end, "tick" },
    ui.text { id = "calls", function() return `{tick:get()}:{calls}` end },
    ui.column {
        id = "page",
        relative = true,
        w = 200,
        h = 200,
        ui.column {
            id = "rail",
            absolute = true,
            top = 0,
            right = 0,
            w = 10,
            h = 200,
            ui.column {
                id = "thumb",
                absolute = true,
                right = 2,
                w = 4,
                h = 20,
                top = ui.map(offset, { from = { 0, 100 }, to = { 0, finish }, clamp = true }),
            },
            ui.column {
                id = "flat",
                absolute = true,
                left = 0,
                w = 4,
                h = 4,
                top = ui.map(offset, { from = { 10, 10 }, to = { 7, 90 } }),
            },
        },
    },
}
"#;

/// The thumb's place in its rail: its `top`.
fn top(h: &mut Harness, id: &str) -> f32 {
    h.bounds(id).y - h.bounds("rail").y
}

#[test]
fn a_mapped_top_follows_the_state_in_a_line_held_within_its_range() {
    let mut h = Harness::inline(RAIL);
    h.settle();
    assert_eq!(top(&mut h, "thumb"), 0.);
    h.click("half");
    h.settle();
    // Half of 0..100 is half of 0..80.
    assert_eq!(top(&mut h, "thumb"), 40.);
    h.click("far");
    h.settle();
    assert_eq!(top(&mut h, "thumb"), 80., "held at its end");
    h.click("back");
    h.settle();
    assert_eq!(top(&mut h, "thumb"), 0., "held at its start");
}

#[test]
fn a_range_of_no_width_gives_its_start() {
    let mut h = Harness::inline(RAIL);
    h.settle();
    assert_eq!(top(&mut h, "flat"), 7.);
    h.click("half");
    h.settle();
    assert_eq!(top(&mut h, "flat"), 7.);
}

/// A function end runs as what it reads changes, not as the mapped state
/// does: moving the thumb runs no script.
#[test]
fn a_function_end_follows_what_it_reads_and_not_the_state() {
    let mut h = Harness::inline(RAIL);
    h.settle();
    let calls = |h: &mut Harness| {
        h.click("tick");
        h.settle();
        let shown = h.text("calls");
        shown.split(':').nth(1).unwrap().parse::<u32>().unwrap()
    };
    let before = calls(&mut h);
    h.click("half");
    h.settle();
    h.click("far");
    h.settle();
    assert_eq!(calls(&mut h), before, "the offset moving ran no script");
    h.click("grow");
    h.settle();
    assert_eq!(
        calls(&mut h),
        before + 1,
        "the track changing ran its end once"
    );
    // Far past 100: its end, now 120.
    assert_eq!(top(&mut h, "thumb"), 120.);
}

#[test]
fn only_a_number_takes_a_map() {
    let code = r#"local offset = ui.state("offset", 0)
        return ui.column { color = ui.map(offset, { from = { 0, 1 }, to = { 0, 1 } }) }"#;
    let error = match zgx_script::testing::TestApp::try_new(|runtime| {
        zgx_script::ScriptHost::inline(runtime, "app", code)
    }) {
        Ok(_) => panic!("a colour took a map"),
        Err(error) => error,
    };
    assert!(error.contains("ui.map gives one"), "{error}");
}
