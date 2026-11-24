//! An `on_drag` returning nil starts no drag from that press: a recycled row
//! that drags only some of what it shows. The press stays a press (let go
//! on the row, it clicks), and no target hears a drag carrying nil.
mod common;

use common::Harness;
use zgx::input::{InputEvent, PointerButton};

const APP: &str = r##"
local log = ui.state("log", "")
local function note(s) log:set(log:peek() .. s .. ";") end
local presses = 0
return ui.column { w_full = true, h_full = true,
    ui.text { id = "log", "{log}" },
    ui.div {
        id = "row", w = 200, h = 40,
        on_click = function() note("click") end,
        on_drag = function()
            presses += 1
            note("asked")
            -- The first two presses are on rows it doesn't drag.
            if presses <= 2 then return nil end
            return { kind = "row" }
        end,
        ui.text { "Row" },
    },
    ui.div {
        id = "target", w = 200, h = 80,
        on_drag_over = function(payload) note("over:" .. tostring(payload.kind)) end,
        on_drop = function(payload) note("drop:" .. tostring(payload.kind)) end,
    },
}
"##;

/// Pressed in the row's middle, moved `dx` across and `dy` down in steps,
/// and let go there.
fn press_and_move(h: &mut Harness, dx: f32, dy: f32) {
    let b = {
        let node = h.find("row").unwrap();
        h.ui.scene.borrow().bounds(node)
    };
    let (x, y) = (b.x + b.width / 2., b.y + b.height / 2.);
    let button = PointerButton::Primary;
    h.ui.dispatch(InputEvent::PointerMove { x, y });
    h.ui.dispatch(InputEvent::PointerDown { x, y, button });
    h.frames(1);
    for step in 1..=5 {
        let t = step as f32 / 5.;
        h.ui.dispatch(InputEvent::PointerMove {
            x: x + dx * t,
            y: y + dy * t,
        });
        h.frames(1);
    }
    h.ui.dispatch(InputEvent::PointerUp {
        x: x + dx,
        y: y + dy,
        button,
    });
    h.frames(1);
}

#[test]
fn a_nil_from_on_drag_starts_no_drag_and_the_next_press_drags() {
    let mut h = Harness::inline(APP);
    h.ui.scene.borrow_mut().resize(400., 300.);
    h.frames(3);
    // Declined, let go on the row: no drag, a click.
    press_and_move(&mut h, 30., 0.);
    assert!(!h.ui.input.is_dragging());
    assert_eq!(h.text("log"), "asked;click;");
    // Declined, moved onto the target: it hears nothing.
    press_and_move(&mut h, 0., 60.);
    assert_eq!(h.text("log"), "asked;click;asked;");
    // The next press drags its payload onto the target.
    press_and_move(&mut h, 0., 60.);
    let log = h.text("log");
    assert!(
        log.starts_with("asked;click;asked;asked;over:row;") && log.ends_with("drop:row;"),
        "{log}"
    );
}
