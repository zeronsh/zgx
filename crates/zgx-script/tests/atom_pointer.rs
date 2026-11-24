//! A textarea's `on_atom_press` and `on_atom_hover`: a chip pressed in
//! place (taken: the caret stays out of it), and the chip under the pointer
//! with its box, told as the script's atom items.
mod common;
use common::Harness;
use zgx::input::{InputEvent, PointerButton};

const APP: &str = r##"
local text = ui.state("text", "see @file.pdf now")
local atoms = ui.state("atoms", { { id = "f1", from = 4, to = 13, data = { kind = "file" } } })
local log = ui.state("log", "")
local function add(line: string)
    log:set(log:peek() .. line .. ";")
end
return ui.column {
    ui.textarea {
        id = "field", label = "Notes", value = text, atoms = atoms, w = 300, h = 80,
        on_atom_press = function(atom, at)
            add(`press {atom.id} {atom.from}-{atom.to} {atom.data.kind} {at.x > 0}`)
            return true
        end,
        on_atom_hover = function(atom, box)
            if atom then
                add(`hover {atom.id} {box.width > 0 and box.height > 0}`)
            else
                add(`leave {box == nil}`)
            end
        end,
    },
    ui.text { id = "log", "{log}" },
}
"##;

#[test]
fn a_press_and_a_hover_reach_the_script() {
    let mut h = Harness::inline(APP);
    h.frames(2);
    let node = h.find("field").unwrap();
    let editor = h.ui.editor(node).unwrap();
    let atom = editor.atoms().into_iter().next().expect("the chip");
    let area = editor.range_boxes(atom.range)[0];
    let (x, y) = (area.x + area.width / 2., area.y + area.height / 2.);
    h.ui.input.focus(&h.ui.scene, Some(node));
    editor.set_selection(0, 0);
    h.frames(1);
    h.ui.dispatch(InputEvent::PointerMove { x, y });
    h.frames(1);
    h.ui.dispatch(InputEvent::PointerDown {
        x,
        y,
        button: PointerButton::Primary,
    });
    h.ui.dispatch(InputEvent::PointerUp {
        x,
        y,
        button: PointerButton::Primary,
    });
    h.frames(1);
    // Off the chip, still over the textarea.
    h.ui.dispatch(InputEvent::PointerMove {
        x: area.x + area.width + 20.,
        y,
    });
    h.frames(1);
    assert_eq!(
        h.text("log"),
        "hover f1 true;press f1 4-13 file true;leave true;"
    );
    assert_eq!(editor.selection(), (0, 0), "taken: the caret stays out");
}

#[test]
fn without_the_props_a_press_places_the_caret() {
    let mut h = Harness::inline(
        r##"return ui.textarea { id = "field", label = "Notes", value = ui.state("t", "see @file.pdf now"),
            atoms = ui.state("a", { { id = "f1", from = 4, to = 13 } }), w = 300, h = 80 }"##,
    );
    h.frames(2);
    let node = h.find("field").unwrap();
    let editor = h.ui.editor(node).unwrap();
    let atom = editor.atoms().into_iter().next().expect("the chip");
    let area = editor.range_boxes(atom.range)[0];
    let (x, y) = (area.x + area.width / 2., area.y + area.height / 2.);
    h.ui.dispatch(InputEvent::PointerDown {
        x,
        y,
        button: PointerButton::Primary,
    });
    h.ui.dispatch(InputEvent::PointerUp {
        x,
        y,
        button: PointerButton::Primary,
    });
    h.frames(1);
    let (anchor, focus) = editor.selection();
    assert!(
        anchor == focus && (4..=13).contains(&focus),
        "{anchor} {focus}"
    );
}
