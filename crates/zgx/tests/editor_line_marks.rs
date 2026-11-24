//! A text area's line marks (`EditorHandle::set_line_marks`): shapes beside
//! its lines (a quote's rule, a code block's wash), placed where their lines
//! are, gone with the marks.
use std::rc::Rc;
use zgx::{
    compose::prelude::*,
    scene::{Color, NodeId, NodeKind, Rect},
    widgets::{EditorHandle, LineInset, LineMark, Ui},
};

const RULE: Color = Color(10, 20, 30, 255);
const WASH: Color = Color(200, 200, 210, 255);

fn area(text: &str) -> (Ui, EditorHandle) {
    let mut ui = Ui::new(400., 300.);
    let value = ui.signal(text.to_string());
    let root = ui.mount(
        column().size(400., 300.).child(
            text_area("Note", value)
                .id("field")
                .size(300., 240.)
                .text_size(10.)
                .line_height(20.)
                .text_wrap(true),
        ),
    );
    ui.prepare_frame();
    let editor = ui.editor(root.find("field").unwrap()).unwrap();
    (ui, editor)
}

/// The quads in the editor filled with `fill`, where they show.
fn quads(ui: &Ui, editor: &EditorHandle, fill: Color) -> Vec<Rect> {
    let scene = ui.scene.borrow();
    let mut stack: Vec<NodeId> = vec![editor.node];
    let mut out = Vec::new();
    while let Some(id) = stack.pop() {
        if matches!(scene.kind(id), NodeKind::Quad(q) if q.fill == fill) {
            out.push(scene.transformed_bounds(id));
        }
        stack.extend(scene.children(id).iter().copied());
    }
    out.sort_by(|a, b| a.y.total_cmp(&b.y));
    out
}

fn frame(ui: &mut Ui) {
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
}

/// A quote's rule beside its lines and a code line's wash to the area's
/// edge show where their lines are, under room the line has above; marks
/// gone, the shapes go.
#[test]
fn line_marks_show_beside_their_lines_and_go_with_them() {
    let (mut ui, editor) = area("plain\n> quoted\ncode\nend");
    editor.set_line_insets(Some(Rc::new(|i, _, _| {
        if i == 2 {
            LineInset::new(0., 10., 0.)
        } else {
            LineInset::default()
        }
    })));
    editor.set_line_marks(Some(Rc::new(|_, line: &str, _| {
        if line.starts_with('>') {
            vec![LineMark::new(0., 0., 3., f32::INFINITY, RULE)]
        } else if line == "code" {
            vec![LineMark::new(0., 0., f32::INFINITY, f32::INFINITY, WASH)]
        } else {
            Vec::new()
        }
    })));
    frame(&mut ui);
    let rule = quads(&ui, &editor, RULE);
    assert_eq!(rule.len(), 1, "one rule: {rule:?}");
    assert_eq!(rule[0].width, 3.);
    assert_eq!(rule[0].height, 20., "as tall as its line");
    let wash = quads(&ui, &editor, WASH);
    assert_eq!(wash.len(), 1, "one wash: {wash:?}");
    // The third line starts a line below the second, its text under the
    // 10 of room above it.
    assert_eq!(wash[0].y - rule[0].y, 30.);
    assert_eq!(wash[0].height, 20.);
    assert!(
        wash[0].width >= 250.,
        "to the area's edge: {}",
        wash[0].width
    );

    editor.set_line_marks(None);
    frame(&mut ui);
    assert!(quads(&ui, &editor, RULE).is_empty(), "the rule went");
    assert!(quads(&ui, &editor, WASH).is_empty(), "the wash went");
}

/// A mark's label is text beside its line (an ordered list's number).
#[test]
fn a_line_marks_label_shows_beside_its_line() {
    let (mut ui, editor) = area("1. one\n2. two");
    editor.set_line_marks(Some(Rc::new(|i, _, _| {
        let mut mark = LineMark::new(0., 0., 20., 20., Color(0, 0, 0, 0));
        mark.label = Some((format!("{}.", i + 1).into(), RULE, 10.));
        mark.tabular = true;
        vec![mark]
    })));
    frame(&mut ui);
    let scene = ui.scene.borrow();
    let mut stack: Vec<NodeId> = vec![editor.node];
    let mut labels = Vec::new();
    while let Some(id) = stack.pop() {
        if let NodeKind::Text { text, color, .. } = scene.kind(id)
            && *color == RULE
        {
            labels.push(text.to_string());
        }
        stack.extend(scene.children(id).iter().copied());
    }
    labels.sort();
    assert_eq!(labels, ["1.", "2."]);
}
