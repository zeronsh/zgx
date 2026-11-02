//! A long document streamed into the value an editor is bound to: the
//! editor applies each token where it is, from the edits the text keeps,
//! rather than comparing the document with its own.
use zgx::{
    compose::prelude::*,
    reactive::Signal,
    text::TextValue,
    text_edit::TextEditor,
    widgets::{EditorHandle, Ui},
};

/// A long document, as a model streaming into it starts from.
fn document() -> String {
    (0..4_000)
        .map(|i| format!("line {i} of what came before\n"))
        .collect()
}

fn mounted(ui: &mut Ui, value: &Signal<TextValue>) -> (zgx::compose::ViewHandle, EditorHandle) {
    let view: View = text_area_value("Answer", value.clone())
        .id("answer")
        .size(400., 300.);
    let mounted = ui.mount(view);
    let node = mounted.find("answer").unwrap();
    assert!(ui.input.focus(&ui.scene, Some(node)));
    ui.prepare_frame();
    let editor = ui.focused_editor().unwrap();
    (mounted, editor)
}

/// `token` added to the end of the bound text.
fn append(value: &Signal<TextValue>, token: &str) {
    value.mutate(|text| match text {
        TextValue::Long(text) => {
            text.append(token);
            true
        }
        TextValue::Flat(text) => {
            text.push_str(token);
            true
        }
    });
}

#[test]
fn tokens_streamed_into_a_bound_document_are_applied_without_comparing_it() {
    let mut ui = Ui::new(500., 400.);
    let value = ui.signal(TextValue::from(document().as_str()));
    assert!(
        value.with_untracked(|v| v.as_text().is_some()),
        "a long text"
    );
    let (mounted, editor) = mounted(&mut ui, &value);
    let compares = TextEditor::whole_compares();
    let mut expected = document();
    for i in 0..2_000 {
        let token = if i % 50 == 49 {
            "\n".to_string()
        } else {
            format!("t{i} ")
        };
        append(&value, &token);
        expected.push_str(&token);
        ui.prepare_frame();
    }
    assert_eq!(
        TextEditor::whole_compares(),
        compares,
        "no token compared the document with the editor's"
    );
    assert_eq!(editor.editor.borrow().text(), expected);
    // One text for both still, not a copy each.
    let ours = editor.editor.borrow().value();
    assert!(value.with_untracked(|v| v.ptr_eq(&ours)));
    mounted.unmount();
}

#[test]
fn more_edits_than_a_text_keeps_are_taken_whole_and_the_stream_goes_on() {
    let mut ui = Ui::new(500., 400.);
    let value = ui.signal(TextValue::from(document().as_str()));
    let (mounted, editor) = mounted(&mut ui, &value);
    let mut expected = document();
    // Forty edits made before the editor sees any: past the sixteen a
    // text keeps.
    let mut text = value.with_untracked(|v| v.as_text().cloned()).unwrap();
    for i in 0..40 {
        let token = format!("burst{i} ");
        text.append(&token);
        expected.push_str(&token);
    }
    let compares = TextEditor::whole_compares();
    value.set(TextValue::Long(text));
    ui.prepare_frame();
    assert_eq!(
        TextEditor::whole_compares(),
        compares + 1,
        "taken whole, once"
    );
    assert_eq!(editor.editor.borrow().text(), expected);
    // And followed again from there.
    let compares = TextEditor::whole_compares();
    for i in 0..10 {
        let token = format!("after{i} ");
        append(&value, &token);
        expected.push_str(&token);
        ui.prepare_frame();
    }
    assert_eq!(TextEditor::whole_compares(), compares);
    assert_eq!(editor.editor.borrow().text(), expected);
    mounted.unmount();
}

#[test]
fn a_return_and_a_line_break_streamed_apart_are_one_break() {
    let mut ui = Ui::new(500., 400.);
    let value = ui.signal(TextValue::from(document().as_str()));
    let (mounted, editor) = mounted(&mut ui, &value);
    append(&value, "end of a line\r");
    ui.prepare_frame();
    append(&value, "\nnext line");
    ui.prepare_frame();
    let text = editor.editor.borrow().text().into_owned();
    assert!(
        text.ends_with("end of a line\nnext line"),
        "one break: {:?}",
        &text[text.len() - 30..]
    );
    mounted.unmount();
}

#[test]
fn a_caret_at_the_end_follows_what_streams_in() {
    let mut ui = Ui::new(500., 400.);
    let value = ui.signal(TextValue::from(document().as_str()));
    let (mounted, editor) = mounted(&mut ui, &value);
    let end = editor.editor.borrow().len();
    editor.editor.borrow_mut().set_selection(end, end);
    for token in ["more ", "and ", "more"] {
        append(&value, token);
        ui.prepare_frame();
    }
    let editor = editor.editor.borrow();
    assert_eq!(editor.selection().focus, editor.len(), "still at the end");
    mounted.unmount();
}
