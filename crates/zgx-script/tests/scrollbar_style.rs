//! `scrollbar = { … }`: a scroll view's or a list's own scrollbar from a
//! script, drawn and timed in Rust; its `hint` and `track` read as they
//! change, never as the view scrolls.
mod common;

use common::Harness;
use std::time::{Duration, Instant};
use zgx::{input::InputEvent, scene::NodeKind, semantics::Role};

const RAIL: &str = r#"{ platform = false, thickness = 3, active_thickness = 5, strip = 10,
    side = 2, ends = 4, inset = 0, min = 24, linger = 1400, fade = 260, fade_in = true,
    ease = "outQuad", active_color = true, track_fill = false, press = "jump",
    linger_from = "scroll", focusable = false, always_overlay = true"#;

fn bar(h: &Harness) -> zgx::scene::NodeId {
    h.ui.semantics
        .borrow()
        .iter()
        .find(|(_, n)| n.role == Role::ScrollBar)
        .unwrap()
        .0
}

/// The thumb's box as drawn.
fn thumb(h: &Harness) -> zgx::scene::Rect {
    let bar = bar(h);
    let scene = h.ui.scene.borrow();
    let thumb = *scene.children(bar).last().unwrap();
    assert!(matches!(scene.kind(thumb), NodeKind::Quad(_)));
    scene.bounds(thumb)
}

fn wheel(h: &mut Harness) {
    h.ui.dispatch(InputEvent::Scroll {
        x: 50.,
        y: 50.,
        delta_x: 0.,
        delta_y: 3.,
    });
    h.ui.prepare_frame();
}

#[test]
fn a_scripts_rail_is_drawn_as_it_says() {
    let mut h = Harness::inline(&format!(
        r#"return ui.scroll {{ w = 200, h = 100, scrollbar = {RAIL} }},
            ui.div {{ h = 400 }} }}"#
    ));
    h.ui.prepare_frame();
    assert_eq!(h.host.error().get(), None);
    assert_eq!(thumb(&h).height, 0., "hidden until scrolled");
    wheel(&mut h);
    h.ui.advance_interactions(Instant::now() + Duration::from_millis(300))
        .unwrap();
    h.ui.prepare_frame();
    let thumb = thumb(&h);
    assert_eq!(thumb.width, 3.);
    assert_eq!(thumb.x + thumb.width, 198.);
    assert_eq!(thumb.height, 24.);
}

#[test]
fn its_hint_and_track_are_read_as_they_change_not_as_it_scrolls() {
    let mut h = Harness::inline(&format!(
        r#"local hinted = ui.state("hinted", false)
        local reads = ui.state("reads", 0)
        local rail = {RAIL},
            hint = function() reads:update(function(n) return n + 1 end) return hinted:get() end,
            track = function() return 60 end }}
        return ui.column {{
            ui.text {{ id = "reads", function() return tostring(reads:get()) end }},
            ui.button {{ id = "hint", on_click = function() hinted:set(true) end, "hint" }},
            ui.list {{ w = 200, h = 100, count = 100, height = 20, scrollbar = rail,
                row = function(i) return ui.text {{ "row " .. i, h = 20 }} end }} }}"#
    ));
    h.ui.prepare_frame();
    assert_eq!(h.host.error().get(), None);
    let before = h.text("reads");
    for _ in 0..20 {
        wheel(&mut h);
    }
    h.ui.prepare_frame();
    assert_eq!(h.text("reads"), before, "scrolling reads no hint");
    h.click("hint");
    h.ui.prepare_frame();
    h.ui.advance_interactions(Instant::now() + Duration::from_millis(300))
        .unwrap();
    h.ui.prepare_frame();
    let thumb = thumb(&h);
    assert!(thumb.height > 0., "the hint shows it");
    let track = h.ui.scene.borrow().bounds(bar(&h));
    assert_eq!(track.height, 60., "its own track");
    assert!(
        thumb.y + thumb.height <= track.y + 56.01,
        "within it, 4 short of its end"
    );
}

#[test]
fn a_scrollbar_table_says_what_it_has_no_place_for() {
    let error = zgx_script::testing::TestApp::try_new(|runtime| {
        zgx_script::ScriptHost::inline(
            runtime,
            "app",
            r#"return ui.scroll { w = 200, h = 100, scrollbar = { thikness = 3 },
                ui.div { h = 400 } }"#,
        )
    })
    .err()
    .expect("an error");
    assert!(error.contains("`scrollbar` has no `thikness`"), "{error}");
}

#[test]
fn a_test_apps_frames_fade_it_in_on_its_clock() {
    // A test's own time, moved a frame at a time: the frames step the fade
    // as a window's do.
    let clock = zgx::clock::VirtualClock::new();
    let mut h = Harness::inline(&format!(
        r#"return ui.list {{ id = "l", w = 200, h = 100, count = 100, height = 20, scrollbar = {RAIL} }},
            row = function(i) return ui.text {{ "row " .. i, h = 20 }} end }}"#
    ));
    h.frame();
    let l = h.find("l").unwrap();
    zgx_script::driver::scroll(&mut h.ui, l, 8.);
    let alpha = |h: &Harness| {
        let bar = bar(h);
        let scene = h.ui.scene.borrow();
        let thumb = *scene.children(bar).last().unwrap();
        match scene.kind(thumb) {
            NodeKind::Quad(quad) => quad.fill.3,
            _ => 0,
        }
    };
    let mut seen = Vec::new();
    for _ in 0..20 {
        clock.advance(clock.elapsed() + Duration::from_millis(16), || {});
        h.frame();
        seen.push(alpha(&h));
    }
    assert!(seen[0] > 0 && seen[0] < seen[19], "rising: {seen:?}");
    assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    assert_eq!(thumb(&h).height, 24.);
}
