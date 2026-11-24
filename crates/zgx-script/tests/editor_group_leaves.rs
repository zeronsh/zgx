//! Lines shown leave their scroll group (`LineInset::scroll_group`) by an
//! edit: a code block's fence or the block deleted, undone and redone. The
//! group's layer goes and its lines show as any other, where they are.
mod common;
use common::Harness;
use zgx::{
    input::{InputEvent, Key, Modifiers},
    scene::{NodeId, NodeKind},
    widgets::{EditorHandle, Ui},
};

/// Each line shown: its text, and where it is in the window.
type Shown = Vec<(String, f32, f32)>;
fn shown(ui: &Ui, editor: &EditorHandle) -> Shown {
    let scene = ui.scene.borrow();
    let area = scene.transformed_bounds(editor.node);
    let mut pending: Vec<NodeId> = vec![editor.node];
    let mut found = Vec::new();
    while let Some(id) = pending.pop() {
        if let NodeKind::Text { text, .. } = scene.kind(id)
            && !text.is_empty()
        {
            let r = scene.transformed_bounds(id);
            if r.y < area.y + area.height && r.y + r.height > area.y {
                found.push((text.to_string(), r.x, r.y));
            }
        }
        pending.extend_from_slice(scene.children(id));
    }
    found.sort_by(|a, b| a.2.total_cmp(&b.2));
    found
}
/// The lines at `y` on, a row (20 points) each, at their `x`.
fn rows(y: f32, lines: &[(&str, f32)]) -> Shown {
    (lines.iter().enumerate())
        .map(|(i, (text, x))| (text.to_string(), *x, y + 20. * i as f32))
        .collect()
}
/// What shows is `lines`, a row each, and each line's first character is
/// where it is drawn.
#[track_caller]
fn check(ui: &Ui, editor: &EditorHandle, lines: &[(&str, f32)]) {
    let drawn = shown(ui, editor);
    assert_eq!(drawn, rows(0., lines));
    let mut start = 0;
    let mut drawn = drawn.into_iter();
    let text = editor.editor.borrow().text().to_string();
    for line in text.split('\n') {
        if !line.is_empty()
            && let Some((shown, x, y)) = drawn.next()
        {
            assert_eq!(shown, line);
            let first = editor.range_boxes(start..start + 1)[0];
            assert_eq!((first.x, first.y), (x, y), "{line:?}'s first character");
        }
        start += line.len() + 1;
    }
}
/// The quads drawn over the lines: with no marks, its group scrollbar's.
fn bar(ui: &Ui, editor: &EditorHandle) -> Vec<(f32, f32)> {
    let scene = ui.scene.borrow();
    let mut pending: Vec<NodeId> = vec![editor.node];
    let mut found = Vec::new();
    while let Some(id) = pending.pop() {
        let r = scene.transformed_bounds(id);
        if matches!(scene.kind(id), NodeKind::Quad(_))
            && r.width > 0.
            && r.height > 0.
            && scene.layout_visible(id)
        {
            found.push((r.x, r.y));
        }
        pending.extend_from_slice(scene.children(id));
    }
    found
}
fn press(ui: &mut Ui, key: Key) {
    ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers: Modifiers::default(),
        repeat: false,
    });
}
const WIDE: &str = "cxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

/// A fence's lines in group 7; `c` lines in group 1 and `g` lines in group 2.
const FENCED: &str = r#"state == "code" and line ~= "```" and 7"#;
const BY_TEXT: &str = r#"({ c = 1, g = 2 })[string.sub(line, 1, 1)]"#;
/// A note 300 by 200 of `text`, each line in the group `group` says 30 in
/// and unwrapped.
fn note(text: &str, group: &str, scrollbar: bool) -> (Harness, EditorHandle) {
    let mut h = Harness::inline(&format!(
        r##"
        return ui.textarea {{ id = "note", value = ui.state("text", {text:?}),
            w = 300, h = 200, p = 0, text_size = 10, line_height = 20, wrap = true,
            group_scrollbar = {scrollbar},
            highlight = function(line, state)
                if line == "```" then
                    return {{}}, if state == "code" then 0 else "code"
                end
                return {{}}, state
            end,
            line_insets = function(line, state)
                local group = {group}
                if group then
                    return {{ left = 30, group = group, nowrap = true }}
                end
                return nil
            end }}
    "##,
    ));
    frame(&mut h);
    assert_eq!(h.error(), None);
    let node = h.find("note").expect("the note");
    let editor = h.ui.editor(node).expect("an editor");
    editor.set_selection(0, 0);
    h.ui.input.focus(&h.ui.scene, Some(node));
    frame(&mut h);
    (h, editor)
}
fn frame(h: &mut Harness) {
    h.frames(3);
    assert_eq!(h.error(), None);
}

/// The fence deleted a character at a time, its last character last.
#[test]
fn backspacing_a_fence_frees_its_lines() {
    let (mut e, editor) = note("a\n```\nb\nd", FENCED, false);
    check(
        &e.ui,
        &editor,
        &[("a", 0.), ("```", 0.), ("b", 30.), ("d", 30.)],
    );
    editor.set_selection(5, 5);
    press(&mut e.ui, Key::Backspace);
    frame(&mut e);
    check(
        &e.ui,
        &editor,
        &[("a", 0.), ("``", 0.), ("b", 0.), ("d", 0.)],
    );
    press(&mut e.ui, Key::Backspace);
    press(&mut e.ui, Key::Backspace);
    frame(&mut e);
    assert_eq!(shown(&e.ui, &editor).len(), 3, "a, b and d");
}

/// Its line selected and deleted, undone and done again.
#[test]
fn a_fence_deleted_undone_and_redone() {
    let (mut e, editor) = note("a\n```\nb\nd", FENCED, false);
    editor.set_selection(2, 6);
    press(&mut e.ui, Key::Backspace);
    frame(&mut e);
    let gone = [("a", 0.), ("b", 0.), ("d", 0.)];
    check(&e.ui, &editor, &gone);
    assert!(editor.undo());
    frame(&mut e);
    check(
        &e.ui,
        &editor,
        &[("a", 0.), ("```", 0.), ("b", 30.), ("d", 30.)],
    );
    assert!(editor.redo());
    frame(&mut e);
    check(&e.ui, &editor, &gone);
}

/// A fence typed over lines already in a group (a second block's) undone and
/// redone.
#[test]
fn a_typed_fence_undone_and_redone() {
    let (mut e, editor) = note("a\n\nb\n```\nd\ne", FENCED, false);
    editor.set_selection(2, 2);
    let before = shown(&e.ui, &editor);
    e.ui.dispatch(InputEvent::Text("```".into()));
    frame(&mut e);
    let typed = [
        ("a", 0.),
        ("```", 0.),
        ("b", 30.),
        ("```", 0.),
        ("d", 0.),
        ("e", 0.),
    ];
    check(&e.ui, &editor, &typed);
    assert!(editor.undo());
    frame(&mut e);
    assert_eq!(shown(&e.ui, &editor), before);
    assert!(editor.redo());
    frame(&mut e);
    check(&e.ui, &editor, &typed);
}

/// The text set whole by the app, its fence gone, and set back.
#[test]
fn text_set_without_its_fence() {
    let (mut e, editor) = note("a\n```\nb\nd", FENCED, false);
    editor.value().set("a\nb\nd".to_string());
    frame(&mut e);
    check(&e.ui, &editor, &[("a", 0.), ("b", 0.), ("d", 0.)]);
    editor.value().set("a\n```\nb\nd".to_string());
    frame(&mut e);
    check(
        &e.ui,
        &editor,
        &[("a", 0.), ("```", 0.), ("b", 30.), ("d", 30.)],
    );
}

/// The line itself edited out of its group, its room read from its text.
#[test]
fn a_line_edited_out_of_its_group_shows() {
    let (mut e, editor) = note("a\nc\nd", BY_TEXT, false);
    check(&e.ui, &editor, &[("a", 0.), ("c", 30.), ("d", 0.)]);
    editor.set_selection(2, 3);
    e.ui.dispatch(InputEvent::Text("x".into()));
    frame(&mut e);
    check(&e.ui, &editor, &[("a", 0.), ("x", 0.), ("d", 0.)]);
    assert!(editor.undo());
    frame(&mut e);
    check(&e.ui, &editor, &[("a", 0.), ("c", 30.), ("d", 0.)]);
    assert!(editor.redo());
    frame(&mut e);
    check(&e.ui, &editor, &[("a", 0.), ("x", 0.), ("d", 0.)]);
}

/// A whole block deleted: prose shows where its lines were; undone, it is
/// back, and redone, gone.
#[test]
fn a_block_deleted_undone_and_redone() {
    let (mut e, editor) = note("a\nc1\nc2\nd\ne", BY_TEXT, false);
    let before = [("a", 0.), ("c1", 30.), ("c2", 30.), ("d", 0.), ("e", 0.)];
    check(&e.ui, &editor, &before);
    editor.set_selection(2, 8);
    press(&mut e.ui, Key::Backspace);
    frame(&mut e);
    let gone = [("a", 0.), ("d", 0.), ("e", 0.)];
    check(&e.ui, &editor, &gone);
    assert!(editor.undo());
    frame(&mut e);
    check(&e.ui, &editor, &before);
    assert!(editor.redo());
    frame(&mut e);
    check(&e.ui, &editor, &gone);
}

/// Two blocks, the first scrolled, its bar showing: it goes, with its bar,
/// and the second stays in its group and scrolls, its bar showing.
#[test]
fn one_of_two_blocks_goes_with_its_bar() {
    let second = WIDE.replacen('c', "g", 1);
    let (mut e, editor) = note(&format!("a\n{WIDE}\nb\n{second}\nd"), BY_TEXT, true);
    assert_eq!(bar(&e.ui, &editor), [], "none moved: no bar");
    editor.set_group_scroll(1, 10.);
    frame(&mut e);
    let first_bar = bar(&e.ui, &editor);
    assert!(!first_bar.is_empty(), "the first block's bar");
    check(
        &e.ui,
        &editor,
        &[("a", 0.), (WIDE, 20.), ("b", 0.), (&second, 30.), ("d", 0.)],
    );
    editor.set_selection(2, 2 + WIDE.len() + 1);
    press(&mut e.ui, Key::Backspace);
    frame(&mut e);
    check(
        &e.ui,
        &editor,
        &[("a", 0.), ("b", 0.), (&second, 30.), ("d", 0.)],
    );
    assert_eq!(editor.group_extent(1), None);
    assert!(editor.group_extent(2).is_some());
    assert_eq!(bar(&e.ui, &editor), [], "its block is gone: no bar");
    editor.set_group_scroll(2, 12.);
    frame(&mut e);
    check(
        &e.ui,
        &editor,
        &[("a", 0.), ("b", 0.), (&second, 18.), ("d", 0.)],
    );
    assert_eq!(
        bar(&e.ui, &editor).len(),
        first_bar.len(),
        "the second's bar"
    );
    assert!(editor.undo());
    frame(&mut e);
    check(
        &e.ui,
        &editor,
        &[("a", 0.), (WIDE, 20.), ("b", 0.), (&second, 18.), ("d", 0.)],
    );
}

/// A block's fence deleted while the block is out of view: scrolled back to,
/// its lines are prose.
#[test]
fn a_fence_deleted_out_of_view_frees_its_lines() {
    let below: String = (0..40).map(|i| format!("\np{i:02}")).collect();
    let (mut e, editor) = note(&format!("a\n```\nb\n```{below}"), FENCED, false);
    editor.set_scroll_offset(400.);
    frame(&mut e);
    assert_eq!(shown(&e.ui, &editor)[0].0, "p16");
    assert!(editor.replace_text(2..6, ""));
    editor.set_scroll_offset(0.);
    frame(&mut e);
    let top = shown(&e.ui, &editor);
    assert_eq!(top[..3], rows(0., &[("a", 0.), ("b", 0.), ("```", 0.)])[..]);
}
