//! A text area's span with a picture in place of its inline box character (`SpanStyle::inline_svg`), as a note's
//! formula among words: one atom in the line, the picture drawn in its box, the line grown to hold it.
use std::{rc::Rc, sync::Arc};
use zgx::{
    compose::{InlineSvg, prelude::*},
    rich_text::INLINE_BOX_TEXT,
    scene::{NodeKind, Rect},
    svg::SvgData,
    text_layout::TextAlign,
    widgets::{EditorHandle, LineInset, LineSpans, SpanStyle, Ui},
};

const BOX: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2 1.5"><rect width="2" height="1.5"/></svg>"#;

fn text() -> String {
    format!("ab{INLINE_BOX_TEXT}cd\n{INLINE_BOX_TEXT}")
}
/// 2 ems wide, 1 above the baseline and 0.5 below, on each box character.
fn formulas(line: &str, _: u32) -> (LineSpans, u32) {
    let svg = Arc::new(SvgData::new(BOX.as_bytes().to_vec()).unwrap());
    let spans = line
        .match_indices(INLINE_BOX_TEXT)
        .map(|(at, s)| {
            let formula = InlineSvg {
                svg: svg.clone(),
                width: 2.,
                height: 1.,
                depth: 0.5,
            };
            (at..at + s.len(), SpanStyle::default().inline_svg(formula))
        })
        .collect();
    (spans, 0)
}
fn note(centred: bool) -> (Ui, EditorHandle) {
    let mut ui = Ui::new(500., 400.);
    let value = ui.signal(text());
    let view = ui.mount(
        text_area("Note", value)
            .w(400.)
            .h(300.)
            .p(0.)
            .text_size(10.)
            .line_height(12.)
            .text_wrap(true)
            .span_highlighter(formulas),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    if centred {
        editor.set_line_insets(Some(Rc::new(|i| match i {
            1 => LineInset {
                align: Some(TextAlign::Center),
                ..LineInset::default()
            },
            _ => LineInset::default(),
        })));
    }
    ui.input.focus(&ui.scene, Some(view.node()));
    ui.prepare_frame();
    (ui, editor)
}
fn pictures(ui: &Ui, editor: &EditorHandle) -> Vec<Rect> {
    let scene = ui.scene.borrow();
    let mut found = Vec::new();
    let mut pending = vec![editor.node];
    while let Some(id) = pending.pop() {
        if let NodeKind::Svg(_) = scene.kind(id) {
            found.push(scene.transformed_bounds(id));
        }
        pending.extend(scene.children(id).iter().copied());
    }
    found.sort_by(|a, b| a.y.total_cmp(&b.y));
    found
}
fn caret(ui: &Ui, editor: &EditorHandle, at: usize) -> Rect {
    editor.set_selection(at, at);
    ui.prepare_frame();
    ui.scene.borrow().transformed_bounds(editor.caret)
}

#[test]
fn a_formula_stands_in_its_line_as_one_atom_drawn_in_its_box() {
    let (ui, editor) = note(false);
    let origin = caret(&ui, &editor, 0);
    let before = caret(&ui, &editor, 2);
    let after = caret(&ui, &editor, 2 + INLINE_BOX_TEXT.len());
    assert_eq!(
        after.x - before.x,
        20.,
        "the box takes its 2 ems, nothing more"
    );
    let drawn = pictures(&ui, &editor);
    assert_eq!(drawn.len(), 2, "each formula drawn: {drawn:?}");
    let formula = drawn[0];
    assert_eq!(
        (formula.x, formula.width, formula.height),
        (before.x, 20., 15.),
        "in its box"
    );
    assert!(
        formula.y >= origin.y - 0.5,
        "inside its line: {formula:?} {origin:?}"
    );
    // The line below starts under the box's 5 points of depth, taller than a bare line's 12.
    let next = caret(&ui, &editor, text().find('\n').unwrap() + 1);
    assert!(
        next.y - origin.y > 12.,
        "the line grew to hold it: {}",
        next.y - origin.y
    );
}

#[test]
fn a_formula_alone_on_a_centred_line_is_drawn_centred() {
    let (ui, editor) = note(true);
    let origin = caret(&ui, &editor, 0);
    let drawn = pictures(&ui, &editor);
    assert_eq!(
        drawn[1].x - origin.x,
        (400. - 20.) / 2.,
        "centred across the line"
    );
    let line = text().find('\n').unwrap() + 1;
    assert_eq!(
        caret(&ui, &editor, line).x,
        drawn[1].x,
        "its caret before it"
    );
}
