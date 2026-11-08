//! Luau's pointer lock: `el:lock_pointer()` in a press's handler, the
//! mouse's motion to `on_pointer_move` as `dx`, `dy`, `on_pointer_lock`
//! told of each change, and Escape letting it go without reaching `on_key`.
mod common;
use common::Harness;
use zgx::input::{InputEvent, Key, Modifiers, PointerButton};

const APP: &str = r#"
local yaw = ui.state("yaw", 0)
local playing = ui.state("playing", false)
local keys = ui.state("keys", 0)
local moved = ui.state("moved", "")
local view = ui.ref()
return ui.column {
    ui.text { id = "state", function()
        return `{playing:get()} {yaw:get()} {keys:get()} {moved:get()}`
    end },
    ui.div {
        ref = view,
        w = 200,
        h = 100,
        focusable = true,
        on_press = function()
            local el = view:get()
            if el and not playing:peek() then
                el:lock_pointer()
            end
        end,
        on_pointer_lock = function(locked)
            playing:set(locked)
        end,
        on_pointer_move = function(e)
            if e.locked then
                yaw:set(yaw:peek() + e.dx)
            else
                moved:set(`{e.dx},{e.dy}`)
            end
        end,
        on_key = function()
            keys:set(keys:peek() + 1)
            return true
        end,
    },
}
"#;

/// A press and release, as a host hands them on (`note_gesture` first).
fn press(h: &mut Harness) {
    let button = PointerButton::Primary;
    h.ui.input.note_gesture();
    h.ui.dispatch(InputEvent::PointerDown {
        x: 50.,
        y: 60.,
        button,
    });
    h.ui.input.note_gesture();
    h.ui.dispatch(InputEvent::PointerUp {
        x: 50.,
        y: 60.,
        button,
    });
    h.frames(2);
}

fn key(h: &mut Harness, key: Key) {
    h.ui.input.note_gesture();
    h.ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    h.frames(2);
}

#[test]
fn a_press_locks_the_pointer_and_its_motion_turns_the_view() {
    let mut h = Harness::inline(APP);
    h.frames(2);
    assert_eq!(h.text("state"), "false 0 0 ");
    // A move tells how far it went since the last.
    h.ui.dispatch(InputEvent::PointerMove { x: 40., y: 50. });
    h.ui.dispatch(InputEvent::PointerMove { x: 45., y: 48. });
    h.frames(2);
    assert_eq!(h.text("state"), "false 0 0 5,-2");
    // Motion without a lock goes nowhere.
    h.ui.dispatch(InputEvent::PointerMotion { dx: 9., dy: 0. });
    h.frames(2);
    assert_eq!(h.text("state"), "false 0 0 5,-2");
    press(&mut h);
    assert_eq!(h.text("state"), "true 0 0 5,-2");
    h.ui.dispatch(InputEvent::PointerMotion { dx: 7., dy: 1. });
    h.ui.dispatch(InputEvent::PointerMotion { dx: -2., dy: 0. });
    h.frames(2);
    assert_eq!(h.text("state"), "true 5 0 5,-2");
    // Escape lets it go, and its `on_key` never hears it.
    key(&mut h, Key::Escape);
    assert_eq!(h.text("state"), "false 5 0 5,-2");
    // Then keys are its own again.
    key(&mut h, Key::Escape);
    assert_eq!(h.text("state"), "false 5 1 5,-2");
    assert_eq!(h.error(), None);
}

#[test]
fn nothing_locks_the_pointer_without_a_press_or_a_key() {
    let mut h = Harness::inline(
        r#"
local held = ui.state("held", "none")
local view = ui.ref()
ui.after(0, function()
    local el = view:get()
    held:set(tostring(el and el:lock_pointer()))
end)
return ui.column {
    ui.text { id = "held", function() return held:get() end },
    ui.div { ref = view, w = 100, h = 100 },
}
"#,
    );
    h.frames(4);
    assert_eq!(h.text("held"), "false");
    assert_eq!(h.error(), None);
}
