//! Lines shown leave their scroll group (`LineInset::scroll_group`) by an
//! edit: a code block's fence or the block deleted, undone and redone. The
//! group's layer goes and its lines show as any other, where they are.
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers},
    scene::{NodeId, NodeKind},
    widgets::{EditorHandle, LineInset, Ui},
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

/// The scroll group a line is in, by its text and the state before it (1
/// after a fence).
type GroupOf = fn(&str, u32) -> Option<u32>;
const FENCED: GroupOf = |line, state| (state == 1 && line != "```").then_some(1);
/// `c` lines are in group 1, `g` lines in group 2.
const BY_TEXT: GroupOf = |line, _| match line.chars().next() {
    Some('c') => Some(1),
    Some('g') => Some(2),
    _ => None,
};
/// A note 300 by 200 of `text`, each grouped line 30 in and unwrapped.
fn note(text: &str, group_of: GroupOf, scrollbar: bool) -> (Ui, EditorHandle) {
    let mut ui = Ui::new(400., 300.);
    let value = ui.signal(text.to_string());
    let mut area = text_area("Note", value)
        .w(300.)
        .h(200.)
        .p(0.)
        .text_size(10.)
        .line_height(20.)
        .text_wrap(true)
        .span_highlighter(|line: &str, state| {
            (Vec::new(), if line == "```" { 1 - state } else { state })
        })
        .line_insets(move |_, line: &str, state| match group_of(line, state) {
            Some(group) => LineInset::new(30., 0., 0.).in_group(group).unwrapped(),
            None => LineInset::default(),
        });
    if scrollbar {
        area = area.group_scrollbar();
    }
    let view = ui.mount(area);
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_selection(0, 0);
    ui.input.focus(&ui.scene, Some(view.node()));
    frame(&mut ui);
    (ui, editor)
}
fn frame(ui: &mut Ui) {
    for _ in 0..2 {
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
    }
}

/// The fence deleted a character at a time, its last character last.
#[test]
fn backspacing_a_fence_frees_its_lines() {
    let (mut e, editor) = note("a\n```\nb\nd", FENCED, false);
    check(
        &e,
        &editor,
        &[("a", 0.), ("```", 0.), ("b", 30.), ("d", 30.)],
    );
    editor.set_selection(5, 5);
    press(&mut e, Key::Backspace);
    frame(&mut e);
    check(&e, &editor, &[("a", 0.), ("``", 0.), ("b", 0.), ("d", 0.)]);
    press(&mut e, Key::Backspace);
    press(&mut e, Key::Backspace);
    frame(&mut e);
    assert_eq!(shown(&e, &editor).len(), 3, "a, b and d");
}

/// Its line selected and deleted, undone and done again.
#[test]
fn a_fence_deleted_undone_and_redone() {
    let (mut e, editor) = note("a\n```\nb\nd", FENCED, false);
    editor.set_selection(2, 6);
    press(&mut e, Key::Backspace);
    frame(&mut e);
    let gone = [("a", 0.), ("b", 0.), ("d", 0.)];
    check(&e, &editor, &gone);
    assert!(editor.undo());
    frame(&mut e);
    check(
        &e,
        &editor,
        &[("a", 0.), ("```", 0.), ("b", 30.), ("d", 30.)],
    );
    assert!(editor.redo());
    frame(&mut e);
    check(&e, &editor, &gone);
}

/// A fence typed over lines already in a group (a second block's) undone and
/// redone.
#[test]
fn a_typed_fence_undone_and_redone() {
    let (mut e, editor) = note("a\n\nb\n```\nd\ne", FENCED, false);
    editor.set_selection(2, 2);
    let before = shown(&e, &editor);
    e.dispatch(InputEvent::Text("```".into()));
    frame(&mut e);
    let typed = [
        ("a", 0.),
        ("```", 0.),
        ("b", 30.),
        ("```", 0.),
        ("d", 0.),
        ("e", 0.),
    ];
    check(&e, &editor, &typed);
    assert!(editor.undo());
    frame(&mut e);
    assert_eq!(shown(&e, &editor), before);
    assert!(editor.redo());
    frame(&mut e);
    check(&e, &editor, &typed);
}

/// The text set whole by the app, its fence gone, and set back.
#[test]
fn text_set_without_its_fence() {
    let (mut e, editor) = note("a\n```\nb\nd", FENCED, false);
    editor.value().set("a\nb\nd".to_string());
    frame(&mut e);
    check(&e, &editor, &[("a", 0.), ("b", 0.), ("d", 0.)]);
    editor.value().set("a\n```\nb\nd".to_string());
    frame(&mut e);
    check(
        &e,
        &editor,
        &[("a", 0.), ("```", 0.), ("b", 30.), ("d", 30.)],
    );
}

/// The line itself edited out of its group, its room read from its text.
#[test]
fn a_line_edited_out_of_its_group_shows() {
    let (mut e, editor) = note("a\nc\nd", BY_TEXT, false);
    check(&e, &editor, &[("a", 0.), ("c", 30.), ("d", 0.)]);
    editor.set_selection(2, 3);
    e.dispatch(InputEvent::Text("x".into()));
    frame(&mut e);
    check(&e, &editor, &[("a", 0.), ("x", 0.), ("d", 0.)]);
    assert!(editor.undo());
    frame(&mut e);
    check(&e, &editor, &[("a", 0.), ("c", 30.), ("d", 0.)]);
    assert!(editor.redo());
    frame(&mut e);
    check(&e, &editor, &[("a", 0.), ("x", 0.), ("d", 0.)]);
}

/// A whole block deleted: prose shows where its lines were; undone, it is
/// back, and redone, gone.
#[test]
fn a_block_deleted_undone_and_redone() {
    let (mut e, editor) = note("a\nc1\nc2\nd\ne", BY_TEXT, false);
    let before = [("a", 0.), ("c1", 30.), ("c2", 30.), ("d", 0.), ("e", 0.)];
    check(&e, &editor, &before);
    editor.set_selection(2, 8);
    press(&mut e, Key::Backspace);
    frame(&mut e);
    let gone = [("a", 0.), ("d", 0.), ("e", 0.)];
    check(&e, &editor, &gone);
    assert!(editor.undo());
    frame(&mut e);
    check(&e, &editor, &before);
    assert!(editor.redo());
    frame(&mut e);
    check(&e, &editor, &gone);
}

/// Two blocks, the first scrolled, its bar showing: it goes, with its bar,
/// and the second stays in its group and scrolls, its bar showing.
#[test]
fn one_of_two_blocks_goes_with_its_bar() {
    let second = WIDE.replacen('c', "g", 1);
    let (mut e, editor) = note(&format!("a\n{WIDE}\nb\n{second}\nd"), BY_TEXT, true);
    assert_eq!(bar(&e, &editor), [], "none moved: no bar");
    editor.set_group_scroll(1, 10.);
    frame(&mut e);
    let first_bar = bar(&e, &editor);
    assert!(!first_bar.is_empty(), "the first block's bar");
    check(
        &e,
        &editor,
        &[("a", 0.), (WIDE, 20.), ("b", 0.), (&second, 30.), ("d", 0.)],
    );
    editor.set_selection(2, 2 + WIDE.len() + 1);
    press(&mut e, Key::Backspace);
    frame(&mut e);
    check(
        &e,
        &editor,
        &[("a", 0.), ("b", 0.), (&second, 30.), ("d", 0.)],
    );
    assert_eq!(editor.group_extent(1), None);
    assert!(editor.group_extent(2).is_some());
    assert_eq!(bar(&e, &editor), [], "its block is gone: no bar");
    editor.set_group_scroll(2, 12.);
    frame(&mut e);
    check(
        &e,
        &editor,
        &[("a", 0.), ("b", 0.), (&second, 18.), ("d", 0.)],
    );
    assert_eq!(bar(&e, &editor).len(), first_bar.len(), "the second's bar");
    assert!(editor.undo());
    frame(&mut e);
    check(
        &e,
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
    assert_eq!(shown(&e, &editor)[0].0, "p16");
    assert!(editor.replace_text(2..6, ""));
    editor.set_scroll_offset(0.);
    frame(&mut e);
    let top = shown(&e, &editor);
    assert_eq!(top[..3], rows(0., &[("a", 0.), ("b", 0.), ("```", 0.)])[..]);
}

/// The area's insets taken away while its group shows scrolled: its lines
/// are prose, and its bar is gone.
#[test]
fn insets_taken_away_free_their_lines() {
    let (mut e, editor) = note(&format!("a\n{WIDE}\nd"), BY_TEXT, true);
    editor.set_group_scroll(1, 10.);
    frame(&mut e);
    assert!(!bar(&e, &editor).is_empty());
    editor.set_line_insets(None);
    frame(&mut e);
    let drawn = shown(&e, &editor);
    let places: Vec<_> = drawn.iter().map(|(text, x, _)| (&text[..1], *x)).collect();
    assert_eq!(places, [("a", 0.), ("c", 0.), ("d", 0.)]);
    assert_eq!(bar(&e, &editor), []);
    // Wrapped again: its box across two rows.
    let boxes = editor.range_boxes(2..2 + WIDE.len());
    assert!(boxes.len() >= 2, "{boxes:?}");
    assert!(boxes.iter().all(|b| b.x == 0.), "{boxes:?}");
}

/// Insets that put no line in a group any more.
#[test]
fn insets_without_groups_free_their_lines() {
    let (mut e, editor) = note("a\nc\nd", BY_TEXT, false);
    editor.set_group_scroll(1, 0.);
    editor.set_line_insets(Some(std::rc::Rc::new(|_, line: &str, _| {
        LineInset::new(if line == "c" { 10. } else { 0. }, 0., 0.)
    })));
    frame(&mut e);
    check(&e, &editor, &[("a", 0.), ("c", 10.), ("d", 0.)]);
}

/// Another highlighter, whose states put no line in a block.
#[test]
fn another_highlighter_frees_its_lines() {
    let (mut e, editor) = note("a\n```\nb\nd", FENCED, false);
    editor.set_span_highlighter(Some(std::rc::Rc::new(|_: &str, _| (Vec::new(), 0))));
    frame(&mut e);
    check(&e, &editor, &[("a", 0.), ("```", 0.), ("b", 0.), ("d", 0.)]);
}
