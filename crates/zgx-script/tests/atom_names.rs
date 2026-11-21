//! A textarea's atom's `name`: what assistive technology reads for it
//! instead of its text, given in `atoms` or by `ui.editor.insert_atom`,
//! read back as given and carried to the text area's semantics (each atom
//! one unit there).
mod common;
use common::Harness;

#[test]
fn an_atoms_name_goes_to_assistive_technology_and_reads_back() {
    let mut h = Harness::inline(
        r##"
local draft = ui.state("draft", "ask @que")
local atoms = ui.state("atoms", {})
local box = ui.ref()
ui.command { id = "add", run = function()
    ui.editor.insert_atom(box:get(), "@q.pdf", {
        id = "q", name = "Quarterly report", replace = { from = 4, to = 8 },
    })
end }
return ui.column { w = 400, h = 200,
    ui.textarea { ref = box, id = "ta", value = draft, atoms = atoms, w = 300, h = 80 },
    ui.text { id = "info", function()
        local a = atoms:get()[1]
        return a and `{a.id}:{a.from}-{a.to}:{a.name}` or "none"
    end },
}"##,
    );
    h.settle();
    h.frames(3);
    h.host.run_command("add");
    h.frames(3);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("info"), "q:4-10:Quarterly report");
    let ta = h.find("ta").unwrap();
    let semantics = h.ui.semantics.borrow();
    let units = semantics
        .get(ta)
        .and_then(|node| node.text_units.clone())
        .expect("its atoms, to a reader");
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].range, 4..10);
    assert_eq!(units[0].label.as_deref(), Some("Quarterly report"));
}
