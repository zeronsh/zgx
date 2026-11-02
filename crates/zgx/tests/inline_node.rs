//! A node in a paragraph's text (`TextSpan::node`), in the headless core's
//! approximate layout: each grapheme a cell 0.6 of the text size wide (6 at
//! size 10), a line 20 tall here. The headless layout knows no baseline, so
//! a node is centred on its line.
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Modifiers, PointerButton},
    scene::{Color, NodeId, Rect},
    widgets::Ui,
};

fn formula(id: &str) -> InlineNode {
    InlineNode::new(column().id(id).w(30.).h(40.).bg(Color(80, 120, 200, 255)))
}
fn document(ui: &mut Ui, paragraph: View) -> zgx::compose::ViewHandle {
    let view = ui.mount(
        column()
            .w(300.)
            .gap(4.)
            .text_size(10.)
            .line_height(20.)
            .selectable(true)
            .children(vec![paragraph, text("Next").id("next")]),
    );
    ui.prepare_frame();
    view
}
fn drag(ui: &mut Ui, from: (f32, f32), to: (f32, f32)) {
    let button = PointerButton::Primary;
    let (x, y) = from;
    ui.dispatch_with_modifiers(
        InputEvent::PointerDown { x, y, button },
        Modifiers::default(),
    );
    let (x, y) = to;
    ui.dispatch_with_modifiers(InputEvent::PointerMove { x, y }, Modifiers::default());
    ui.dispatch_with_modifiers(InputEvent::PointerUp { x, y, button }, Modifiers::default());
}
/// Every node under `id`, depth first.
fn under(ui: &Ui, id: NodeId) -> Vec<NodeId> {
    let scene = ui.scene.borrow();
    let mut out = Vec::new();
    let mut pending = vec![id];
    while let Some(id) = pending.pop() {
        out.push(id);
        pending.extend(scene.children(id).iter().rev());
    }
    out
}

#[test]
fn a_node_keeps_a_words_room_in_the_line_and_the_line_grows_to_hold_it() {
    let mut ui = Ui::new(400., 300.);
    let node = formula("formula");
    let view = document(
        &mut ui,
        rich_text()
            .child(text_span("where "))
            .child(text_span("$x$").node(&node, 30., 40., 30.))
            .child(text_span(" holds"))
            .into(),
    );
    let bounds = |id: &str| ui.scene.borrow().bounds(view.find(id).unwrap());
    // After "where " (six cells), as wide and tall as it was given, on a
    // line grown to hold it (centred: the headless layout has no baseline).
    assert_eq!(bounds("formula"), Rect::new(36., 0., 30., 40.));
    assert_eq!(bounds("next").y, 44.);
    // A selection takes it as one character: half way into it, a drag
    // reaches past it; short of half way, it stops before it.
    drag(&mut ui, (8., 20.), (56., 20.));
    assert_eq!(ui.selected_text(), "here $x$");
    drag(&mut ui, (8., 20.), (44., 20.));
    assert_eq!(ui.selected_text(), "here ");
    // Copied, it is the text it stands for, as rich text too.
    drag(&mut ui, (0., 20.), (110., 20.));
    assert_eq!(ui.selected_text(), "where $x$ holds");
    assert!(
        ui.selected_html().contains("where $x$ holds"),
        "{}",
        ui.selected_html()
    );
    // Read out, so is it.
    let semantics = ui.semantics.borrow();
    let read: Vec<String> = under(&ui, ui.root())
        .into_iter()
        .filter_map(|id| semantics.get(id).map(|node| node.label.clone()))
        .collect();
    assert!(
        read.iter().any(|label| label == "where $x$ holds"),
        "{read:?}"
    );
}

#[test]
fn a_node_follows_its_size_and_stays_mounted_while_no_span_holds_it() {
    let mut ui = Ui::new(400., 300.);
    let node = formula("formula");
    let width = ui.runtime.signal(30_f32);
    let shown = ui.runtime.signal(true);
    let view = document(&mut ui, {
        let (width, shown, node) = (width.clone(), shown.clone(), node.clone());
        rich_text_signal(move || {
            let mut spans = vec![text_span("a ")];
            if shown.get() {
                spans.push(text_span("$x$").node(&node, width.get(), 40., 30.));
            }
            spans.push(text_span(" b"));
            spans
        })
    });
    let formula = view.find("formula").unwrap();
    // The room it is placed in, as wide as the span says.
    let room = ui.scene.borrow().parent(formula).unwrap();
    let bounds = |ui: &Ui, id: NodeId| ui.scene.borrow().bounds(id);
    assert_eq!(bounds(&ui, room), Rect::new(12., 0., 30., 40.));
    width.set(50.);
    ui.prepare_frame();
    assert_eq!(bounds(&ui, room), Rect::new(12., 0., 50., 40.));
    // No span holds it: hidden, the line its text's again, the node kept.
    shown.set(false);
    ui.prepare_frame();
    assert!(!ui.scene.borrow().layout_visible(formula));
    assert_eq!(bounds(&ui, view.find("next").unwrap()).y, 24.);
    shown.set(true);
    ui.prepare_frame();
    assert_eq!(view.find("formula"), Some(formula), "the same node, kept");
    assert!(ui.scene.borrow().layout_visible(formula));
    assert_eq!(bounds(&ui, room), Rect::new(12., 0., 50., 40.));
}
