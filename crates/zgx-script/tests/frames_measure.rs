//! `ui.frames`: an action's frames measured as the web measures a tab
//! switch, and a gesture's from `begin` to `finish`, on the app's frames.
mod common;
use common::Harness;

/// The app: a button that measures a state change, writing what it heard.
const APP: &str = r#"
local said = ui.state("said", "")
local n = ui.state("n", 0)
local function show(m)
    said:set(table.concat({
        "frames=" .. m.frames,
        "long=" .. m.long_frames,
        "duration=" .. tostring(m.duration_ms),
        "settled=" .. tostring(m.settled_ms),
        "blocked=" .. tostring(m.blocked_ms),
        "times=" .. #m.times,
        "start=" .. tostring(m.start_ms > 0),
        "capped=" .. tostring(m.capped),
    }, " "))
end
local gesture = nil
return ui.column {
    ui.button { id = "measure", "Measure", on_click = function()
        ui.frames.measure(function() n:set(n:peek() + 1) end, { window_ms = 200 }):after(show)
    end },
    ui.button { id = "begin", "Begin", on_click = function()
        gesture = ui.frames.begin()
    end },
    ui.button { id = "finish", "Finish", on_click = function()
        gesture:finish():after(show)
    end },
    ui.text { id = "said", said },
    ui.text { id = "n", n },
}
"#;

fn field<'a>(said: &'a str, key: &str) -> &'a str {
    said.split(' ')
        .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
        .unwrap_or_else(|| panic!("{key} in {said:?}"))
}

#[test]
fn a_measure_settles_after_its_window_with_the_webs_fields() {
    let mut h = Harness::inline(APP);
    h.frames(2);
    h.click("measure");
    assert!(h.wait_until(|h| !h.text("said").is_empty()), "it settled");
    let said = h.text("said");
    // 200 ms of 60 Hz frames: 12 or 13, none long; the second frame's time.
    let frames: usize = field(&said, "frames").parse().unwrap();
    assert!((12..=14).contains(&frames), "{said}");
    assert_eq!(field(&said, "times"), field(&said, "frames"), "{said}");
    assert_eq!(field(&said, "long"), "0", "{said}");
    assert_eq!(field(&said, "duration"), "33", "{said}");
    assert_eq!(field(&said, "settled"), "33", "{said}");
    assert_eq!(field(&said, "blocked"), "0", "{said}");
    assert_eq!(field(&said, "start"), "true", "{said}");
    assert_eq!(field(&said, "capped"), "nil", "{said}");
    assert_eq!(h.text("n"), "1", "the action ran");
}

#[test]
fn a_gesture_records_from_begin_to_finish() {
    let mut h = Harness::inline(APP);
    h.frames(2);
    h.click("begin");
    h.frames(30);
    h.click("finish");
    assert!(h.wait_until(|h| !h.text("said").is_empty()), "it settled");
    let said = h.text("said");
    let frames: usize = field(&said, "frames").parse().unwrap();
    assert!((29..=32).contains(&frames), "{said}");
    assert_eq!(field(&said, "long"), "0", "{said}");
    assert_eq!(field(&said, "duration"), "33", "{said}");
}

#[test]
fn nothing_measures_and_no_frame_is_asked_for_while_none_runs() {
    let mut h = Harness::inline(APP);
    h.frames(2);
    h.click("measure");
    assert!(h.wait_until(|h| !h.text("said").is_empty()), "it settled");
    // Ended: no task waits for a frame, so the app is at rest again.
    h.frames(2);
    assert!(!h.wants_frame(), "a settled measure asks for no frames");
}

#[test]
fn a_window_that_is_no_time_is_an_error() {
    let error = zgx_script::testing::TestApp::try_new(|runtime| {
        zgx_script::ScriptHost::inline(
            runtime,
            "app",
            "ui.frames.measure(function() end, { window_ms = 0 })\nreturn ui.column {}\n",
        )
    })
    .err()
    .expect("it fails to load");
    assert!(
        error.contains("`window_ms` is a number of milliseconds above 0"),
        "{error}"
    );
}

#[test]
fn a_frame_late_by_more_than_50_ms_is_long_and_blocks() {
    let mut h = Harness::inline(APP);
    h.frames(2);
    h.click("begin");
    h.frames(10);
    // Six refreshes missed: one gap of 7 frames' time (117 ms).
    h.skip_frames(6);
    h.frames(10);
    h.click("finish");
    assert!(h.wait_until(|h| !h.text("said").is_empty()), "it settled");
    let said = h.text("said");
    assert_eq!(field(&said, "long"), "1", "{said}");
    let blocked: f64 = field(&said, "blocked").parse().unwrap();
    assert!((100. ..=130.).contains(&blocked), "{said}");
    let settled: f64 = field(&said, "settled").parse().unwrap();
    assert!(settled > 150., "settled at the long gap's end: {said}");
}

#[test]
fn a_new_measure_ends_the_last_with_its_duration_alone() {
    let mut h = Harness::inline(APP);
    h.frames(2);
    h.click("measure");
    h.frames(3);
    // The next action's measure starts: the first ends now, short of its
    // window, as the web's next switch ends its last.
    h.click("measure");
    assert!(
        h.wait_until(|h| h.text("said").contains("settled=nil")),
        "{}",
        h.text("said")
    );
    let said = h.text("said");
    assert_eq!(field(&said, "duration"), "33", "{said}");
    assert_eq!(field(&said, "blocked"), "nil", "{said}");
}
