//! An atom shown as a picture (`AtomStyle::svg`, an equation): its text one
//! box of the picture's size, one caret step, its line as tall as the box
//! needs, its text kept for copy.
use std::sync::Arc;
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers},
    scene::{NodeId, NodeKind, Rect},
    svg::SvgData,
    text_edit::Atom,
    widgets::{AtomStyle, AtomSvg, EditorHandle, Ui},
};

const SIZE: f32 = 14.;

fn picture() -> Arc<SvgData> {
    Arc::new(
        SvgData::new(
            br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10"><rect width="20" height="10"/></svg>"#
                .to_vec(),
        )
        .unwrap(),
    )
}

/// `[width, height, descent]` in EM.
fn equation(extents: [f32; 3]) -> AtomStyle {
    AtomStyle {
        svg: Some(AtomSvg {
            svg: picture(),
            width: extents[0],
            height: extents[1],
            descent: extents[2],
        }),
        ..AtomStyle::default()
    }
}

fn field(text: &str, atom: (usize, usize), extents: [f32; 3]) -> (Ui, EditorHandle) {
    let mut ui = Ui::new(400., 300.);
    let value = ui.signal(text.to_string());
    let root = ui.mount(
        column().size(400., 300.).child(
            text_area("Note", value)
                .id("field")
                .size(300., 240.)
                .text_size(SIZE)
                .line_height(20.)
                .text_wrap(true),
        ),
    );
    ui.prepare_frame();
    let editor = ui.editor(root.find("field").unwrap()).unwrap();
    let id = editor.new_atom_id();
    editor.set_atoms(vec![Atom::new(id, atom.0..atom.1)]);
    editor.set_atom_style(Some(equation(extents)));
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    (ui, editor)
}

fn pictures(ui: &Ui, editor: &EditorHandle) -> Vec<Rect> {
    let scene = ui.scene.borrow();
    let mut stack: Vec<NodeId> = vec![editor.node];
    let mut out = Vec::new();
    while let Some(id) = stack.pop() {
        if matches!(scene.kind(id), NodeKind::Svg(_)) {
            out.push(scene.transformed_bounds(id));
        }
        stack.extend(scene.children(id).iter().copied());
    }
    out
}

fn key(ui: &mut Ui, key: Key) {
    ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.prepare_frame();
}

const TEXT: &str = "area is x^2 here";

#[test]
fn an_equation_shows_as_one_box_of_its_size_with_its_picture_in_it() {
    let (ui, editor) = field(TEXT, (8, 11), [2., 1., 0.25]);
    // Its first character the box, the rest of its text none.
    let shown: Vec<_> = editor
        .range_boxes(8..11)
        .into_iter()
        .filter(|r| r.width > 0.)
        .collect();
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(
        (shown[0].width - 2. * SIZE).abs() < 0.5,
        "two EM wide: {shown:?}"
    );
    // The text after it starts where the box ends.
    let after = editor.range_boxes(12..13)[0];
    assert!(
        after.x >= shown[0].x + shown[0].width,
        "{after:?} after {shown:?}"
    );
    let drawn = pictures(&ui, &editor);
    assert_eq!(drawn.len(), 1);
    assert_eq!((drawn[0].width, drawn[0].height), (2. * SIZE, SIZE));
    assert!(
        (drawn[0].x - shown[0].x).abs() < 0.5,
        "in its box: {drawn:?} {shown:?}"
    );
    // Its text is still the text's.
    assert_eq!(editor.editor.borrow().text(), TEXT);
}

#[test]
fn the_caret_steps_over_an_equation_in_one() {
    let (mut ui, editor) = field(TEXT, (8, 11), [2., 1., 0.25]);
    ui.input.focus(&ui.scene, Some(editor.node));
    editor.set_selection(8, 8);
    ui.prepare_frame();
    key(&mut ui, Key::ArrowRight);
    assert_eq!(editor.selection(), (11, 11));
    key(&mut ui, Key::ArrowLeft);
    assert_eq!(editor.selection(), (8, 8));
    editor.select_all();
    assert_eq!(editor.copy(), TEXT, "copy reads its text");
}

#[test]
fn a_tall_equation_makes_its_line_taller() {
    let (_ui, editor) = field("a\nb x^2 c\nd", (4, 7), [2., 2.5, 0.5]);
    let plain = editor.range_boxes(0..1)[0];
    let tall = editor.range_boxes(2..3)[0];
    assert!(
        tall.height >= 2.5 * SIZE - 0.5,
        "its line holds it: {tall:?}, a plain one {plain:?}"
    );
    let below = editor.range_boxes(10..11)[0];
    assert!(
        below.y >= tall.y + tall.height - 0.5,
        "the next line below it"
    );
}

#[test]
fn a_press_on_a_pictures_right_half_lands_after_it() {
    // Long source text: its bytes say nothing of where its box shows.
    let text = r"see \frac{a+b}{c+d} now";
    let (mut ui, editor) = field(text, (4, 19), [2., 1.5, 0.4]);
    ui.input.focus(&ui.scene, Some(editor.node));
    let shown: Vec<_> = editor
        .range_boxes(4..19)
        .into_iter()
        .filter(|r| r.width > 0.)
        .collect();
    let press = |ui: &mut Ui, x: f32, y: f32| {
        ui.dispatch(InputEvent::PointerDown {
            x,
            y,
            button: zgx::input::PointerButton::Primary,
        });
        ui.dispatch(InputEvent::PointerUp {
            x,
            y,
            button: zgx::input::PointerButton::Primary,
        });
        ui.prepare_frame();
    };
    let b = shown[0];
    let y = b.y + b.height / 2.;
    press(&mut ui, b.x + b.width * 0.75, y);
    assert_eq!(editor.selection(), (19, 19), "its right half: after it");
    std::thread::sleep(std::time::Duration::from_millis(600));
    press(&mut ui, b.x + b.width * 0.25, y);
    assert_eq!(editor.selection(), (4, 4), "its left half: before it");
}

#[test]
fn a_highlighters_picture_boxes_each_replacement_character() {
    use std::{cell::Cell, rc::Rc};
    use zgx::widgets::SpanStyle;
    let text = "a \u{FFFC}\u{FFFC} b";
    let mut ui = Ui::new(400., 300.);
    let value = ui.signal(text.to_string());
    let named: Rc<Cell<Option<std::num::NonZeroU32>>> = Rc::default();
    let name = named.clone();
    let root = ui.mount(
        column().size(400., 300.).child(
            text_area("Note", value)
                .id("field")
                .size(300., 200.)
                .text_size(SIZE)
                .line_height(20.)
                .span_highlighter(move |line, open| {
                    let spans = match (name.get(), line.find('\u{FFFC}')) {
                        (Some(id), Some(at)) => {
                            vec![(at..at + 6, SpanStyle::default().inline_svg(id))]
                        }
                        _ => Vec::new(),
                    };
                    (spans, open)
                }),
        ),
    );
    ui.prepare_frame();
    let editor = ui.editor(root.find("field").unwrap()).unwrap();
    named.set(Some(editor.inline_svg(picture(), [1., 1., 0.25])));
    editor.rehighlight();
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    for range in [2..5, 5..8] {
        let shown = editor.range_boxes(range.clone());
        assert!(
            (shown[0].width - SIZE).abs() < 0.5,
            "{range:?}: one EM: {shown:?}"
        );
    }
    assert_eq!(pictures(&ui, &editor).len(), 2, "a picture in each");
}
