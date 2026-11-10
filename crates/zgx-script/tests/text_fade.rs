//! A span's text fading in (`fade_in`): clear at its start, its own colour
//! once it has faded, in between on the way, and drawn a frame at a time in
//! Rust: no script runs for it while it fades, and the paragraph doesn't lay
//! out again. (Its time is the wall clock's, as `ui.now()`'s is: frames
//! here are paced with real time between them.)

use std::time::Duration;

use zgx::scene::{Color, NodeKind};
use zgx_script::testing::TestApp;

/// The alpha of each run of the paragraph holding `words`.
fn alphas(app: &mut TestApp, words: &str) -> Vec<(String, u8)> {
    let node = app
        .find(&format!("~\"{words}\""))
        .unwrap_or_else(|| panic!("no {words}:\n{}", app.tree()));
    let scene = app.ui.scene.borrow();
    let mut stack = vec![node];
    while let Some(id) = stack.pop() {
        if let NodeKind::RichText { text } = scene.kind(id) {
            return text
                .runs()
                .iter()
                .map(|run| (text.text()[run.range.clone()].to_owned(), run.color.3))
                .collect();
        }
        stack.extend(scene.children(id).iter().rev().copied());
    }
    panic!("no rich text under {words}");
}

fn paced(app: &TestApp, frames: usize) {
    for _ in 0..frames {
        std::thread::sleep(Duration::from_millis(16));
        app.frames(1);
    }
}

const SCRIPT: &str = r##"
local started = ui.state("started", 0)
started:set(ui.now())
return ui.rich {
    ui.span { "Shown already. ", color = "#ff000080" },
    ui.span { "Fading in.", color = "#ff000080", fade_in = { start = ui.now(), ms = 300, power = 1.6 } },
}
"##;

#[test]
fn a_span_fades_in_from_clear_to_its_colour_with_no_script_per_frame() {
    let mut app = TestApp::inline(SCRIPT);
    app.ui.scene.borrow_mut().resize(600., 200.);
    app.frames(1);
    assert_eq!(app.error(), None);
    let first = alphas(&mut app, "Fading in.");
    assert_eq!(
        first[0],
        ("Shown already. ".into(), 0x80),
        "a span without a fade is as it was: {first:?}"
    );
    assert!(
        first[1].1 < 0x40,
        "the fading span starts near clear: {first:?}"
    );

    // On its way: between clear and its own alpha, with no script call.
    let calls = app.calls();
    paced(&app, 8);
    let between = alphas(&mut app, "Fading in.");
    assert!(
        between[1].1 > first[1].1 && between[1].1 < 0x80,
        "on its way in: {between:?}"
    );
    assert_eq!(app.calls(), calls, "no script runs while it fades");

    // Done: its own colour, and no frames asked for after.
    paced(&app, 20);
    let done = alphas(&mut app, "Fading in.");
    assert_eq!(done[1].1, 0x80, "faded in to its colour: {done:?}");
    assert_eq!(app.calls(), calls, "no script ran for the whole fade");
    app.frames(2);
    assert!(
        !app.clock.wants_frame(),
        "a paragraph done fading asks for no frames"
    );
    let _ = Color(0, 0, 0, 0);
}

/// A span with room after it unlike the room before (`padding = { 2, 12, 0 }`)
/// is drawn as its room, then its text, then the room after: its text's run
/// is the one that fades, and its room runs keep their colour.
#[test]
fn a_span_with_room_apart_fades_its_text_run() {
    let mut app = TestApp::inline(
        r##"
return ui.rich {
    ui.span { "Before. ", color = "#ff000080" },
    ui.span { "Fading in.", color = "#ff000080", padding = { 2, 12, 0 }, fade_in = { start = ui.now(), ms = 300 } },
    ui.span { " After.", color = "#ff000080" },
}
"##,
    );
    app.ui.scene.borrow_mut().resize(600., 200.);
    app.frames(1);
    assert_eq!(app.error(), None);
    let runs = alphas(&mut app, "Fading in.");
    assert!(
        runs.len() > 3,
        "the span's room is runs of its own: {runs:?}"
    );
    for (text, alpha) in &runs {
        if text == "Fading in." {
            assert!(*alpha < 0x40, "its text starts near clear: {runs:?}");
        } else {
            assert_eq!(*alpha, 0x80, "{text:?} doesn't fade: {runs:?}");
        }
    }
}
