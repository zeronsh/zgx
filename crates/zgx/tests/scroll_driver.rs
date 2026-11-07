//! A platform's own scroller driving a scroll view (iOS's `UIScrollView`,
//! headless here): it finds what is under a finger, its positions move the
//! view, and past either end the offset an app reads stays in range while the
//! rest shows as stretch.
use std::{cell::Cell, rc::Rc};

use zgx::{compose::prelude::*, widgets::Ui};

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
fn a_driver_moves_the_view_and_keeps_its_offset_in_range() {
    let ui = window(1000.);
    let driver = ui.scroll_driver_at(50., 50.).expect("a view with room");
    let metrics = driver.metrics().unwrap();
    assert_eq!(metrics.size.1, 100.);
    assert_eq!(metrics.extent.1, 1000.);
    assert!(!metrics.horizontal);

    driver.drive(200.);
    assert_eq!(driver.metrics().unwrap().offset, 200.);
    assert_eq!(driver.metrics().unwrap().stretch, 0.);

    driver.drive(-30.);
    assert_eq!(driver.metrics().unwrap().offset, 0.);
    assert_eq!(driver.metrics().unwrap().stretch, -30.);

    driver.drive(950.);
    assert_eq!(driver.metrics().unwrap().offset, 900.);
    assert_eq!(driver.metrics().unwrap().stretch, 50.);
}

#[test]
fn content_that_fits_gives_no_driver() {
    let ui = window(50.);
    assert!(ui.scroll_driver_at(50., 50.).is_none());
    assert!(ui.main_scroll_driver()().is_none());
}

#[test]
fn a_tap_on_the_status_bar_finds_the_main_view() {
    let ui = window(1000.);
    let top = ui.main_scroll_driver()().expect("a view with room");
    assert!(top.same(&ui.scroll_driver_at(50., 50.).unwrap()));
}
