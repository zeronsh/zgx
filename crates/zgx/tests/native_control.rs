//! The system's own controls where a host has them, drawn where not.
use std::{cell::Cell, rc::Rc};
use zgx::compose::prelude::*;
use zgx::native_control::{
    ButtonStyle, MenuAction, SliderState, TabsMinimize, button_stand_in, escape, native_tabs,
    native_tabs_minimizing, native_toggle, on_escape, page_control, page_control_width,
    pull_refresh_threshold, segmented_stand_in, slider_stand_in, stepper_stand_in,
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
fn a_minimizing_tab_bar_is_a_plain_one_where_nothing_minimizes() {
    // Only iOS's system bar minimizes: elsewhere its row of buttons takes a click.
    let mut ui = Ui::new(400., 83.);
    let chosen = ui.signal(0_usize);
    let (read, write) = (chosen.clone(), chosen.clone());
    let items = ["Settings", "Search"]
        .map(|title| (title.to_string(), "circle".to_string()))
        .to_vec();
    let view = ui.mount(
        div()
            .id("holder")
            .w(400.)
            .h(83.)
            .child(native_tabs_minimizing(
                items,
                move || read.get(),
                move |index| {
                    write.set(index);
                },
                TabsMinimize::OnScrollDown,
            )),
    );
    ui.prepare_frame();
    let holder = view.find("holder").unwrap();
    let tabs = {
        let scene = ui.scene.borrow();
        let bar = scene.children(holder)[0];
        scene.children(bar).to_vec()
    };
    assert_eq!(
        ui.input.pointer_target(&ui.scene.borrow(), 300., 40.),
        Some(tabs[1])
    );
    ui.input
        .dispatch_to(&ui.scene, tabs[1], zgx::input::InputEvent::Activate);
    ui.prepare_frame();
    assert_eq!(chosen.get(), 1);
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
fn a_page_control_is_one_element_saying_which_page_and_as_wide_as_uikits() {
    // Read in UIKit: "page 1 of 5", adjustable; 37.67 points for a page, 108.33
    // for five, and its intrinsic size stops growing at eight (161.33).
    assert!((page_control_width(1) - 37.67).abs() < 0.01);
    assert!((page_control_width(5) - 108.33).abs() < 0.01);
    assert!((page_control_width(8) - 161.33).abs() < 0.01);
    assert_eq!(page_control_width(30), page_control_width(8));
    let mut ui = Ui::new(300., 100.);
    let page = ui.signal(0_usize);
    let view = ui.mount(
        div()
            .id("holder")
            .child(page_control("Photos", 5, page.clone())),
    );
    ui.set_assistive_technology(true);
    ui.prepare_frame();
    let holder = view.find("holder").unwrap();
    let node = ui.scene.borrow().children(holder)[0];
    {
        let semantics = ui.semantics.borrow();
        let control = semantics.get(node).expect("an element");
        assert_eq!(control.role, Role::Slider);
        assert_eq!(control.label, "Photos");
        assert_eq!(control.value.as_deref(), Some("page 1 of 5"));
    }
    page.set(3);
    ui.prepare_frame();
    assert_eq!(
        ui.semantics.borrow().get(node).unwrap().value.as_deref(),
        Some("page 4 of 5")
    );
}

#[test]
#[cfg(not(target_os = "ios"))]
fn a_drawn_page_control_goes_to_the_dot_chosen() {
    let mut ui = Ui::new(300., 100.);
    let page = ui.signal(0_usize);
    let view = ui.mount(
        div()
            .id("holder")
            .child(page_control("Photos", 3, page.clone())),
    );
    ui.prepare_frame();
    let holder = view.find("holder").unwrap();
    let dots = {
        let scene = ui.scene.borrow();
        let bar = scene.children(holder)[0];
        scene.children(bar).to_vec()
    };
    assert_eq!(dots.len(), 3);
    ui.input
        .dispatch_to(&ui.scene, dots[2], zgx::input::InputEvent::Activate);
    ui.prepare_frame();
    assert_eq!(page.get(), 2);
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

/// A system that gives every button one size, as UIKit's does for a short title.
struct Sized_;
impl zgx::native_control::SystemUi for Sized_ {
    fn haptic(&self, _: &str) {}
    fn log_gesture(&self, _: &str) {}
    fn share(&self, _: Option<&str>, _: Option<&str>, _: &[String]) {}
    fn alert(
        &self,
        _: zgx::native_control::AlertStyle,
        _: &str,
        _: &str,
        _: &[(String, zgx::native_control::AlertButtonStyle)],
        _: Box<dyn FnOnce(usize)>,
    ) {
    }
    fn button_size(&self, title: &str, _: Option<&str>, _: ButtonStyle) -> Option<(f32, f32)> {
        Some((24. + 10. * title.len() as f32, 34.))
    }
    fn symbol_name(&self, symbol: &str) -> Option<String> {
        (symbol == "square.and.arrow.up").then(|| "Share".to_string())
    }
}

#[test]
fn a_hosted_button_stands_on_a_button_named_and_sized_for_what_it_says() {
    zgx::native_control::set_system_ui(Rc::new(Sized_));
    let mut ui = Ui::new(300., 100.);
    let title = ui.signal("Save".to_string());
    let on = ui.signal(true);
    let pressed = Rc::new(Cell::new(0));
    let (read, enabled, count) = (title.clone(), on.clone(), pressed.clone());
    let (view, state) = button_stand_in(
        move || read.get(),
        Some("star".into()),
        ButtonStyle::Filled,
        move || enabled.get(),
        (false, false),
        None,
    );
    let view = ui.mount(
        div()
            .id("holder")
            .child(view.on_click(move || count.set(count.get() + 1))),
    );
    ui.set_assistive_technology(true);
    ui.prepare_frame();
    let holder = view.find("holder").unwrap();
    let node = ui.scene.borrow().children(holder)[0];
    let read = |ui: &Ui| {
        let semantics = ui.semantics.borrow();
        let n = semantics.get(node).unwrap();
        (n.role, n.label.clone())
    };
    assert_eq!(read(&ui), (Role::Button, "Save".to_string()));
    let size = |ui: &Ui| {
        let bounds = ui.scene.borrow().bounds(node);
        (bounds.width, bounds.height)
    };
    assert_eq!(size(&ui), (64., 34.), "the system's size for it");
    assert_eq!(state.style, ButtonStyle::Filled);
    assert_eq!(state.symbol.as_deref(), Some("star"));

    // What it says changes: its name, what the host shows, and its size.
    title.set("Continue".to_string());
    ui.prepare_frame();
    assert_eq!(read(&ui).1, "Continue");
    assert_eq!(*state.title.borrow(), "Continue");
    assert_eq!(size(&ui), (104., 34.));

    // UIKit's press, as the host passes it on, is its click.
    let press = state.pressed.borrow().clone().expect("mounted");
    press();
    assert_eq!(pressed.get(), 1);
    // As assistive technology's.
    ui.input
        .dispatch_to(&ui.scene, node, zgx::input::InputEvent::Activate);
    assert_eq!(pressed.get(), 2);

    assert!(state.enabled.get());
    on.set(false);
    ui.prepare_frame();
    assert!(!state.enabled.get(), "the host's enabled follows");
}

#[test]
fn a_hosted_button_of_a_symbol_alone_is_named_as_the_system_names_it() {
    zgx::native_control::set_system_ui(Rc::new(Sized_));
    let name = |symbol: &str| {
        let mut ui = Ui::new(300., 100.);
        let (view, _) = button_stand_in(
            String::new,
            Some(symbol.to_string()),
            ButtonStyle::Plain,
            || true,
            (false, false),
            None,
        );
        let view = ui.mount(div().id("holder").child(view));
        ui.set_assistive_technology(true);
        ui.prepare_frame();
        let holder = view.find("holder").unwrap();
        let node = ui.scene.borrow().children(holder)[0];
        ui.semantics.borrow().get(node).unwrap().label.clone()
    };
    assert_eq!(name("square.and.arrow.up"), "Share");
    assert_eq!(
        name("gearshape"),
        "gearshape",
        "its name, where the system has none"
    );
}

#[test]
fn a_hosted_buttons_menu_follows_its_open_state() {
    let mut ui = Ui::new(300., 100.);
    let open = ui.signal(false);
    let build: zgx::native_control::MenuBuilder = Rc::new(Vec::new);
    let (view, state) = button_stand_in(
        || "Sort".to_string(),
        None,
        ButtonStyle::Gray,
        || true,
        (false, false),
        Some((build, open.clone())),
    );
    let _view = ui.mount(div().child(view));
    ui.prepare_frame();
    assert!(!state.open.get());
    open.set(true);
    ui.prepare_frame();
    assert!(state.open.get());
    // The host is told at once, with no frame: the scene doesn't change with this.
    let told = Rc::new(Cell::new(None));
    let heard = told.clone();
    *state.present.borrow_mut() = Some(Rc::new(move |open| heard.set(Some(open))));
    open.set(false);
    ui.runtime.flush();
    assert_eq!(told.get(), Some(false));
    open.set(true);
    ui.runtime.flush();
    assert_eq!(told.get(), Some(true));
}

#[test]
fn a_hosted_button_given_a_size_keeps_it() {
    zgx::native_control::set_system_ui(Rc::new(Sized_));
    let mut ui = Ui::new(300., 100.);
    let (view, _) = button_stand_in(
        || "Save".to_string(),
        None,
        ButtonStyle::Gray,
        || true,
        (true, false),
        None,
    );
    let view = ui.mount(div().id("holder").child(view.w(200.)));
    ui.prepare_frame();
    let holder = view.find("holder").unwrap();
    let node = ui.scene.borrow().children(holder)[0];
    let bounds = ui.scene.borrow().bounds(node);
    assert_eq!((bounds.width, bounds.height), (200., 34.));
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
                checked: false,
                disabled: false,
                keep_open: false,
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

#[test]
fn a_tab_bar_tells_its_height_while_it_is_mounted() {
    let mut ui = Ui::new(400., 300.);
    let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mounted = ui.signal(true);
    let (read, told) = (mounted.clone(), heard.clone());
    ui.mount(div().child(switch(
        move || read.get(),
        move |mounted, _| {
            if !mounted {
                return div();
            }
            let told = told.clone();
            div().w(400.).h(83.).child(zgx::compose::provide(
                zgx::native_control::TabBarHeight::new(move |height| {
                    told.borrow_mut().push(height);
                }),
                native_tabs(
                    vec![("One".to_string(), "circle".to_string())],
                    || 0,
                    |_| {},
                ),
            ))
        },
    )));
    ui.prepare_frame();
    ui.prepare_frame();
    assert_eq!(*heard.borrow(), [83.], "its height, once");
    ui.prepare_frame();
    assert_eq!(*heard.borrow(), [83.], "nothing while nothing changes");
    mounted.set(false);
    ui.prepare_frame();
    assert_eq!(*heard.borrow(), [83., 0.], "and that it's gone");
    // Told again by a new one, though it's the height the last one had.
    mounted.set(true);
    ui.prepare_frame();
    ui.prepare_frame();
    assert_eq!(*heard.borrow(), [83., 0., 83.]);
}
