//! A code block in a text area (the web's `white-space: pre` and `overflow-x: auto`): its lines never wrap
//! (`LineInset::nowrap`) and scroll sideways together in a scroll group, its surface and line numbers pinned
//! (`LineMark::pinned`); a mark may draw over the text (`LineMark::over_text`, its actions).
use std::rc::Rc;
use zgx::{
    compose::prelude::*,
    scene::{NodeId, NodeKind},
    widgets::{EditorHandle, LineInset, LineMark, Ui},
};

const GROUP: u32 = 3;

/// A note 400 points wide: a prose line, a two-line code block (lines 1 and 2, the first far wider than the note),
/// and a long prose line that wraps.
fn text() -> String {
    format!(
        "intro\nlet wide = \"{}\";\nshort\n{}",
        "x".repeat(200),
        "word ".repeat(60)
    )
}

fn note(marks: impl Fn(usize) -> Vec<LineMark> + 'static) -> (Ui, EditorHandle) {
    let mut ui = Ui::new(600., 400.);
    let value = ui.signal(text());
    let view = ui.mount(
        text_area("Note", value)
            .w(400.)
            .h(300.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(true),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_line_insets(Some(Rc::new(|i| match i {
        1 | 2 => LineInset {
            left: 30.,
            nowrap: true,
            scroll_group: Some(GROUP),
            ..LineInset::default()
        },
        _ => LineInset::default(),
    })));
    editor.set_line_marks(Some(Rc::new(marks)));
    ui.prepare_frame();
    (ui, editor)
}

fn line_range(i: usize) -> std::ops::Range<usize> {
    let text = text();
    let start: usize = text.split('\n').take(i).map(|l| l.len() + 1).sum();
    start..start + text.split('\n').nth(i).unwrap().len()
}

/// The quads of marks drawn with border width `width`: where each shows across.
fn marks_x(ui: &Ui, editor: &EditorHandle, width: f32) -> Vec<f32> {
    let scene = ui.scene.borrow();
    let mut found = Vec::new();
    let mut pending = vec![editor.node];
    while let Some(id) = pending.pop() {
        if matches!(scene.kind(id), NodeKind::Quad(q) if q.border_width == width) {
            found.push(scene.transform(id).x);
        }
        pending.extend_from_slice(scene.children(id));
    }
    found
}

#[test]
fn a_code_blocks_wide_line_does_not_wrap_and_scrolls_with_its_group() {
    let (ui, editor) = note(|_| Vec::new());
    let wide = editor.range_boxes(line_range(1));
    assert_eq!(wide.len(), 1, "one row, not wrapped: {wide:?}");
    assert!(wide[0].width > 400., "as wide as its text: {wide:?}");
    // Prose outside the block wraps as before.
    assert!(editor.range_boxes(line_range(3)).len() >= 2, "prose wraps");
    let (view, content) = editor.group_extent(GROUP).expect("a group");
    assert!(content > view, "it scrolls: {view} of {content}");
    let before = editor.range_boxes(line_range(1).start..line_range(1).start + 1)[0];
    editor.set_group_scroll(GROUP, 100.);
    ui.prepare_frame();
    let after = editor.range_boxes(line_range(1).start..line_range(1).start + 1)[0];
    assert!(
        (before.x - after.x - 100.).abs() < 0.5,
        "its text moved by the offset: {before:?} {after:?}"
    );
    // The prose did not move.
    let prose = editor.range_boxes(0..1)[0];
    assert!(prose.x.abs() < 0.5, "{prose:?}");
}

#[test]
fn pinned_marks_stay_as_the_group_scrolls_and_others_move() {
    let (ui, editor) = note(|i| match i {
        1 | 2 => vec![
            // The block's surface, full width, pinned.
            LineMark {
                x: 0.,
                width: f32::INFINITY,
                border_width: 1.,
                pinned: true,
                ..LineMark::default()
            },
            // A mark that moves with the text.
            LineMark {
                x: 30.,
                width: 10.,
                border_width: 2.,
                ..LineMark::default()
            },
        ],
        _ => Vec::new(),
    });
    assert_eq!(marks_x(&ui, &editor, 1.), [0., 0.]);
    assert_eq!(marks_x(&ui, &editor, 2.), [30., 30.]);
    editor.set_group_scroll(GROUP, 100.);
    ui.prepare_frame();
    assert_eq!(marks_x(&ui, &editor, 1.), [0., 0.], "pinned: still");
    assert_eq!(
        marks_x(&ui, &editor, 2.),
        [-70., -70.],
        "moved with the text"
    );
}

/// The node showing a code line, and the marks' nodes, in the order they paint.
fn paint_order(ui: &Ui, nodes: &[NodeId]) -> Vec<usize> {
    let scene = ui.scene.borrow();
    let order: Vec<NodeId> = scene.paint_items().map(|item| item.id).collect();
    nodes
        .iter()
        .map(|id| order.iter().position(|o| o == id).expect("painted"))
        .collect()
}

#[test]
fn an_over_text_mark_paints_above_the_text_and_others_below() {
    let (ui, editor) = note(|i| match i {
        1 => vec![
            LineMark {
                x: 300.,
                width: 60.,
                height: 20.,
                border_width: 3.,
                fill: zgx::scene::Color(200, 200, 200, 255),
                over_text: true,
                ..LineMark::default()
            },
            LineMark {
                x: 0.,
                width: f32::INFINITY,
                border_width: 4.,
                fill: zgx::scene::Color(240, 240, 240, 255),
                ..LineMark::default()
            },
        ],
        _ => Vec::new(),
    });
    let scene = ui.scene.borrow();
    let mut text = None;
    let (mut over, mut under) = (None, None);
    let mut pending = vec![editor.node];
    while let Some(id) = pending.pop() {
        match scene.kind(id) {
            NodeKind::Text { text: t, .. } if t.starts_with("let wide") => text = Some(id),
            NodeKind::Quad(q) if q.border_width == 3. => over = Some(id),
            NodeKind::Quad(q) if q.border_width == 4. => under = Some(id),
            _ => {}
        }
        pending.extend_from_slice(scene.children(id));
    }
    drop(scene);
    let [text, over, under] = paint_order(&ui, &[text.unwrap(), over.unwrap(), under.unwrap()])[..]
    else {
        unreachable!()
    };
    assert!(
        under < text && text < over,
        "under {under}, text {text}, over {over}"
    );
}

/// With a group scrollbar (`View::group_scrollbar`): none until a group moves, then one across the bottom of the
/// block that moved, from its frame's left, as tall as its lines.
#[test]
fn a_group_scrollbar_shows_at_the_block_that_moved() {
    let mut ui = Ui::new(600., 400.);
    let value = ui.signal(text());
    let view = ui.mount(
        text_area("Note", value)
            .w(400.)
            .h(300.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(true)
            .group_scrollbar(),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_line_insets(Some(Rc::new(|i| match i {
        1 | 2 => LineInset {
            left: 30.,
            nowrap: true,
            scroll_group: Some(GROUP),
            ..LineInset::default()
        },
        _ => LineInset::default(),
    })));
    ui.prepare_frame();
    let placed = |ui: &Ui| {
        let scene = ui.scene.borrow();
        scene
            .children(editor.node)
            .iter()
            .map(|id| scene.transform(*id))
            .any(|t| (t.x, t.y) == (30., 20.))
    };
    assert!(!placed(&ui), "no group moved: no bar placed");
    editor.set_group_scroll(GROUP, 100.);
    ui.prepare_frame();
    assert!(placed(&ui), "at the block: its left, its first line's top");
}
