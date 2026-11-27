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
        let mark = LineMark::new(0., 0., 20., 20., Color(0, 0, 0, 0));
        vec![
            mark.label((format!("{}.", i + 1).into(), RULE, 10.))
                .tabular(true),
        ]
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

/// A mark's decoration gives its quad per-corner radii and per-side borders
/// (a table's outer cells).
#[test]
fn a_marks_decoration_gives_its_quad_corners_and_side_borders() {
    use std::sync::Arc;
    use zgx::decoration::{Corners, Decoration};
    let (mut ui, editor) = area("one\ntwo");
    let decoration = Arc::new(Decoration {
        corners: Some(Corners {
            top_left: 15.,
            ..Corners::default()
        }),
        border_widths: Some(zgx::scene::Insets {
            top: 1.,
            right: 1.,
            bottom: 0.,
            left: 1.,
        }),
        ..Decoration::default()
    });
    let shown = decoration.clone();
    editor.set_line_marks(Some(Rc::new(move |i, _, _| {
        let mark = LineMark::new(0., 0., 100., 20., WASH);
        vec![if i == 0 {
            mark.decoration(shown.clone())
        } else {
            mark
        }]
    })));
    frame(&mut ui);
    let scene = ui.scene.borrow();
    let mut stack: Vec<NodeId> = vec![editor.node];
    let mut found = Vec::new();
    while let Some(id) = stack.pop() {
        if let NodeKind::Quad(quad) = scene.kind(id)
            && quad.fill == WASH
        {
            found.push(quad.decoration.clone());
        }
        stack.extend(scene.children(id).iter().copied());
    }
    found.sort_by_key(Option::is_none);
    assert_eq!(found, [Some(decoration), None]);
}

/// A mark aligns across its line's width, `x` then offsetting it; an SVG
/// shows in place of its box.
#[test]
fn marks_align_across_their_line_and_show_an_svg() {
    use std::sync::Arc;
    use zgx::{svg::SvgData, text_layout::TextAlign};
    let (mut ui, editor) = area("one\ntwo\nthree\nfour\nfive");
    let svg = Arc::new(
        SvgData::new(br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"/>"##.to_vec())
            .unwrap(),
    );
    let shown = svg.clone();
    editor.set_line_marks(Some(Rc::new(move |i, _, _| {
        let mark = LineMark::new(4., 0., 40., 20., RULE);
        vec![match i {
            0 => mark,
            1 => mark.align(TextAlign::Center),
            2 => mark.align(TextAlign::Right),
            3 => mark.svg(shown.clone()),
            // As wide as the line.
            _ => LineMark::new(0., 0., f32::INFINITY, 20., WASH),
        }]
    })));
    frame(&mut ui);
    let width = quads(&ui, &editor, WASH)[0].width;
    let at: Vec<f32> = quads(&ui, &editor, RULE).iter().map(|r| r.x).collect();
    assert_eq!(at.len(), 3, "the fourth is an SVG: {at:?}");
    let left = at[0] - 4.;
    assert_eq!(at[1] - left, (width - 40.) / 2. + 4.);
    assert_eq!(at[2] - left, width - 40. - 4.);
    let scene = ui.scene.borrow();
    let mut stack: Vec<NodeId> = vec![editor.node];
    let mut svgs = 0;
    while let Some(id) = stack.pop() {
        svgs += usize::from(matches!(scene.kind(id), NodeKind::Svg(s) if **s == *svg));
        stack.extend(scene.children(id).iter().copied());
    }
    assert_eq!(svgs, 1);
}

/// What marks' nodes under `editor` draw: each one's kind, place and turn.
fn drawn(ui: &Ui, editor: &EditorHandle) -> Vec<String> {
    let scene = ui.scene.borrow();
    let mut stack: Vec<NodeId> = vec![editor.node];
    let mut found = Vec::new();
    while let Some(id) = stack.pop() {
        let mark = match scene.kind(id) {
            NodeKind::Quad(quad) => quad.fill == WASH || quad.fill == RULE,
            NodeKind::Text { text, .. } => text.ends_with(')'),
            _ => false,
        };
        if mark {
            assert!(scene.children(id).is_empty());
            found.push(format!(
                "{:?} {:?} {:?} {:?}",
                scene.kind(id),
                scene.transformed_bounds(id),
                scene.local_transform(id),
                scene.clip_inset(id),
            ));
        }
        stack.extend(scene.children(id).iter().copied());
    }
    found.sort();
    found
}
/// Plain marks (a wash, a rule, a number) draw as they did where no mark has
/// had a look, beside marks that have one, and once those are gone.
#[test]
fn plain_marks_draw_the_same_whatever_others_have() {
    use std::{cell::Cell, sync::Arc};
    use zgx::{decoration::Decoration, image::ImageData, widgets::MarkImage};
    let looks = Rc::new(Cell::new(false));
    let (mut ui, editor) = area("one\ntwo\nthree");
    let look = looks.clone();
    let image = Arc::new(ImageData::new(1, 1, vec![9_u8; 4]).unwrap());
    editor.set_line_marks(Some(Rc::new(move |i, _, _| {
        let mut marks = vec![
            LineMark::new(0., 0., f32::INFINITY, f32::INFINITY, WASH).pinned(true),
            LineMark::new(2., 0., 3., f32::INFINITY, RULE).radius(1.),
            LineMark::new(0., 0., 20., 20., Color(0, 0, 0, 0))
                .label((format!("{i})").into(), RULE, 10.))
                .over_text(true),
        ];
        if look.get() {
            let box_ = LineMark::new(9., 0., 8., 8., Color(1, 2, 3, 255));
            // Before the plain ones, so nodes change hands.
            marks.insert(0, box_.clone().image(MarkImage::cover(image.clone())));
            marks.insert(0, box_.clone().rotate(90.));
            marks.insert(0, box_.decoration(Arc::new(Decoration::default())));
        }
        marks
    })));
    frame(&mut ui);
    let plain = drawn(&ui, &editor);
    assert_eq!(plain.len(), 9);
    for on in [true, false, true, false] {
        looks.set(on);
        editor.refresh();
        for _ in 0..3 {
            frame(&mut ui);
        }
        assert_eq!(drawn(&ui, &editor), plain, "with looks {on}");
    }
}
