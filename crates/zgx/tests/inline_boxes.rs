//! Inline boxes: views in a paragraph's lines, atomic as CSS's inline-block.
//! The headless core's approximate layout puts each grapheme in a cell 0.6
//! of the text size wide, its baseline 0.8 em below half its leading.
use std::{cell::Cell, rc::Rc};
use zgx::{
    compose::{ViewHandle, prelude::*},
    input::{InputEvent, Key, Modifiers, PointerButton},
    rich_text::{InlineAlign, RichText},
    scene::{NodeId, NodeKind, Rect},
    text_layout::LineHeight,
    widgets::Ui,
};

/// At size 10, a character 6 wide; the line's baseline this far down.
const SIZE: f32 = 10.;
fn line() -> f32 {
    LineHeight::NORMAL.resolve(SIZE)
}
fn baseline() -> f32 {
    (line() - SIZE) / 2. + 0.8 * SIZE
}
fn chip(id: &str, width: f32, height: f32) -> View {
    row().id(id).w(width).h(height)
}
fn paragraph() -> RichTextView {
    rich_text()
        .id("text")
        .w(300.)
        .text_size(SIZE)
        .text_wrap(true)
}
fn the_paragraph(ui: &Ui, handle: &ViewHandle) -> NodeId {
    ui.scene.borrow().children(handle.find("text").unwrap())[0]
}
fn laid_text(ui: &Ui, node: NodeId) -> std::sync::Arc<RichText> {
    match ui.scene.borrow().kind(node) {
        NodeKind::RichText { text } => text.clone(),
        _ => panic!("a paragraph"),
    }
}
fn bounds(ui: &Ui, handle: &ViewHandle, id: &str) -> Rect {
    ui.scene.borrow().bounds(handle.find(id).unwrap())
}
fn click(ui: &mut Ui, x: f32, y: f32) {
    let button = PointerButton::Primary;
    ui.dispatch(InputEvent::PointerDown { x, y, button });
    ui.dispatch(InputEvent::PointerUp { x, y, button });
}

/// A view takes its room in the line, on the baseline; the text after it
/// moves over; its line stays the text's while it fits.
#[test]
fn an_inline_box_sits_on_the_baseline_and_moves_the_text_after_it() {
    let mut ui = Ui::new(400., 200.);
    let handle = ui.mount(
        paragraph()
            .child(text_span("Ask "))
            .inline(chip("chip", 30., 6.))
            .child(text_span(" now"))
            .into(),
    );
    ui.prepare_frame();
    let node = the_paragraph(&ui, &handle);
    let at = ui.scene.borrow().bounds(node);
    let placed = bounds(&ui, &handle, "chip");
    assert_eq!(placed.x - at.x, 24., "after four characters");
    assert_eq!((placed.width, placed.height), (30., 6.));
    assert!(
        (placed.y + 6. - at.y - baseline()).abs() < 0.01,
        "{placed:?}"
    );
    assert_eq!(at.height, line(), "the line is the text's");
    let text = laid_text(&ui, node);
    assert_eq!(text.text(), "Ask \u{FFFC} now");
    let b = text.inline_boxes()[0];
    assert_eq!((b.at, b.width, b.height, b.baseline), (4, 30., 6., 6.));
    assert_eq!(at.width, 8. * 6. + 30., "eight characters and the box");
    // It paints, over the text.
    let scene = ui.scene.borrow();
    let items: Vec<_> = scene.paint_items().map(|item| item.id).collect();
    let (text_at, chip_at) = (
        items.iter().position(|id| *id == node),
        items
            .iter()
            .position(|id| *id == handle.find("chip").unwrap()),
    );
    assert!(text_at < chip_at && chip_at.is_some(), "{items:?}");
}

/// A box taller than the text grows its line and moves what follows; a
/// box's margins are part of its room.
#[test]
fn a_tall_box_grows_its_line_and_margins_take_room() {
    let mut ui = Ui::new(400., 200.);
    let handle = ui.mount(
        column()
            .child(
                paragraph()
                    .child(text_span("a"))
                    .inline(chip("chip", 20., 40.).ml(3.).mr(5.))
                    .child(text_span("b")),
            )
            .child(chip("below", 10., 10.)),
    );
    ui.prepare_frame();
    let node = the_paragraph(&ui, &handle);
    let at = ui.scene.borrow().bounds(node);
    let placed = bounds(&ui, &handle, "chip");
    assert_eq!(placed.x - at.x, 6. + 3., "after a character and its margin");
    let text_below = line() - baseline();
    assert!((at.height - (40. + text_below)).abs() < 0.01, "{at:?}");
    assert!((placed.y - at.y).abs() < 0.01, "the tallest in its line");
    assert_eq!(laid_text(&ui, node).inline_boxes()[0].width, 28.);
    assert!(bounds(&ui, &handle, "below").y >= at.y + at.height);
}

/// A line breaks around a box, never through it: one that doesn't fit
/// starts the next line.
#[test]
fn a_box_that_does_not_fit_starts_the_next_line() {
    let mut ui = Ui::new(400., 200.);
    let handle = ui.mount(
        paragraph()
            .w(60.)
            .child(text_span("aaaa"))
            .inline(chip("chip", 40., 6.))
            .into(),
    );
    ui.prepare_frame();
    let at = ui.scene.borrow().bounds(the_paragraph(&ui, &handle));
    let placed = bounds(&ui, &handle, "chip");
    assert_eq!(placed.x, at.x);
    assert!(placed.y >= at.y + line() - 6., "{placed:?}");
    assert!(at.height > line());
}

/// A box with text aligns that text's baseline with the line's (a chip's
/// label with the words around it), through its padding and its row.
#[test]
fn a_chip_with_a_label_aligns_its_label_with_the_line() {
    let mut ui = Ui::new(400., 200.);
    let handle = ui.mount(
        paragraph()
            .child(text_span("Ask "))
            .inline(
                row()
                    .id("chip")
                    .p(4.)
                    .child(chip("icon", 8., 8.))
                    .child(text("label").id("label").text_size(SIZE)),
            )
            .into(),
    );
    ui.prepare_frame();
    let node = the_paragraph(&ui, &handle);
    let at = ui.scene.borrow().bounds(node);
    let label = bounds(&ui, &handle, "label");
    let line_baseline = {
        // The paragraph grew above its text by the chip's padding.
        let b = laid_text(&ui, node).inline_boxes()[0];
        let placed = bounds(&ui, &handle, "chip");
        assert!((placed.y - at.y + b.baseline - (label.y - at.y + baseline())).abs() < 0.01);
        placed.y + b.baseline
    };
    assert!(
        (label.y + baseline() - line_baseline).abs() < 0.01,
        "{label:?}"
    );
    assert!(at.height > line(), "its padding grows the line");
}

/// Each alignment places a box as CSS's `vertical-align` does.
#[test]
fn inline_align_places_a_box_as_vertical_align_does() {
    let top_of = |align: InlineAlign| {
        let mut ui = Ui::new(400., 200.);
        let handle = ui.mount(
            paragraph()
                .child(text_span("a"))
                .inline(inline_box(chip("chip", 8., 8.)).align(align))
                .into(),
        );
        ui.prepare_frame();
        let at = ui.scene.borrow().bounds(the_paragraph(&ui, &handle));
        (bounds(&ui, &handle, "chip").y - at.y, at.height)
    };
    let close = |a: f32, b: f32| (a - b).abs() < 0.01;
    assert!(close(top_of(InlineAlign::Baseline).0, baseline() - 8.));
    assert!(close(top_of(InlineAlign::Top).0, 0.));
    assert!(close(top_of(InlineAlign::Bottom).0, line() - 8.));
    assert!(close(
        top_of(InlineAlign::TextTop).0,
        baseline() - 0.8 * SIZE
    ));
    assert!(close(
        top_of(InlineAlign::TextBottom).0,
        baseline() + 0.2 * SIZE - 8.
    ));
    // Its middle half an x-height (half an em here) above the baseline.
    assert!(close(top_of(InlineAlign::Middle).0, baseline() - 2.5 - 4.));
    assert!(close(top_of(InlineAlign::Raise(1.)).0, baseline() - 9.));
}

/// A box resized moves the text after it, and a link there follows, with
/// no change of the paragraph's spans; a colour change keeps its layout.
#[test]
fn a_box_resized_moves_the_text_and_links_after_it() {
    let mut ui = Ui::new(400., 200.);
    let width = ui.signal(20_f32);
    let read = width.clone();
    let color = ui.signal(zgx::scene::Color(0, 0, 0, 255));
    let tint = color.clone();
    let handle = ui.mount(
        rich_text_signal(move || {
            vec![
                text_span("a"),
                inline_span(),
                text_span("link").text_color(tint.get()).on_click(|| {}),
            ]
        })
        .id("text")
        .w(300.)
        .text_size(SIZE)
        .text_wrap(true)
        .child(
            row()
                .id("chip")
                .h(6.)
                .reactive_style(move || zgx::style::Styles::new().w(read.get())),
        ),
    );
    ui.prepare_frame();
    let node = the_paragraph(&ui, &handle);
    let link = || {
        let owner = handle.find("text").unwrap();
        let scene = ui.scene.borrow();
        let children = scene.children(owner).to_vec();
        children
            .iter()
            .find(|n| **n != node)
            .map(|n| scene.bounds(*n))
            .unwrap()
    };
    let before = link();
    let at = ui.scene.borrow().bounds(node);
    assert_eq!(before.x - at.x, 6. + 20.);
    assert_eq!(laid_text(&ui, node).inline_boxes()[0].width, 20.);
    width.set(50.);
    ui.prepare_frame();
    assert_eq!(laid_text(&ui, node).inline_boxes()[0].width, 50.);
    let after = link();
    assert_eq!(after.x - at.x, 6. + 50., "the link moved with its text");
    // Only a colour: the box keeps its size, nothing to lay out.
    ui.scene.borrow_mut().flush();
    color.set(zgx::scene::Color(9, 9, 9, 255));
    let report = ui.scene.borrow_mut().flush();
    assert_eq!(report.layout_nodes, 0, "colour alone lays out nothing");
    assert_eq!(laid_text(&ui, node).inline_boxes()[0].width, 50.);
}

/// A box is an ordinary view: a click reaches what it holds, and its
/// handlers run; the paragraph's other text is still the paragraph's.
#[test]
fn clicks_reach_what_an_inline_box_holds() {
    let mut ui = Ui::new(400., 200.);
    let clicks = Rc::new(Cell::new(0));
    let clicked = clicks.clone();
    let handle = ui.mount(
        paragraph()
            .child(text_span("Ask "))
            .inline(
                row()
                    .id("chip")
                    .child(chip("inner", 30., 8.).on_click(move || clicked.set(clicked.get() + 1))),
            )
            .child(text_span(" now"))
            .into(),
    );
    ui.prepare_frame();
    let placed = bounds(&ui, &handle, "inner");
    click(&mut ui, placed.x + 5., placed.y + 4.);
    assert_eq!(clicks.get(), 1);
    let at = ui.scene.borrow().bounds(the_paragraph(&ui, &handle));
    click(&mut ui, at.x + 2., at.y + 2.);
    assert_eq!(clicks.get(), 1, "the text before it isn't the box");
}

/// Selecting across a box takes it as one character: the copy has what it
/// holds in its place, or what it says it copies as, and a press inside it
/// selects no text apart.
#[test]
fn selection_and_copy_take_a_box_as_its_text() {
    let mut ui = Ui::new(400., 200.);
    let handle = ui.mount(
        paragraph()
            .selectable(true)
            .child(text_span("Ask "))
            .inline(row().child(text("Wing").text_size(SIZE)))
            .child(text_span(" and "))
            .inline(inline_box(row().child(text("@id").text_size(SIZE))).alt("Sol"))
            .child(text_span(" now"))
            .into(),
    );
    ui.prepare_frame();
    let at = bounds(&ui, &handle, "text");
    ui.dispatch(InputEvent::PointerDown {
        x: at.x + 1.,
        y: at.y + 1.,
        button: PointerButton::Primary,
    });
    ui.dispatch(InputEvent::PointerUp {
        x: at.x + 1.,
        y: at.y + 1.,
        button: PointerButton::Primary,
    });
    let modifiers = Modifiers {
        meta: cfg!(target_os = "macos"),
        control: !cfg!(target_os = "macos"),
        ..Modifiers::default()
    };
    ui.dispatch_with_modifiers(
        InputEvent::KeyDown {
            key: Key::Character("a".into()),
            modifiers,
            repeat: false,
        },
        modifiers,
    );
    assert_eq!(ui.selected_text(), "Ask Wing and Sol now");
}

/// A paragraph reads with each box as its text, in its place, and the box
/// keeps its own place in the tree. Mounted and removed, it leaves nothing.
#[test]
fn a_box_reads_in_its_place() {
    let mut ui = Ui::new(400., 200.);
    let handle = ui.mount(
        paragraph()
            .child(text_span("Ask "))
            .inline(row().child(text("Wing").text_size(SIZE)))
            .child(text_span(" now"))
            .into(),
    );
    ui.prepare_frame();
    let node = the_paragraph(&ui, &handle);
    let read = ui.semantics.borrow().get(node).unwrap().label.clone();
    assert_eq!(read, "Ask Wing now");
    ui.remove(handle.node());
    assert!(!ui.scene.borrow().contains(node));
    ui.prepare_frame();
}

/// A box a line clamp cuts off shows nowhere and takes no hit; those before
/// the cut are where they were.
#[test]
fn a_box_past_a_line_clamp_shows_nowhere() {
    let mut ui = Ui::new(400., 200.);
    let clicks = Rc::new(Cell::new(0));
    let clicked = clicks.clone();
    let handle = ui.mount(
        paragraph()
            .w(60.)
            .line_clamp(1)
            .inline(chip("kept", 20., 6.))
            .child(text_span("aaaaaaaaaaaaaaaaaaaaaaaa"))
            .inline(chip("cut", 20., 6.).on_click(move || clicked.set(clicked.get() + 1)))
            .into(),
    );
    ui.prepare_frame();
    let at = bounds(&ui, &handle, "text");
    let kept = bounds(&ui, &handle, "kept");
    assert!(kept.x >= at.x && kept.x < at.x + 60., "{kept:?}");
    let cut = bounds(&ui, &handle, "cut");
    assert!(cut.x + cut.width < 0., "off the window: {cut:?}");
    let shown = ui
        .scene
        .borrow()
        .visible_bounds(handle.find("cut").unwrap());
    assert!(shown.is_none(), "{shown:?}");
    click(&mut ui, at.x + 30., at.y + 4.);
    assert_eq!(clicks.get(), 0);
}

/// A paragraph without boxes has no children and no boxes in its text.
#[test]
fn a_paragraph_without_boxes_is_as_it_was() {
    let mut ui = Ui::new(400., 200.);
    let handle = ui.mount(paragraph().child(text_span("plain")).into());
    ui.prepare_frame();
    let node = the_paragraph(&ui, &handle);
    assert!(ui.scene.borrow().children(node).is_empty());
    assert!(!laid_text(&ui, node).has_inline_boxes());
}
