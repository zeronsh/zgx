//! A textarea's `atom_style` as a mention's look (colour, weight,
//! underline, leading icon) and an atom's own `style` over it, read back
//! as given.
mod common;
use common::Harness;
use zgx::scene::{Color, NodeKind};

const APP: &str = r##"
local dot = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="8"/></svg>'
local atoms = ui.state("atoms", {
    { id = "ada", from = 4, to = 16 },
    { id = "doc", from = 23, to = 31, style = { icon = { code = dot, size = 12, gap = 2 } } },
})
local log = ui.state("log", "")
return ui.column {
    ui.textarea {
        id = "field", label = "Message", value = ui.state("text", "ask Ada Lovelace about the plan.md"),
        atoms = atoms, w = 380, h = 80,
        atom_style = {
            color = "#1c1c1c", weight = 500,
            underline = { color = "rgba(28, 28, 28, 0.36)", offset = 2 },
            icon = { code = dot, size = 16, gap = 4 },
        },
    },
    ui.button { id = "read", w = 10, h = 10, on_click = function()
        local items = atoms:get()
        log:set(`{items[2].style.icon.size} {items[1].style == nil}`)
    end },
    ui.text { id = "log", "{log}" },
}
"##;

#[test]
fn a_mention_look_and_an_atoms_own_icon_show() {
    let mut h = Harness::inline(APP);
    h.frames(3);
    let node = h.find("field").unwrap();
    let editor = h.ui.editor(node).unwrap();
    assert_eq!(editor.atoms().len(), 2);
    // Icons drawn: one each, the second its own size.
    let scene = h.ui.scene.borrow();
    let mut stack = vec![node];
    let mut sizes = Vec::new();
    let mut weights = Vec::new();
    while let Some(id) = stack.pop() {
        match scene.kind(id) {
            NodeKind::Svg(_) => sizes.push(scene.transformed_bounds(id).width),
            NodeKind::RichText { text } => {
                for run in text.runs() {
                    if run.color == Color(28, 28, 28, 255) {
                        weights.push(run.font.weight);
                    }
                }
            }
            _ => {}
        }
        stack.extend(scene.children(id).iter().copied());
    }
    drop(scene);
    sizes.sort_by(f32::total_cmp);
    assert_eq!(sizes, [12., 16.]);
    assert!(
        !weights.is_empty() && weights.iter().all(|w| *w == 500),
        "{weights:?}"
    );
    // Read back as given.
    h.click("read");
    assert_eq!(h.text("log"), "12 true");
}
