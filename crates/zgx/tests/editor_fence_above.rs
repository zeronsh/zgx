//! A text area's lines whose room and marks depend on the highlighter's
//! state before them, as a fenced code block's do: a fence typed above lines
//! makes them its block's, and one deleted makes them prose again, as in a
//! note opened with that text. What a person sees is asserted: where lines,
//! marks, characters and the caret show, what a wheel and a press do, and how
//! far the note scrolls, in view and far below it.
use std::{cell::Cell, rc::Rc};
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    scene::{Color, NodeKind},
    widgets::{EditorHandle, LineInset, LineMark, Ui},
};

const GROUP: u32 = 7;
const LINES: usize = 40;

/// Every check of a test, each printed, failed together at its end: one that
/// fails doesn't hide the rest.
#[derive(Default)]
struct Checks(Vec<String>);
impl Checks {
    fn eq<T: PartialEq + std::fmt::Debug>(&mut self, what: &str, got: T, want: T) {
        if got == want {
            println!("PASS {what}: {got:?}");
        } else {
            println!("FAIL {what}: got {got:?}, want {want:?}");
            self.0.push(format!("{what}: got {got:?}, want {want:?}"));
        }
    }
    fn near(&mut self, what: &str, got: f32, want: f32) {
        if (got - want).abs() < 0.05 {
            println!("PASS {what}: {got}");
        } else {
            println!("FAIL {what}: got {got}, want {want}");
            self.0.push(format!("{what}: got {got}, want {want}"));
        }
    }
    fn done(self) {
        assert!(
            self.0.is_empty(),
            "{} failed:\n{}",
            self.0.len(),
            self.0.join("\n")
        );
    }
}

/// `intro`, a line `second`, then lines each twice as wide as the area's
/// text wraps in (with no word break), each told by its number.
fn text(second: &str) -> String {
    let mut text = format!("intro\n{second}");
    for i in 0..LINES {
        text.push_str(&format!("\nL{i:02}{}", "x".repeat(110)));
    }
    text
}
/// Line `i` of those, in `text(second)`.
fn line_start(second: &str, i: usize) -> usize {
    text(second).find(&format!("L{i:02}")).unwrap()
}

fn round(v: f32) -> f32 {
    (v * 100.).round() / 100.
}
/// Each line drawn: its first characters, and its box in the window. A
/// line's node only just out of view may be kept: those wholly outside the
/// area are left out.
type Drawn = Vec<(String, [f32; 4])>;
fn lines_drawn(ui: &Ui, editor: &EditorHandle) -> Drawn {
    let scene = ui.scene.borrow();
    let area = scene.transformed_bounds(editor.node);
    let mut found = Vec::new();
    let mut pending = vec![editor.node];
    while let Some(id) = pending.pop() {
        if let NodeKind::Text { text, .. } = scene.kind(id) {
            let r = scene.transformed_bounds(id);
            let line = ["L", "intro", "`"].iter().any(|s| text.starts_with(s));
            if line && r.y < area.y + area.height && r.y + r.height > area.y {
                let start = text
                    .chars()
                    .take(if text.starts_with("intro") { 5 } else { 3 })
                    .collect();
                found.push((start, [r.x, r.y, r.width, r.height].map(round)));
            }
        }
        pending.extend_from_slice(scene.children(id));
    }
    found.sort_by(|a, b| a.1[1].total_cmp(&b.1[1]));
    found
}
/// Each mark drawn in colour `fill`: its box in the window.
fn marks_drawn(ui: &Ui, editor: &EditorHandle, fill: Color) -> Vec<[f32; 4]> {
    let scene = ui.scene.borrow();
    let area = scene.transformed_bounds(editor.node);
    let mut found = Vec::new();
    let mut pending = vec![editor.node];
    while let Some(id) = pending.pop() {
        if matches!(scene.kind(id), NodeKind::Quad(q) if q.fill == fill) {
            let r = scene.transformed_bounds(id);
            if r.y < area.y + area.height && r.y + r.height > area.y {
                found.push([r.x, r.y, r.width, r.height].map(round));
            }
        }
        pending.extend_from_slice(scene.children(id));
    }
    found.sort_by(|a, b| a[1].total_cmp(&b[1]));
    found
}
const WASH: Color = Color(0xee, 0xee, 0xee, 255);
const DOT: Color = Color(0xff, 0, 0, 255);

/// `a` and `b`'s lines, marks and whole height, compared: `a` edited to the
/// text `b` was opened with, both scrolled as far.
fn same_view(checks: &mut Checks, what: &str, a: (&Ui, &EditorHandle), b: (&Ui, &EditorHandle)) {
    checks.eq(
        &format!("{what}: lines drawn"),
        lines_drawn(a.0, a.1),
        lines_drawn(b.0, b.1),
    );
    for (name, fill) in [("washes", WASH), ("dots", DOT)] {
        checks.eq(
            &format!("{what}: {name} drawn"),
            marks_drawn(a.0, a.1, fill),
            marks_drawn(b.0, b.1, fill),
        );
    }
    checks.eq(
        &format!("{what}: view and whole height"),
        a.1.scroll_extent(),
        b.1.scroll_extent(),
    );
    checks.eq(
        &format!("{what}: group extent"),
        a.1.group_extent(GROUP),
        b.1.group_extent(GROUP),
    );
}
fn first_x(drawn: &Drawn, start: &str) -> f32 {
    drawn
        .iter()
        .find(|(s, _)| s == start)
        .map_or(f32::NAN, |(_, r)| r[0])
}
/// A key press that deletes what's selected.
fn delete(ui: &mut Ui) {
    ui.dispatch(InputEvent::KeyDown {
        key: Key::Backspace,
        modifiers: Modifiers::default(),
        repeat: false,
    });
}
fn wheel_across(ui: &mut Ui, (x, y): (f32, f32), delta_x: f32) {
    ui.dispatch(InputEvent::Scroll {
        x,
        y,
        delta_x,
        delta_y: 0.,
    });
}
fn click(ui: &mut Ui, x: f32, y: f32) {
    let button = PointerButton::Primary;
    ui.dispatch(InputEvent::PointerDown { x, y, button });
    ui.dispatch(InputEvent::PointerUp { x, y, button });
}

/// A note 400 by 200 of `text(second)`: a fence's lines are 30 in and 4
/// below the line above, in a scroll group, unwrapped, with a pinned wash
/// and a dot.
fn note(second: &str) -> (Ui, EditorHandle) {
    let mut ui = Ui::new(600., 400.);
    let value = ui.signal(text(second));
    let view = ui.mount(
        text_area("Note", value)
            .w(400.)
            .h(200.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(true)
            .scrollbar(true)
            .span_highlighter(|line: &str, state| {
                (Vec::new(), if line == "```" { 1 - state } else { state })
            })
            .line_insets(|_, line: &str, state| {
                if state == 1 && line != "```" {
                    LineInset::new(30., 4., 0.).in_group(GROUP).unwrapped()
                } else {
                    LineInset::default()
                }
            })
            .line_marks(|_, line: &str, state| {
                if state != 1 || line == "```" {
                    return Vec::new();
                }
                let mut wash = LineMark::new(0., 0., f32::INFINITY, f32::INFINITY, WASH);
                wash.pinned = true;
                vec![wash, LineMark::new(60., 4., 6., 6., DOT)]
            }),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    // The caret at the top, so taking the focus scrolls nothing.
    editor.set_selection(0, 0);
    ui.input.focus(&ui.scene, Some(view.node()));
    frame(&mut ui);
    assert_eq!(editor.scroll_offset(), (0., 0.));
    assert_eq!(editor.group_scroll(GROUP), 0.);
    (ui, editor)
}
fn frame(ui: &mut Ui) {
    for _ in 0..3 {
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
    }
}
fn frame_ref(ui: &Ui) {
    for _ in 0..3 {
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
    }
}

/// A fence typed above lines: they are its block's, as in a note opened so.
#[test]
fn a_fence_typed_above_lines_makes_them_its_block() {
    let mut checks = Checks::default();
    let (mut e, editor) = note("");
    let (opened, reference) = note("```");
    editor.set_selection(6, 6);
    for _ in 0..3 {
        e.dispatch(InputEvent::Text("`".into()));
        frame(&mut e);
    }
    let origin = first_x(&lines_drawn(&e, &editor), "intro");
    checks.eq(
        "the text",
        editor.editor.borrow().text().to_string(),
        text("```"),
    );
    // At the top, before anything scrolls.
    same_view(
        &mut checks,
        "typed, at the top",
        (&e, &editor),
        (&opened, &reference),
    );
    let start = line_start("```", 0);
    checks.eq(
        "L00's rows",
        editor.range_boxes(start..start + 113).len(),
        1,
    );
    checks.eq(
        "a group is there",
        editor.group_extent(GROUP).is_some(),
        true,
    );
    // Its lines scroll sideways, together.
    let before = lines_drawn(&e, &editor);
    let box_before = editor.range_boxes(start + 20..start + 21).first().copied();
    let l00 = before
        .iter()
        .find(|(s, _)| s == "L00")
        .map_or([0.; 4], |(_, r)| *r);
    wheel_across(&mut e, (150., l00[1] + 8.), 24.);
    frame(&mut e);
    checks.eq(
        "group_scroll after a sideways wheel",
        editor.group_scroll(GROUP),
        24.,
    );
    let after = lines_drawn(&e, &editor);
    for line in ["L00", "L01", "L04"] {
        checks.near(
            &format!("{line} drawn this far across"),
            first_x(&after, line),
            first_x(&before, line) - 24.,
        );
    }
    let box_after = editor.range_boxes(start + 20..start + 21).first().copied();
    checks.near(
        "a character of L00 shows this far across",
        box_after.map_or(f32::NAN, |r| r.x),
        box_before.map_or(f32::NAN, |r| r.x - 24.),
    );
    checks.near(
        "L00's dot moved",
        marks_drawn(&e, &editor, DOT)
            .first()
            .map_or(f32::NAN, |r| r[0]),
        marks_drawn(&opened, &reference, DOT)
            .first()
            .map_or(f32::NAN, |r| r[0] - 24.),
    );
    checks.near(
        "L00's pinned wash still",
        marks_drawn(&e, &editor, WASH)
            .first()
            .map_or(f32::NAN, |r| r[0]),
        marks_drawn(&opened, &reference, WASH)
            .first()
            .map_or(f32::NAN, |r| r[0]),
    );
    // A press on that character, where it shows, puts the caret there.
    if let Some(r) = box_after {
        click(&mut e, r.x + 1., r.y + r.height / 2.);
        frame(&mut e);
        checks.eq(
            "a press on the character",
            editor.selection(),
            (start + 20, start + 20),
        );
        let caret = e.scene.borrow().transformed_bounds(editor.caret);
        checks.near("the caret is drawn at it", caret.x, r.x);
        checks.near("the caret is drawn on its row", caret.y, r.y);
    }
    reference.set_group_scroll(GROUP, 24.);
    // Far below: a line in the middle, then the end.
    editor.scroll_to_line(22, zgx::compose::RowAlign::Start);
    reference.scroll_to_line(22, zgx::compose::RowAlign::Start);
    frame(&mut e);
    frame_ref(&opened);
    checks.eq(
        "L20 at the view's top",
        editor.scroll_offset(),
        reference.scroll_offset(),
    );
    same_view(
        &mut checks,
        "L20 scrolled to",
        (&e, &editor),
        (&opened, &reference),
    );
    editor.set_scroll_offset(100_000.);
    reference.set_scroll_offset(100_000.);
    frame(&mut e);
    frame_ref(&opened);
    checks.eq(
        "how far down it scrolls",
        editor.scroll_offset(),
        reference.scroll_offset(),
    );
    same_view(&mut checks, "the end", (&e, &editor), (&opened, &reference));
    checks.near(
        "L39 moved with its group",
        first_x(&lines_drawn(&e, &editor), "L39"),
        origin + 30. - 24.,
    );
    checks.done();
}

/// The fence deleted: its lines are prose again, as in a note opened so.
#[test]
fn a_fence_deleted_above_lines_makes_them_prose() {
    let mut checks = Checks::default();
    let (mut e, editor) = note("```");
    let (opened, reference) = note("");
    let origin = first_x(&lines_drawn(&e, &editor), "intro");
    let l00 = lines_drawn(&e, &editor);
    let y = l00
        .iter()
        .find(|(s, _)| s == "L00")
        .map_or(0., |(_, r)| r[1]);
    wheel_across(&mut e, (150., y + 8.), 24.);
    frame(&mut e);
    checks.eq("the block scrolled first", editor.group_scroll(GROUP), 24.);
    checks.near(
        "L00 moved",
        first_x(&lines_drawn(&e, &editor), "L00"),
        origin + 30. - 24.,
    );
    editor.set_selection(6, 9);
    delete(&mut e);
    frame(&mut e);
    checks.eq(
        "the text",
        editor.editor.borrow().text().to_string(),
        text(""),
    );
    same_view(
        &mut checks,
        "deleted, at the top",
        (&e, &editor),
        (&opened, &reference),
    );
    let start = line_start("", 0);
    checks.eq(
        "L00's rows",
        editor.range_boxes(start..start + 113).len(),
        2,
    );
    checks.eq("no group", editor.group_extent(GROUP), None);
    checks.near(
        "L00's offset gone",
        first_x(&lines_drawn(&e, &editor), "L00"),
        origin,
    );
    checks.eq("no washes", marks_drawn(&e, &editor, WASH).len(), 0);
    checks.eq("no dots", marks_drawn(&e, &editor, DOT).len(), 0);
    let shown = editor.range_boxes(start + 20..start + 21).first().copied();
    let want = reference
        .range_boxes(start + 20..start + 21)
        .first()
        .copied();
    checks.eq("a character of L00 shows", shown, want);
    // A sideways wheel moves nothing.
    let before = lines_drawn(&e, &editor);
    wheel_across(&mut e, (150., y + 8.), 24.);
    frame(&mut e);
    checks.eq(
        "a sideways wheel moves no line",
        lines_drawn(&e, &editor),
        before,
    );
    if let Some(r) = want {
        click(&mut e, r.x + 1., r.y + r.height / 2.);
        frame(&mut e);
        checks.eq(
            "a press on the character",
            editor.selection(),
            (start + 20, start + 20),
        );
        let caret = e.scene.borrow().transformed_bounds(editor.caret);
        checks.near("the caret is drawn at it", caret.x, r.x);
    }
    editor.scroll_to_line(22, zgx::compose::RowAlign::Start);
    reference.scroll_to_line(22, zgx::compose::RowAlign::Start);
    frame(&mut e);
    frame_ref(&opened);
    checks.eq(
        "L20 at the view's top",
        editor.scroll_offset(),
        reference.scroll_offset(),
    );
    same_view(
        &mut checks,
        "L20 scrolled to",
        (&e, &editor),
        (&opened, &reference),
    );
    editor.set_scroll_offset(100_000.);
    reference.set_scroll_offset(100_000.);
    frame(&mut e);
    frame_ref(&opened);
    checks.eq(
        "how far down it scrolls",
        editor.scroll_offset(),
        reference.scroll_offset(),
    );
    same_view(&mut checks, "the end", (&e, &editor), (&opened, &reference));
    checks.done();
}

/// A note 400 by 200 of `text`: the lines of its n-th block (from 0) in
/// scroll group 2n + 1, its group scrollbar on, no marks.
fn blocks(text: &str) -> (Ui, EditorHandle) {
    let mut ui = Ui::new(600., 400.);
    let value = ui.signal(text.to_string());
    let view = ui.mount(
        text_area("Note", value)
            .w(400.)
            .h(200.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(true)
            .group_scrollbar()
            .span_highlighter(|line: &str, state| (Vec::new(), state + u32::from(line == "```")))
            .line_insets(|_, line: &str, state| {
                if state % 2 == 1 && line != "```" {
                    LineInset::new(30., 4., 0.).in_group(state).unwrapped()
                } else {
                    LineInset::default()
                }
            }),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_selection(0, 0);
    ui.input.focus(&ui.scene, Some(view.node()));
    frame(&mut ui);
    (ui, editor)
}

/// The whole of `a` is the whole of `b`, a note opened with its text: at the
/// top, at L20 and at the end.
fn same_everywhere(
    checks: &mut Checks,
    what: &str,
    a: (&mut Ui, &EditorHandle),
    b: (&mut Ui, &EditorHandle),
) {
    let (a, editor) = a;
    let (b, reference) = b;
    for (name, to) in [("top", 0.), ("L20", -1.), ("end", 1.0e6)] {
        if to < 0. {
            editor.scroll_to_line(22, zgx::compose::RowAlign::Start);
            reference.scroll_to_line(22, zgx::compose::RowAlign::Start);
        } else {
            editor.set_scroll_offset(to);
            reference.set_scroll_offset(to);
        }
        frame(a);
        frame(b);
        checks.eq(
            &format!("{what}, {name}: scrolled"),
            editor.scroll_offset(),
            reference.scroll_offset(),
        );
        same_view(
            checks,
            &format!("{what}, {name}"),
            (a, editor),
            (b, reference),
        );
    }
    editor.set_scroll_offset(0.);
    reference.set_scroll_offset(0.);
    frame(a);
    frame(b);
    // A sideways wheel over the line at the view's middle, on both.
    let before = lines_drawn(b, reference);
    wheel_across(a, (150., 110.), 24.);
    wheel_across(b, (150., 110.), 24.);
    frame(a);
    frame(b);
    let moved = before != lines_drawn(b, reference);
    checks.eq(
        &format!("{what}: group scroll after a wheel (moves: {moved})"),
        editor.group_scroll(GROUP),
        reference.group_scroll(GROUP),
    );
    checks.eq(
        &format!("{what}: lines after a wheel"),
        lines_drawn(a, editor),
        lines_drawn(b, reference),
    );
    editor.set_group_scroll(GROUP, 0.);
    frame(a);
}
/// Undo, or redo, until the text is `want`.
fn step_to(
    checks: &mut Checks,
    what: &str,
    editor: &EditorHandle,
    step: fn(&EditorHandle) -> bool,
    want: &str,
) {
    for _ in 0..4 {
        if editor.editor.borrow().text() == want || !step(editor) {
            break;
        }
    }
    checks.eq(
        what,
        editor.editor.borrow().text().to_string(),
        want.to_string(),
    );
}

/// A typed fence undone and redone: at each step, the note opened so.
#[test]
fn a_typed_fence_undone_and_redone_is_the_note_opened_so() {
    let mut checks = Checks::default();
    let (mut e, editor) = note("");
    editor.set_selection(6, 6);
    for _ in 0..3 {
        e.dispatch(InputEvent::Text("`".into()));
        frame(&mut e);
    }
    let (mut o, reference) = note("```");
    same_everywhere(
        &mut checks,
        "typed",
        (&mut e, &editor),
        (&mut o, &reference),
    );
    step_to(
        &mut checks,
        "undone",
        &editor,
        EditorHandle::undo,
        &text(""),
    );
    frame(&mut e);
    let (mut o, reference) = note("");
    same_everywhere(
        &mut checks,
        "undone",
        (&mut e, &editor),
        (&mut o, &reference),
    );
    step_to(
        &mut checks,
        "redone",
        &editor,
        EditorHandle::redo,
        &text("```"),
    );
    frame(&mut e);
    let (mut o, reference) = note("```");
    same_everywhere(
        &mut checks,
        "redone",
        (&mut e, &editor),
        (&mut o, &reference),
    );
    checks.done();
}

/// A deleted fence undone and redone: at each step, the note opened so.
#[test]
fn a_deleted_fence_undone_and_redone_is_the_note_opened_so() {
    let mut checks = Checks::default();
    let (mut e, editor) = note("```");
    editor.set_selection(6, 9);
    delete(&mut e);
    frame(&mut e);
    let (mut o, reference) = note("");
    same_everywhere(
        &mut checks,
        "deleted",
        (&mut e, &editor),
        (&mut o, &reference),
    );
    step_to(
        &mut checks,
        "undone",
        &editor,
        EditorHandle::undo,
        &text("```"),
    );
    frame(&mut e);
    let (mut o, reference) = note("```");
    same_everywhere(
        &mut checks,
        "undone",
        (&mut e, &editor),
        (&mut o, &reference),
    );
    step_to(
        &mut checks,
        "redone",
        &editor,
        EditorHandle::redo,
        &text(""),
    );
    frame(&mut e);
    let (mut o, reference) = note("");
    same_everywhere(
        &mut checks,
        "redone",
        (&mut e, &editor),
        (&mut o, &reference),
    );
    checks.done();
}

/// Lines `names`, a line each, those starting `L` wider than the area.
fn doc(names: &[&str]) -> String {
    let lines: Vec<String> = (names.iter())
        .map(|name| match name.starts_with('L') {
            true => format!("{name}{}", "x".repeat(110)),
            false => name.to_string(),
        })
        .collect();
    lines.join("\n")
}
/// Three blocks with prose between them, the last far below.
fn three_blocks() -> String {
    let mut names = vec![
        "intro", "```", "LA1", "LA2", "```", "LM0", "```", "LB1", "```",
    ];
    names.extend(["LP0"; 30]);
    names.extend(["```", "LC1", "```", "intro end"]);
    doc(&names)
}
const IDS: [u32; 4] = [1, 3, 5, 7];
/// The quads drawn: with no marks, its group scrollbar's.
fn bars_drawn(ui: &Ui, editor: &EditorHandle) -> Vec<[f32; 4]> {
    let scene = ui.scene.borrow();
    let mut found = Vec::new();
    let mut pending = vec![editor.node];
    while let Some(id) = pending.pop() {
        let r = scene.transformed_bounds(id);
        if matches!(scene.kind(id), NodeKind::Quad(_))
            && r.width > 0.
            && r.height > 0.
            && scene.layout_visible(id)
        {
            found.push([r.x, r.y, r.width, r.height].map(round));
        }
        pending.extend_from_slice(scene.children(id));
    }
    found
}
/// What `e` shows is what a note opened with its text shows, its groups
/// scrolled as far (`last` last): at the top, in the middle and at the end;
/// and no group is scrolled past its lines.
fn same_blocks(
    checks: &mut Checks,
    what: &str,
    e: &mut Ui,
    editor: &EditorHandle,
    last: &Rc<Cell<u32>>,
) {
    let text = editor.editor.borrow().text().to_string();
    let (mut o, reference) = blocks(&text);
    for id in IDS {
        let at = editor.group_scroll(id);
        if at > 0. {
            reference.set_group_scroll(id, at);
        }
        if let Some((view, content)) = editor.group_extent(id) {
            let limit = (content - view).max(0.);
            checks.eq(
                &format!("{what}: group {id} at {at} within {limit}"),
                at <= limit,
                true,
            );
        }
    }
    // The one that moved last: its bar shows.
    let (last, at) = (last.get(), editor.group_scroll(last.get()));
    reference.set_group_scroll(last, at + 1.);
    reference.set_group_scroll(last, at - 1.);
    reference.set_group_scroll(last, at);
    for (name, to) in [("top", 0.), ("middle", -1.), ("end", 1.0e6)] {
        if to < 0. {
            editor.scroll_to_line(20, zgx::compose::RowAlign::Start);
            reference.scroll_to_line(20, zgx::compose::RowAlign::Start);
        } else {
            editor.set_scroll_offset(to);
            reference.set_scroll_offset(to);
        }
        frame(e);
        frame(&mut o);
        let what = format!("{what}, {name}");
        checks.eq(
            &format!("{what}: scrolled"),
            editor.scroll_offset(),
            reference.scroll_offset(),
        );
        checks.eq(
            &format!("{what}: lines drawn"),
            lines_drawn(e, editor),
            lines_drawn(&o, &reference),
        );
        checks.eq(
            &format!("{what}: bars drawn"),
            bars_drawn(e, editor),
            bars_drawn(&o, &reference),
        );
        for id in IDS {
            checks.eq(
                &format!("{what}: group {id}'s extent"),
                editor.group_extent(id),
                reference.group_extent(id),
            );
        }
    }
    editor.set_scroll_offset(0.);
    frame(e);
}
/// The group of `editor`'s that moved last.
fn watched(editor: &EditorHandle) -> Rc<Cell<u32>> {
    let last = Rc::new(Cell::new(1));
    let told = last.clone();
    editor.on_group_scroll(move |id, _| told.set(id));
    last
}
fn edited(
    checks: &mut Checks,
    e: &mut Ui,
    editor: &EditorHandle,
    last: &Rc<Cell<u32>>,
    what: &str,
    (range, with): (std::ops::Range<usize>, &str),
) {
    let before = editor.editor.borrow().text().to_string();
    checks.eq(what, editor.replace_text(range, with), true);
    frame(e);
    let after = editor.editor.borrow().text().to_string();
    same_blocks(checks, what, e, editor, last);
    let undone = format!("{what}, undone");
    step_to(checks, &undone, editor, EditorHandle::undo, &before);
    frame(e);
    same_blocks(checks, &undone, e, editor, last);
    let redone = format!("{what}, redone");
    step_to(checks, &redone, editor, EditorHandle::redo, &after);
    frame(e);
    same_blocks(checks, &redone, e, editor, last);
}

/// A fence typed inside a block splits it, and flips those below.
#[test]
fn a_fence_that_splits_a_block_is_the_note_opened_so() {
    let mut checks = Checks::default();
    let start = three_blocks();
    let (mut e, editor) = blocks(&start);
    let last = &watched(&editor);
    editor.set_group_scroll(1, 24.);
    frame(&mut e);
    same_blocks(&mut checks, "before", &mut e, &editor, last);
    let at = start.find("LA2").unwrap();
    edited(
        &mut checks,
        &mut e,
        &editor,
        last,
        "split",
        (at..at, "```\n"),
    );
    checks.done();
}

/// A block's close, the prose after it and the next one's fence deleted:
/// the two are one.
#[test]
fn a_fence_that_merges_blocks_is_the_note_opened_so() {
    let mut checks = Checks::default();
    let start = three_blocks();
    let (mut e, editor) = blocks(&start);
    let last = &watched(&editor);
    editor.set_group_scroll(3, 12.);
    editor.set_group_scroll(1, 24.);
    frame(&mut e);
    same_blocks(&mut checks, "before", &mut e, &editor, last);
    let from = start.find("```\nLM0").unwrap();
    let to = start.find("LB1").unwrap();
    edited(&mut checks, &mut e, &editor, last, "merge", (from..to, ""));
    checks.done();
}

/// The first block's opening fence deleted: the blocks below flip.
#[test]
fn a_first_fence_deleted_flips_the_blocks_below() {
    let mut checks = Checks::default();
    let start = three_blocks();
    let (mut e, editor) = blocks(&start);
    let last = &watched(&editor);
    editor.set_group_scroll(1, 24.);
    frame(&mut e);
    edited(&mut checks, &mut e, &editor, last, "flip", (6..10, ""));
    checks.done();
}

/// A block's text cut short by the app, the caret elsewhere: its group isn't
/// scrolled past its lines.
#[test]
fn a_block_cut_short_is_not_scrolled_past_its_lines() {
    let mut checks = Checks::default();
    let start = doc(&["intro", "```", "LA1", "```", "LP0"]);
    let (mut e, editor) = blocks(&start);
    let last = &watched(&editor);
    editor.set_group_scroll(1, 200.);
    frame(&mut e);
    checks.eq("scrolled first", editor.group_scroll(1), 200.);
    editor.value().set(start.replacen(&"x".repeat(110), "", 1));
    frame(&mut e);
    same_blocks(&mut checks, "cut", &mut e, &editor, last);
    checks.done();
}
