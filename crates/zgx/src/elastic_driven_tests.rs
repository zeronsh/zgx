//! What iOS's scroller adds to a scroll view (`Driven`): room past its end and
//! a drawn position held while the system moves it. Compiled for iOS and for
//! these tests only, so no desktop build carries it.
use crate::{compose::prelude::*, widgets::Ui};
use std::{cell::Cell, rc::Rc};

fn window(content_height: f32) -> Ui {
    let mut ui = Ui::new(100., 100.);
    let offset = ui.signal(0.);
    ui.mount(
        scroll(offset)
            .w(100.)
            .h(100.)
            .child(div().w(100.).h(content_height)),
    );
    ui.prepare_frame();
    ui
}

#[test]
fn room_past_a_views_end_lets_it_scroll_there_and_stay_through_a_layout() {
    let ui = window(150.);
    let driver = ui.scroll_driver_at(50., 50.).expect("a view with room");
    driver.drive(100.);
    ui.prepare_frame();
    assert_eq!(driver.metrics().unwrap().offset, 50., "its content's end");
    driver.set_end_room(200.);
    driver.drive(180.);
    ui.prepare_frame();
    assert_eq!(driver.metrics().unwrap().offset, 180.);
    assert_eq!(driver.metrics().unwrap().stretch, 0.);
    driver.set_end_room(0.);
    driver.drive(50.);
    ui.prepare_frame();
    assert_eq!(driver.metrics().unwrap().offset, 50.);
}

#[test]
fn room_past_a_lists_end_lets_it_scroll_there_too() {
    let mut ui = Ui::new(100., 100.);
    let offset = ui.signal(0.);
    ui.mount(
        virtual_list(
            offset.clone(),
            30.,
            1,
            || 5,
            |index| index,
            |_, _, _| div().h(30.),
        )
        .w(100.)
        .h(100.),
    );
    ui.prepare_frame();
    let driver = ui.scroll_driver_at(50., 50.).expect("a list with room");
    driver.set_end_room(200.);
    driver.drive(170.);
    ui.prepare_frame();
    assert_eq!(driver.metrics().unwrap().offset, 170.);
}

#[test]
fn a_held_view_stays_drawn_where_it_is_as_its_offset_goes_on() {
    let ui = window(1000.);
    let driver = ui.scroll_driver_at(50., 50.).expect("a view with room");
    driver.drive(100.);
    ui.prepare_frame();
    let content = |ui: &Ui| {
        let scene = ui.scene.borrow();
        let root = scene.root();
        let mut node = root;
        let mut y = 0.;
        // The one transformed node of this little tree.
        while let Some(child) = scene.children(node).first().copied() {
            y += scene.transform(child).y;
            node = child;
        }
        y
    };
    assert_eq!(content(&ui), -100.);

    assert!(driver.hold(Some(100.)));
    driver.drive(300.);
    ui.prepare_frame();
    assert_eq!(driver.metrics().unwrap().offset, 300., "readers follow");
    assert_eq!(content(&ui), -100., "nothing of it moves");

    driver.hold(None);
    ui.prepare_frame();
    assert_eq!(content(&ui), -300.);
}

#[test]
fn a_held_list_stays_drawn_where_it_is_as_its_offset_and_rows_go_on() {
    let mut ui = Ui::new(100., 100.);
    let offset = ui.signal(0.);
    let built = Rc::new(Cell::new(0));
    let counted = built.clone();
    ui.mount(
        virtual_list(
            offset.clone(),
            30.,
            0,
            || 100,
            |index| index,
            move |_, index, _| {
                counted.set(counted.get().max(index));
                div().h(30.)
            },
        )
        .w(100.)
        .h(100.),
    );
    ui.prepare_frame();
    let driver = ui.scroll_driver_at(50., 50.).expect("a list with room");
    driver.drive(100.);
    ui.prepare_frame();
    // The list, its viewport, its content: not as far as a row's own place.
    let content = |ui: &Ui| {
        let scene = ui.scene.borrow();
        let mut node = scene.root();
        let mut y = 0.;
        for _ in 0..3 {
            let Some(child) = scene.children(node).first().copied() else {
                break;
            };
            y += scene.transform(child).y;
            node = child;
        }
        y
    };
    assert_eq!(content(&ui), -100.);

    assert!(driver.hold(Some(100.)));
    driver.drive(600.);
    ui.prepare_frame();
    assert_eq!(driver.metrics().unwrap().offset, 600., "readers follow");
    assert_eq!(offset.get(), 600.);
    assert_eq!(content(&ui), -100., "nothing of it moves");
    assert!(built.get() >= 20, "the rows of its offset are there");

    driver.hold(None);
    ui.prepare_frame();
    assert_eq!(content(&ui), -600.);
}

#[test]
fn a_views_keyboard_ride_reaches_its_driver() {
    use crate::elastic::KeyboardRide;
    for (ride, want) in [
        (None, KeyboardRide::Native),
        (Some(KeyboardRide::Attached), KeyboardRide::Attached),
    ] {
        let mut ui = Ui::new(100., 100.);
        let offset = ui.signal(0.);
        let view = scroll(offset).w(100.).h(100.).child(div().w(100.).h(400.));
        ui.mount(match ride {
            Some(ride) => view.keyboard_ride(ride),
            None => view,
        });
        ui.prepare_frame();
        let driver = ui.scroll_driver_at(50., 50.).expect("the scroll view");
        assert_eq!(driver.keyboard_ride(), want);
    }
}
