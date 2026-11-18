//! A node is disabled when it or any ancestor is (a fieldset's controls):
//! no input or focus reaches it, its own disabled style shows, and
//! assistive technology hears it disabled. Re-enabling the ancestor gives
//! it all back, without building it again.
use std::{cell::Cell, rc::Rc};
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    scene::NodeId,
    widgets::Ui,
};

fn opacity(ui: &Ui, node: NodeId) -> f32 {
    ui.scene.borrow().effects(node).opacity
}
fn hears_disabled(ui: &Ui, node: NodeId) -> bool {
    ui.semantics.borrow().get(node).is_some_and(|s| s.disabled)
}
fn click(ui: &mut Ui, node: NodeId) {
    let at = ui.scene.borrow().bounds(node);
    let (x, y) = (at.x + at.width / 2., at.y + at.height / 2.);
    for down in [true, false] {
        ui.dispatch(if down {
            InputEvent::PointerDown {
                x,
                y,
                button: PointerButton::Primary,
            }
        } else {
            InputEvent::PointerUp {
                x,
                y,
                button: PointerButton::Primary,
            }
        });
    }
    ui.prepare_frame();
}
fn tab(ui: &mut Ui) {
    ui.dispatch(InputEvent::KeyDown {
        key: Key::Tab,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.prepare_frame();
}

/// A form: a disabled group holding a button, a box with a disabled style
/// of its own and a plain box, then a button outside it.
fn form(off: &zgx::reactive::Signal<bool>, clicks: &Rc<Cell<u32>>) -> View {
    let (read, count) = (off.clone(), clicks.clone());
    column()
        .gap(4.)
        .child(
            column()
                .id("group")
                .disabled_when(move || read.get())
                .child(
                    button()
                        .id("inside")
                        .size(80., 30.)
                        .on_click(move || count.set(count.get() + 1)),
                )
                .child(
                    div()
                        .id("styled")
                        .size(20., 20.)
                        .disabled_style(|s| s.opacity(0.6)),
                )
                .child(div().id("plain").size(20., 20.)),
        )
        .child(button().id("outside").size(80., 30.))
}

#[test]
fn a_control_under_a_disabled_ancestor_takes_no_input_or_focus() {
    let mut ui = Ui::new(300., 300.);
    let off = ui.signal(true);
    let clicks = Rc::new(Cell::new(0));
    let view = form(&off, &clicks);
    let view = ui.mount(view);
    ui.prepare_frame();
    let (inside, outside) = (view.find("inside").unwrap(), view.find("outside").unwrap());
    click(&mut ui, inside);
    assert_eq!(clicks.get(), 0, "no click reaches it");
    tab(&mut ui);
    assert_eq!(ui.input.focused(), Some(outside), "Tab passes it by");
    ui.input.focus(&ui.scene, Some(inside));
    assert_ne!(ui.input.focused(), Some(inside), "nor can it be focused");
    off.set(false);
    ui.prepare_frame();
    assert_eq!(view.find("inside"), Some(inside), "the same node");
    click(&mut ui, inside);
    assert_eq!(clicks.get(), 1, "enabled again, it clicks");
    ui.input.focus(&ui.scene, Some(inside));
    assert_eq!(ui.input.focused(), Some(inside), "and takes focus");
}

#[test]
fn a_descendant_shows_its_own_disabled_style() {
    let mut ui = Ui::new(300., 300.);
    let off = ui.signal(true);
    let clicks = Rc::new(Cell::new(0));
    let view = form(&off, &clicks);
    let view = ui.mount(view);
    ui.prepare_frame();
    let [inside, styled, plain, outside] =
        ["inside", "styled", "plain", "outside"].map(|id| view.find(id).unwrap());
    assert!((opacity(&ui, inside) - 0.45).abs() < 1e-6, "a control dims");
    assert!(
        (opacity(&ui, styled) - 0.6).abs() < 1e-6,
        "its own disabled style"
    );
    assert_eq!(opacity(&ui, plain), 1., "a box with none stays");
    assert_eq!(opacity(&ui, outside), 1., "outside the group, enabled");
    off.set(false);
    ui.prepare_frame();
    for (node, name) in [(inside, "inside"), (styled, "styled"), (plain, "plain")] {
        assert_eq!(opacity(&ui, node), 1., "{name}: enabled again");
    }
    off.set(true);
    ui.prepare_frame();
    assert!((opacity(&ui, styled) - 0.6).abs() < 1e-6, "disabled again");
}

#[test]
fn assistive_technology_hears_a_descendant_disabled() {
    let mut ui = Ui::new(300., 300.);
    let off = ui.signal(true);
    let clicks = Rc::new(Cell::new(0));
    let view = form(&off, &clicks);
    let view = ui.mount(view);
    ui.prepare_frame();
    let (inside, outside) = (view.find("inside").unwrap(), view.find("outside").unwrap());
    assert!(hears_disabled(&ui, inside), "inside a disabled group");
    assert!(!hears_disabled(&ui, outside));
    off.set(false);
    ui.prepare_frame();
    assert!(!hears_disabled(&ui, inside), "enabled again");
}

#[test]
fn nested_groups_disable_what_either_holds() {
    let mut ui = Ui::new(300., 300.);
    let (outer, inner) = (ui.signal(false), ui.signal(false));
    let (o, i) = (outer.clone(), inner.clone());
    let view = ui.mount(
        column().disabled_when(move || o.get()).child(
            column()
                .disabled_when(move || i.get())
                .child(button().id("b").size(40., 20.)),
        ),
    );
    ui.prepare_frame();
    let b = view.find("b").unwrap();
    assert_eq!(opacity(&ui, b), 1.);
    for (o, i, dimmed) in [
        (true, false, true),
        (false, true, true),
        (true, true, true),
        (false, false, false),
    ] {
        outer.set(o);
        inner.set(i);
        ui.prepare_frame();
        let expected = if dimmed { 0.45 } else { 1. };
        assert!(
            (opacity(&ui, b) - expected).abs() < 1e-6,
            "outer {o} inner {i}"
        );
        assert_eq!(hears_disabled(&ui, b), dimmed, "outer {o} inner {i}");
    }
}
