//! A code block kept as one line of a text area, its code lines joined by
//! hard breaks (`\u{1e}`, as Anara's notes keep it), never wrapping
//! (`LineInset::nowrap`): each code line a row, its carets and hits on the
//! row it shows on, the block as tall as its rows, as with the real shaper.
use std::rc::Rc;
use zgx::{
    compose::prelude::*,
    widgets::{LineInset, Ui},
};
use zgx_gpu::GpuRenderer;

const ROW: f32 = 20.;

/// A text area of `text`, its line 1 a code block (`nowrap` in a scroll
/// group, 30 points in), with the real shaper; `mono`: its code in the
/// monospaced font by a highlighter (a styled line, shaped as rich text).
fn note(text: &str, mono: bool) -> (GpuRenderer, Ui, zgx::widgets::EditorHandle) {
    let gpu = GpuRenderer::new(400, 300).unwrap();
    let mut ui = Ui::new(400., 300.);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let value = ui.signal(text.to_owned());
    let view = ui.mount(
        text_area("Note", value)
            .id("note")
            .w(360.)
            .h(260.)
            .p(0.)
            .text_size(12.)
            .line_height(ROW)
            .text_wrap(true),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.find("note").unwrap()).unwrap();
    editor.set_line_insets(Some(Rc::new(|i| match i {
        1 => LineInset {
            left: 30.,
            nowrap: true,
            scroll_group: Some(3),
            ..LineInset::default()
        },
        _ => LineInset::default(),
    })));
    if mono {
        editor.set_span_highlighter(Some(Rc::new(|line: &str, state| {
            let spans = if line.starts_with("const") {
                let mut style = zgx::widgets::SpanStyle::default();
                style.mono = true;
                vec![(0..line.len(), style)]
            } else {
                Vec::new()
            };
            (spans, state)
        })));
    }
    ui.prepare_frame();
    (gpu, ui, editor)
}

#[test]
fn a_code_blocks_hard_breaks_are_its_rows() {
    rows(false);
}

#[test]
fn a_highlighted_code_blocks_hard_breaks_are_its_rows() {
    rows(true);
}

/// As Anara's note: a 16-point text area, its code line's spans their own
/// size and pitch (14 and 20), room above and below it, a pinned surface.
#[test]
fn a_notes_code_block_with_its_own_size_has_a_row_per_code_line() {
    let text = format!(
        "intro\nconst x = \"{}\";\u{1e}short\nafter",
        "x".repeat(200)
    );
    let gpu = GpuRenderer::new(400, 400).unwrap();
    let mut ui = Ui::new(400., 400.);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let value = ui.signal(text.clone());
    let view = ui.mount(
        text_area("Note", value)
            .id("note")
            .w(360.)
            .h(360.)
            .p(0.)
            .text_size(16.)
            .line_height(24.)
            .text_wrap(true),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.find("note").unwrap()).unwrap();
    editor.set_line_insets(Some(Rc::new(|i| match i {
        1 => LineInset {
            left: 30.,
            above: 22.,
            below: 22.,
            nowrap: true,
            scroll_group: Some(3),
            ..LineInset::default()
        },
        _ => LineInset::default(),
    })));
    editor.set_line_marks(Some(Rc::new(|i| match i {
        1 => vec![zgx::widgets::LineMark {
            x: 10.,
            width: f32::INFINITY,
            height: f32::INFINITY,
            bottom: 4.,
            radius: 10.,
            border_width: 0.5,
            pinned: true,
            ..zgx::widgets::LineMark::default()
        }],
        _ => Vec::new(),
    })));
    editor.set_span_highlighter(Some(Rc::new(|line: &str, state| {
        let spans = if line.starts_with("const") {
            let mut style = zgx::widgets::SpanStyle::default();
            style.mono = true;
            style.size = Some(14.);
            style.line_height = Some(20.);
            let mut token = style.clone();
            token.color = Some(zgx::scene::Color(200, 30, 30, 255));
            vec![(0..line.len(), style), (0..5, token)]
        } else {
            Vec::new()
        };
        (spans, state)
    })));
    ui.prepare_frame();
    let at = |s: &str| text.find(s).unwrap();
    let first = editor.range_boxes(at("const")..at("const") + 1)[0];
    let second = editor.range_boxes(at("short")..at("short") + 1)[0];
    let caret = editor.text_area(at("short")..at("short")).unwrap();
    assert!(
        second.y >= first.y + 19. && (second.x - first.x).abs() < 1.,
        "the code line after the break is the next row, at the block's left: {first:?} then {second:?}"
    );
    assert!(
        (caret.y - second.y).abs() < 1. && caret.x < first.x + 2.,
        "the caret there on the second row: {caret:?} ({second:?})"
    );
    let after = editor.range_boxes(at("after")..at("after") + 1)[0];
    assert!(
        after.y >= second.y + 20. + 22. - 1.,
        "the next line below the block's last row and its room: {second:?} then {after:?}"
    );
}

fn rows(mono: bool) {
    let text = format!(
        "intro\nconst x = \"{}\";\u{1e}short\nafter",
        "x".repeat(200)
    );
    let (_gpu, _ui, editor) = note(&text, mono);
    let at = |s: &str| text.find(s).unwrap();
    // As an input method asks (`text_area`): the caret before "short".
    let caret = editor.text_area(at("short")..at("short")).unwrap();
    let first = editor.range_boxes(at("const")..at("const") + 1)[0];
    let second = editor.range_boxes(at("short")..at("short") + 1)[0];
    assert!(
        (second.y - (first.y + ROW)).abs() < 1.,
        "the code line after the break is the next row: {first:?} then {second:?}"
    );
    assert!(
        (second.x - first.x).abs() < 1.,
        "starting at the block's left, as the first: {first:?} then {second:?}"
    );
    assert!(
        (caret.y - second.y).abs() < 1. && caret.x < first.x + 2.,
        "the caret there on the second row: {caret:?}"
    );
    // The block as tall as its two rows: the line after it below them.
    let after = editor.range_boxes(at("after")..at("after") + 1)[0];
    assert!(
        after.y >= second.y + ROW - 1.,
        "the next line below the block's last row: {second:?} then {after:?}"
    );
    // A hit on the second row lands in it.
    let hit = editor
        .offset_at(second.x + 2., second.y + ROW / 2.)
        .unwrap();
    assert!(
        (at("short")..at("short") + 2).contains(&hit),
        "a hit on the second row is in it: {hit} for {:?}",
        at("short")
    );
}

/// As a note loads: the code block's insets set before its spans, shaped
/// plain first; its spans arriving frames later; its insets set again.
/// Its rows are the same as shaped once.
#[test]
fn a_code_blocks_rows_hold_as_its_spans_arrive_after_it_shows() {
    let text = format!(
        "intro\nconst x = \"{}\";\u{1e}short\nafter",
        "x".repeat(200)
    );
    let gpu = GpuRenderer::new(400, 400).unwrap();
    let mut ui = Ui::new(400., 400.);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let value = ui.signal(text.clone());
    let view = ui.mount(
        text_area("Note", value)
            .id("note")
            .w(360.)
            .h(360.)
            .p(0.)
            .text_size(16.)
            .line_height(24.)
            .text_wrap(true),
    );
    let editor = ui.editor(view.find("note").unwrap()).unwrap();
    let insets = || -> zgx::widgets::LineInsets {
        Rc::new(|i| match i {
            1 => LineInset {
                left: 30.,
                above: 22.,
                below: 22.,
                nowrap: true,
                scroll_group: Some(3),
                ..LineInset::default()
            },
            _ => LineInset::default(),
        })
    };
    editor.set_line_insets(Some(insets()));
    for _ in 0..3 {
        ui.prepare_frame();
    }
    editor.set_span_highlighter(Some(Rc::new(|line: &str, state| {
        let spans = if line.starts_with("const") {
            let mut style = zgx::widgets::SpanStyle::default();
            style.mono = true;
            style.size = Some(14.);
            style.line_height = Some(20.);
            vec![(0..line.len(), style)]
        } else {
            Vec::new()
        };
        (spans, state)
    })));
    for _ in 0..3 {
        ui.prepare_frame();
    }
    editor.set_line_insets(Some(insets()));
    for _ in 0..3 {
        ui.prepare_frame();
    }
    let at = |s: &str| text.find(s).unwrap();
    let first = editor.range_boxes(at("const")..at("const") + 1)[0];
    let second = editor.range_boxes(at("short")..at("short") + 1)[0];
    let caret = editor.text_area(at("short")..at("short")).unwrap();
    assert!(
        second.y >= first.y + 19. && (second.x - first.x).abs() < 1.,
        "the code line after the break is the next row: {first:?} then {second:?}"
    );
    assert!(
        (caret.y - second.y).abs() < 1. && caret.x < first.x + 2.,
        "the caret there on the second row: {caret:?} ({second:?})"
    );
    let after = editor.range_boxes(at("after")..at("after") + 1)[0];
    assert!(
        after.y >= second.y + 20. + 22. - 1.,
        "the next line below the block's last row and its room: {second:?} then {after:?}"
    );
}
