//! A textarea's atoms on the clipboard: copied beside the text with their
//! names, data and looks (`application/x-zgx-atoms`), and made again by a
//! paste into another textarea's `atoms`, from this app or another (its
//! codec reading what the source's wrote).
mod common;
use common::Harness;
use std::cell::RefCell;
use zgx::{
    platform::{FilePicker, Pending, Platform},
    widgets::{ATOMS_TYPE, EditCommand},
};

/// A clipboard in memory, with types beside the text.
#[derive(Default)]
struct Clipboard {
    text: RefCell<Option<String>>,
    types: RefCell<Vec<(String, String)>>,
}
impl Platform for Clipboard {
    fn clipboard_text(&self) -> Option<String> {
        self.text.borrow().clone()
    }
    fn set_clipboard_text(&self, text: String) -> bool {
        *self.text.borrow_mut() = Some(text);
        self.types.borrow_mut().clear();
        true
    }
    fn set_clipboard_typed(
        &self,
        text: String,
        _: Option<String>,
        types: &[(String, String)],
    ) -> bool {
        *self.text.borrow_mut() = Some(text);
        *self.types.borrow_mut() = types.to_vec();
        true
    }
    fn clipboard_typed(&self, mime: &str) -> Option<String> {
        let types = self.types.borrow();
        types
            .iter()
            .find(|(t, _)| t == mime)
            .map(|(_, v)| v.clone())
    }
    fn open_files(&self, _: FilePicker) -> Pending<Result<Vec<std::path::PathBuf>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn save_file(&self, _: FilePicker) -> Pending<Result<Option<std::path::PathBuf>, String>> {
        Box::pin(async { Ok(None) })
    }
}

const APP: &str = r##"
local a = ui.state("a", "see  now")
local b = ui.state("b", "")
local a_atoms = ui.state("a_atoms", {})
local b_atoms = ui.state("b_atoms", {})
local box = ui.ref()
ui.command { id = "add", run = function()
    box:get():set_selection(4, 4)
    ui.editor.insert_atom(box:get(), "@file.pdf", {
        id = "file-1", data = { path = "a/b.pdf" }, style = { bg = "#3366ff40" },
    })
end }
local function info(atoms)
    return function()
        local out = {}
        for _, x in atoms:get() do
            table.insert(out, `{x.id}:{x.from}-{x.to}:{x.data and x.data.path}:{x.style and x.style.bg}`)
        end
        return table.concat(out, ",")
    end
end
return ui.column { w = 400, h = 300,
    ui.textarea { ref = box, id = "a", value = a, atoms = a_atoms, w = 300, h = 80 },
    ui.textarea { id = "b", value = b, atoms = b_atoms, w = 300, h = 80 },
    ui.text { id = "a_info", info(a_atoms) },
    ui.text { id = "b_info", info(b_atoms) },
}"##;

/// The app with an atom in "a", "a" all copied to `clipboard`.
fn copied(clipboard: &Clipboard) -> Harness {
    let mut h = Harness::inline(APP);
    h.settle();
    h.frames(3);
    h.host.run_command("add");
    h.frames(3);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("a_info"), "file-1:4-13:a/b.pdf:#3366ff40");
    let a = h.find("a").unwrap();
    h.ui.input.focus(&h.ui.scene, Some(a));
    h.ui.editor(a).unwrap().set_selection(0, 17);
    assert!(h.ui.edit_command(EditCommand::Copy, clipboard));
    h
}

fn paste_into_b(h: &mut Harness, clipboard: &Clipboard) {
    let b = h.find("b").unwrap();
    h.ui.input.focus(&h.ui.scene, Some(b));
    h.frames(1);
    assert!(h.ui.edit_command(EditCommand::Paste, clipboard));
    h.frames(3);
}

#[test]
fn a_copy_puts_the_atoms_names_data_and_looks_beside_the_text() {
    let clipboard = Clipboard::default();
    let _h = copied(&clipboard);
    let json = clipboard
        .clipboard_typed(ATOMS_TYPE)
        .expect("the atoms type");
    assert!(json.contains(r##""text":"see @file.pdf now""##), "{json}");
    assert!(json.contains(r##""from":4,"to":13"##), "{json}");
    assert!(json.contains(r##""id":"file-1""##), "{json}");
    assert!(json.contains(r##""path":"a/b.pdf""##), "{json}");
    assert!(json.contains(r##""bg":"#3366ff40""##), "{json}");
}

#[test]
fn a_paste_in_the_app_makes_the_atoms_in_the_other_textarea() {
    let clipboard = Clipboard::default();
    let mut h = copied(&clipboard);
    paste_into_b(&mut h, &clipboard);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("b"), "see @file.pdf now");
    assert_eq!(h.text("b_info"), "file-1:4-13:a/b.pdf:#3366ff40");
}

#[test]
fn a_paste_from_another_app_reads_them_back_by_the_codec() {
    let clipboard = Clipboard::default();
    let mut h = copied(&clipboard);
    // As from another window or app: none of this one's own copy kept.
    zgx::widgets::forget_copied_atoms();
    paste_into_b(&mut h, &clipboard);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("b_info"), "file-1:4-13:a/b.pdf:#3366ff40");
    // Again: the name is taken there, so the second has one of its own.
    zgx::widgets::forget_copied_atoms();
    assert!(h.ui.edit_command(EditCommand::Paste, &clipboard));
    h.frames(3);
    let info = h.text("b_info");
    let parts: Vec<&str> = info.split(',').collect();
    assert_eq!(parts.len(), 2, "{info}");
    assert!(parts[1].ends_with(":21-30:a/b.pdf:#3366ff40"), "{info}");
    assert!(!parts[1].starts_with("file-1:"), "{info}");
}
