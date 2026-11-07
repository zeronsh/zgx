//! The system's own controls where a host has them, drawn where not.
use std::{cell::Cell, rc::Rc};
use zgx::compose::prelude::*;
use zgx::native_control::{
    MenuAction, SliderState, escape, native_tabs, native_toggle, on_escape, pull_refresh_threshold,
    segmented_stand_in, slider_stand_in, stepper_stand_in,
};
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
fn a_hosted_slider_stands_on_one_slider_moved_a_step_at_a_time() {
    let mut ui = Ui::new(300., 100.);
    let level = ui.signal(20_f32);
    let view = ui.mount(div().id("holder").child(slider_stand_in(
        "Volume",
        level.clone(),
        0.0..=100.,
        Some(5.),
    )));
    ui.set_assistive_technology(true);
    ui.prepare_frame();
    let holder = view.find("holder").unwrap();
    let node = ui.scene.borrow().children(holder)[0];
    {
        let semantics = ui.semantics.borrow();
        let slider = semantics.get(node).expect("a slider");
        assert_eq!(
            (slider.role, slider.label.as_str()),
            (Role::Slider, "Volume")
        );
        assert_eq!(
            (slider.min, slider.max, slider.numeric_value),
            (Some(0.), Some(100.), Some(20.))
        );
        // UISlider's reading (read in UIKit: 0.123 of the range is "12%").
        assert_eq!(slider.value.as_deref(), Some("20%"));
    }
    assert_eq!(ui.scene.borrow().numeric_step(node), Some(5.));
    ui.input
        .dispatch_to(&ui.scene, node, zgx::input::InputEvent::Increment);
    ui.prepare_frame();
    assert_eq!(level.get(), 25., "assistive technology moves it a step");
    level.set(70.);
    ui.prepare_frame();
    assert_eq!(
        ui.semantics.borrow().get(node).unwrap().numeric_value,
        Some(70.),
        "it follows the value"
    );
    assert_eq!(
        ui.semantics.borrow().get(node).unwrap().value.as_deref(),
        Some("70%")
    );
    assert_eq!(
        ui.scene.borrow().bounds(node).height,
        31.,
        "the system's height"
    );
}

#[test]
fn a_slider_drag_lands_in_its_range_and_on_its_step() {
    let state = |step| SliderState {
        value: Cell::new(0.),
        min: 10.,
        max: 20.,
        step,
        changed: Box::new(|_| {}),
    };
    let stepped = state(Some(2.5));
    assert_eq!(stepped.landed(13.6), 12.5);
    assert_eq!(stepped.landed(13.8), 15.);
    assert_eq!(stepped.landed(99.), 20.);
    assert_eq!(stepped.landed(f32::NAN), 10.);
    assert_eq!(state(None).landed(13.6), 13.6);
    assert_eq!(state(None).landed(-4.), 10.);
}

#[test]
fn a_hosted_segmented_control_stands_on_a_button_per_segment() {
    // As UIKit's reads (read with accessibility on): each segment a button
    // named by its title, the chosen one selected.
    let mut ui = Ui::new(400., 100.);
    let chosen = ui.signal(0usize);
    let items = ["One", "Two", "Three"].map(String::from).to_vec();
    let view = ui.mount(div().id("holder").child(segmented_stand_in(
        "Mode",
        items,
        chosen.clone(),
    )));
    ui.set_assistive_technology(true);
    ui.prepare_frame();
    let holder = view.find("holder").unwrap();
    let (bar, segments) = {
        let scene = ui.scene.borrow();
        let bar = scene.children(holder)[0];
        (bar, scene.children(bar).to_vec())
    };
    assert_eq!(
        ui.scene.borrow().bounds(bar).height,
        32.,
        "the system's height"
    );
    let read = |ui: &Ui| -> Vec<(Role, String, Option<bool>)> {
        let semantics = ui.semantics.borrow();
        segments
            .iter()
            .map(|s| {
                let n = semantics.get(*s).unwrap();
                (n.role, n.label.clone(), n.selected)
            })
            .collect()
    };
    assert_eq!(
        read(&ui),
        [
            (Role::Button, "One".into(), Some(true)),
            (Role::Button, "Two".into(), Some(false)),
            (Role::Button, "Three".into(), Some(false)),
        ]
    );
    ui.input
        .dispatch_to(&ui.scene, segments[2], zgx::input::InputEvent::Activate);
    ui.prepare_frame();
    assert_eq!(chosen.get(), 2);
    assert_eq!(read(&ui)[2].2, Some(true));
    chosen.set(1);
    ui.prepare_frame();
    assert_eq!(read(&ui)[1].2, Some(true), "it follows the value");
}

#[test]
fn a_hosted_stepper_stands_on_a_spin_button_held_in_its_range() {
    let mut ui = Ui::new(300., 100.);
    let count = ui.signal(9_f64);
    let view = ui.mount(div().id("holder").child(stepper_stand_in(
        "Copies",
        count.clone(),
        0.0..=10.,
        2.,
    )));
    ui.set_assistive_technology(true);
    ui.prepare_frame();
    let holder = view.find("holder").unwrap();
    let node = ui.scene.borrow().children(holder)[0];
    {
        let semantics = ui.semantics.borrow();
        let stepper = semantics.get(node).expect("a spin button");
        assert_eq!(
            (stepper.role, stepper.label.as_str(), stepper.numeric_value),
            (Role::SpinButton, "Copies", Some(9.))
        );
    }
    let bounds = ui.scene.borrow().bounds(node);
    assert_eq!(
        (bounds.width, bounds.height),
        (94., 32.),
        "the system's size"
    );
    ui.input
        .dispatch_to(&ui.scene, node, zgx::input::InputEvent::Increment);
    ui.prepare_frame();
    assert_eq!(count.get(), 10., "a step, held in its range");
    ui.input
        .dispatch_to(&ui.scene, node, zgx::input::InputEvent::Decrement);
    ui.prepare_frame();
    assert_eq!(count.get(), 8.);
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

#[test]
fn a_pull_refreshes_where_uikit_snaps() {
    // UIRefreshControl's `_snappingHeight`, read at runtime on an 874-point
    // iPhone 17 Pro: 173.107.
    assert!((pull_refresh_threshold(874.) - 173.107_394).abs() < 1e-3);
    // Under 372 points the cap stops shrinking, at 112.5 × 372 / 568.
    assert!((pull_refresh_threshold(300.) - 73.680).abs() < 1e-3);
    assert!((pull_refresh_threshold(200.) - 73.680).abs() < 1e-3);
    // A shorter one refreshes at half its height.
    assert_eq!(pull_refresh_threshold(100.), 50.);
    assert!(pull_refresh_threshold(912.) > pull_refresh_threshold(874.));
}

#[test]
fn a_swipe_begins_as_uikits_row_swipes_do() {
    use zgx::native_control::{SwipePan, swipe_pan};
    // UIKit, driven: -100/-30 swipes (25 across before 10 down), -100/-50
    // scrolls (10 down at 20 across), both ways across.
    assert_eq!(swipe_pan(-24.9, 0.), SwipePan::Waits);
    assert_eq!(swipe_pan(-25., -7.5), SwipePan::Begins);
    assert_eq!(swipe_pan(25., 7.5), SwipePan::Begins);
    assert_eq!(swipe_pan(-20.1, -10.05), SwipePan::Fails);
    assert_eq!(swipe_pan(0., 10.5), SwipePan::Fails);
}
