//! A text area's atoms styled as a mention (`EditorHandle::set_atom_style`):
//! the name's colour, weight and underline in its own colour and place, and
//! a leading icon in room before its first word, which wraps with it; still
//! one caret unit.
use std::sync::Arc;
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers},
    scene::{Color, NodeId, NodeKind, Rect},
    svg::SvgData,
    text_edit::Atom,
    widgets::{AtomIcon, AtomStyle, AtomUnderline, EditorHandle, Ui},
};

const INK: Color = Color(28, 28, 28, 255);
const LINE: Color = Color(28, 28, 28, 92);

fn icon() -> Arc<SvgData> {
    Arc::new(
        SvgData::new(
            br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="8"/></svg>"#
                .to_vec(),
        )
        .unwrap(),
    )
}

fn mention() -> AtomStyle {
    AtomStyle {
        color: Some(INK),
        weight: Some(500),
        underline: Some(AtomUnderline {
            color: Some(LINE),
            offset: Some(2.),
        }),
        icon: Some(AtomIcon {
            svg: icon(),
            size: 16.,
            gap: 4.,
        }),
        svg: None,
    }
}

/// A text area `width` wide of `text`, its atom over `atom` styled by
/// `style` (none: unstyled).
fn field(
    text: &str,
    atom: (usize, usize),
    width: f32,
    style: Option<AtomStyle>,
) -> (Ui, EditorHandle, u64) {
    let mut ui = Ui::new(400., 300.);
    let value = ui.signal(text.to_string());
    let root = ui.mount(
        column().size(400., 300.).child(
            text_area("Message", value)
                .id("field")
                .size(width, 200.)
                .text_size(14.)
                .line_height(20.)
                .text_wrap(true),
        ),
    );
    ui.prepare_frame();
    let editor = ui.editor(root.find("field").unwrap()).unwrap();
    let id = editor.new_atom_id();
    editor.set_atoms(vec![Atom::new(id, atom.0..atom.1)]);
    editor.set_atom_style(style);
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    (ui, editor, id)
}

fn descendants(ui: &Ui, node: NodeId) -> Vec<NodeId> {
    let scene = ui.scene.borrow();
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(id) = stack.pop() {
        out.push(id);
        stack.extend(scene.children(id).iter().copied());
    }
    out
}

/// The icons drawn in the text area, where they show.
fn icons(ui: &Ui, editor: &EditorHandle) -> Vec<Rect> {
    let nodes = descendants(ui, editor.node);
    let scene = ui.scene.borrow();
    nodes
        .into_iter()
        .filter(|id| matches!(scene.kind(*id), NodeKind::Svg(_)))
        .map(|id| scene.transformed_bounds(id))
        .collect()
}

/// The run drawing byte `at`, from the text area's styled lines.
fn run_at(ui: &Ui, editor: &EditorHandle, label: &str) -> zgx::rich_text::TextRun {
    let nodes = descendants(ui, editor.node);
    let scene = ui.scene.borrow();
    for id in nodes {
        if let NodeKind::RichText { text } = scene.kind(id)
            && let Some(at) = text.text().find(label)
            && let Some(run) = text.runs().iter().find(|r| r.range.contains(&at))
        {
            return run.clone();
        }
    }
    panic!("no styled line shows {label:?}");
}

fn key(ui: &mut Ui, key: Key) {
    ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.prepare_frame();
}

const TEXT: &str = "ask Ada Lovelace about it";

#[test]
fn a_mention_draws_its_colour_weight_underline_and_icon() {
    let (ui, editor, _) = field(TEXT, (4, 16), 300., Some(mention()));
    let run = run_at(&ui, &editor, "Ada");
    assert_eq!(run.color, INK);
    assert_eq!(run.font.weight, 500);
    let line = run.underline.expect("underlined");
    assert_eq!(line.color, Some(LINE));
    assert_eq!(line.offset_px(), Some(2.));
    // One point here; the font's own thickness comes with the note editor's
    // decorations (`Decoration::is_auto` on the Anara pin).
    assert_eq!(line.thickness(), 1.);
    // The text before it is the field's own.
    let before = run_at(&ui, &editor, "ask");
    assert!(before.underline.is_none() && before.font.weight != 500);
    // Its icon in the room before its first letter.
    let first = editor.range_boxes(4..5)[0];
    let shown = icons(&ui, &editor);
    assert_eq!(shown.len(), 1, "{shown:?}");
    let icon = shown[0];
    assert_eq!((icon.width, icon.height), (16., 16.));
    assert!(
        (icon.x - first.x).abs() < 0.5,
        "at the room's start: {icon:?} {first:?}"
    );
    assert!(
        first.width >= 20.,
        "room for the icon and its gap: {first:?}"
    );
    let middle = |r: Rect| r.y + r.height / 2.;
    assert!(
        (middle(icon) - middle(first)).abs() < 1.,
        "centred on its line"
    );
}

#[test]
fn a_mention_wraps_and_its_icon_stays_with_its_first_word() {
    // Narrow: the name starts near the line's end and its first word goes
    // to the next line with its icon.
    let text = "please ask Ada Lovelace now";
    let (ui, editor, id) = field(text, (11, 23), 120., Some(mention()));
    let atom = editor.atoms().into_iter().find(|a| a.id == id).unwrap();
    let pieces = editor.range_boxes(atom.range.clone());
    assert!(pieces.len() >= 2, "the name wraps: {pieces:?}");
    let first = editor.range_boxes(11..12)[0];
    let icon = icons(&ui, &editor)[0];
    assert!(
        icon.y >= first.y - 0.5 && icon.y + icon.height <= first.y + first.height + 0.5,
        "the icon is on its first word's line: {icon:?} {first:?}"
    );
}

#[test]
fn a_wrapped_mention_is_one_caret_unit_and_deletes_whole() {
    let text = "please ask Ada Lovelace now";
    let (mut ui, editor, _) = field(text, (11, 23), 120., Some(mention()));
    ui.input.focus(&ui.scene, Some(editor.node));
    editor.set_selection(11, 11);
    ui.prepare_frame();
    key(&mut ui, Key::ArrowRight);
    assert_eq!(editor.selection(), (23, 23), "past it in one step");
    key(&mut ui, Key::Backspace);
    assert_eq!(editor.editor.borrow().text(), "please ask  now");
    assert!(editor.atoms().is_empty());
    assert!(icons(&ui, &editor).is_empty(), "its icon goes with it");
}

#[test]
fn an_unstyled_atom_draws_no_icon_and_takes_no_room() {
    let (ui, editor, _) = field(TEXT, (4, 16), 300., None);
    assert!(icons(&ui, &editor).is_empty());
    let first = editor.range_boxes(4..5)[0];
    assert!(first.width < 16., "no room before it: {first:?}");
}

#[test]
fn an_atoms_own_icon_wins_over_the_text_areas() {
    let (ui, editor, id) = field(TEXT, (4, 16), 300., Some(mention()));
    let own = icon();
    let mut atom = editor.atoms().into_iter().find(|a| a.id == id).unwrap();
    atom.style = Some(Arc::new(AtomStyle {
        icon: Some(AtomIcon {
            svg: own.clone(),
            size: 12.,
            gap: 2.,
        }),
        ..AtomStyle::default()
    }));
    editor.set_atoms(vec![atom]);
    ui.prepare_frame();
    let shown = icons(&ui, &editor);
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].width, 12.);
    // Its other fields are the text area's.
    assert_eq!(run_at(&ui, &editor, "Ada").color, INK);
}
