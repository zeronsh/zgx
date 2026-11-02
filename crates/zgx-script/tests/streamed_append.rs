//! `state:append` streaming into a long text an editor is bound to: each
//! token is added where the text ends, and the editor applies it alone,
//! never comparing the document.
mod common;
use common::Harness;
use zgx::text_edit::TextEditor;

const APP: &str = r##"
-- A reply already past a long text's size: held in chunks.
local reply = ui.state("reply", string.rep("a line of what came before\n", 3000))
local n = 0
ui.command { id = "token", run = function()
    n += 1
    reply:append(if n % 40 == 0 then "\n" else "tok" .. n .. " ")
end }
return ui.textarea { id = "reply", value = reply, w = 400, h = 300 }
"##;

#[test]
fn ten_thousand_appends_reach_a_bound_editor_without_comparing_the_document() {
    let mut h = Harness::inline(APP);
    h.frames(1);
    assert_eq!(h.error(), None);
    let compares = TextEditor::whole_compares();
    let mut expected = "a line of what came before\n".repeat(3000);
    for n in 1..=10_000 {
        assert!(h.host.run_command("token"));
        expected.push_str(&if n % 40 == 0 {
            "\n".to_string()
        } else {
            format!("tok{n} ")
        });
    }
    h.frames(1);
    assert_eq!(
        TextEditor::whole_compares(),
        compares,
        "every token applied where it is"
    );
    let node = h.find("reply").unwrap();
    let editor = h.ui.editor(node).expect("the text area's editor");
    assert_eq!(editor.editor.borrow().text(), expected);
}

#[test]
fn a_short_reply_growing_long_is_followed_from_then_on() {
    let mut h = Harness::inline(
        r##"
local reply = ui.state("reply", "")
ui.command { id = "chunk", run = function() reply:append(string.rep("x", 1000)) end }
return ui.textarea { id = "reply", value = reply, w = 400, h = 300 }
"##,
    );
    h.frames(1);
    // Short: a string; past 64 KiB, a long text.
    for _ in 0..70 {
        assert!(h.host.run_command("chunk"));
    }
    let compares = TextEditor::whole_compares();
    for _ in 0..30 {
        assert!(h.host.run_command("chunk"));
    }
    h.frames(1);
    assert_eq!(TextEditor::whole_compares(), compares, "long now: followed");
    let node = h.find("reply").unwrap();
    let editor = h.ui.editor(node).unwrap();
    assert_eq!(editor.editor.borrow().len(), 100_000);
}
