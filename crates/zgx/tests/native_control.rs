//! The system's own controls where a host has them, drawn where not.
use std::{cell::Cell, rc::Rc};
use zgx::compose::prelude::*;
use zgx::native_control::{MenuAction, escape, native_tabs, native_toggle, on_escape};
use zgx::semantics::Role;
use zgx::widgets::Ui;

#[test]
fn a_drawn_switch_is_one_switch_that_toggles() {
    let mut ui = Ui::new(300., 100.);
    let on = ui.signal(false);
    let (read, write) = (on.clone(), on.clone());
    let view = ui.mount(div().id("holder").child(native_toggle(
        "Airplane Mode",
        move || read.get(),
        move |next| {
            write.set(next);
        },
    )));
    ui.set_assistive_technology(true);
    ui.prepare_frame();
    let holder = view.find("holder").unwrap();
    let node = ui.scene.borrow().children(holder)[0];
    {
        let semantics = ui.semantics.borrow();
        let switch = semantics.get(node).expect("a switch");
        assert_eq!(
            (switch.role, switch.label.as_str(), switch.checked),
            (Role::Switch, "Airplane Mode", Some(false))
        );
    }
    ui.input
        .dispatch_to(&ui.scene, node, zgx::input::InputEvent::Activate);
    ui.prepare_frame();
    assert!(on.get(), "activating it turns it on");
    assert_eq!(ui.semantics.borrow().get(node).unwrap().checked, Some(true));
}

#[test]
fn drawn_tabs_are_a_tab_list_whose_tabs_choose() {
    let mut ui = Ui::new(400., 100.);
    let chosen = ui.signal(0usize);
    let (read, write) = (chosen.clone(), chosen.clone());
    let items = ["Settings", "Search", "Profile"]
        .map(|title| (title.to_string(), "circle".to_string()))
        .to_vec();
    let view = ui.mount(div().id("holder").w(400.).h(83.).child(native_tabs(
        items,
        move || read.get(),
        move |index| {
            write.set(index);
        },
    )));
    ui.set_assistive_technology(true);
    ui.prepare_frame();
    let holder = view.find("holder").unwrap();
    let (bar, tabs) = {
        let scene = ui.scene.borrow();
        let bar = scene.children(holder)[0];
        (bar, scene.children(bar).to_vec())
    };
    assert_eq!(ui.semantics.borrow().get(bar).unwrap().role, Role::TabList);
    assert_eq!(tabs.len(), 3);
    let selected = |ui: &Ui| -> Vec<Option<bool>> {
        let semantics = ui.semantics.borrow();
        tabs.iter()
            .map(|t| semantics.get(*t).unwrap().selected)
            .collect()
    };
    assert_eq!(selected(&ui), [Some(true), Some(false), Some(false)]);
    ui.input
        .dispatch_to(&ui.scene, tabs[2], zgx::input::InputEvent::Activate);
    ui.prepare_frame();
    assert_eq!(chosen.get(), 2);
    assert_eq!(selected(&ui), [Some(false), Some(false), Some(true)]);
}

#[test]
fn going_back_asks_the_latest_handler_first() {
    let order = Rc::new(Cell::new(0));
    let (first, second) = (order.clone(), order.clone());
    on_escape(Rc::new(move || {
        first.set(first.get() * 10 + 1);
        true
    }));
    on_escape(Rc::new(move || {
        second.set(second.get() * 10 + 2);
        false
    }));
    assert!(escape(), "one went back");
    assert_eq!(
        order.get(),
        21,
        "the latest asked first, then the one before"
    );
}

#[test]
fn a_system_menu_is_found_from_a_point_inside_its_node() {
    let mut ui = Ui::new(300., 200.);
    let view = ui.mount(
        div()
            .id("row")
            .w(200.)
            .h(60.)
            .child(div().id("inner").w(40.).h(40.)),
    );
    ui.prepare_frame();
    let row = view.find("row").unwrap();
    zgx::native_control::set_system_menu(
        row,
        Rc::new(|| {
            vec![MenuAction {
                title: "Copy".into(),
                symbol: Some("doc.on.doc".into()),
                destructive: false,
                run: Rc::new(|| {}),
            }]
        }),
    );
    let found = zgx::native_control::system_menu_at(&ui.scene.borrow(), 20., 20.);
    let (rect, actions) = found.expect("the row's menu, from inside its child");
    assert_eq!((rect.width, rect.height), (200., 60.));
    assert_eq!(actions[0].title, "Copy");
    assert!(zgx::native_control::system_menu_at(&ui.scene.borrow(), 250., 150.).is_none());
}
