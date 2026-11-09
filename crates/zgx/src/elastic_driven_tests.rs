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
fn a_views_keyboard_dismiss_reaches_its_driver() {
    use crate::elastic::KeyboardDismiss;
    for (dismiss, want) in [
        (None, KeyboardDismiss::None),
        (Some(KeyboardDismiss::OnDrag), KeyboardDismiss::OnDrag),
        (
            Some(KeyboardDismiss::Interactive),
            KeyboardDismiss::Interactive,
        ),
    ] {
        let mut ui = Ui::new(100., 100.);
        let offset = ui.signal(0.);
        let view = scroll(offset).w(100.).h(100.).child(div().w(100.).h(400.));
        ui.mount(match dismiss {
            Some(dismiss) => view.keyboard_dismiss(dismiss),
            None => view,
        });
        ui.prepare_frame();
        let driver = ui.scroll_driver_at(50., 50.).expect("the scroll view");
        assert_eq!(driver.keyboard_dismiss(), want);
    }
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

#[test]
fn a_paging_view_tells_the_platforms_scroller_and_others_dont() {
    let mut ui = Ui::new(100., 100.);
    let (pages, free) = (ui.signal(0.), ui.signal(0.));
    ui.mount(
        div()
            .child(
                scroll_x(pages)
                    .paging(true)
                    .w(100.)
                    .h(50.)
                    .child(div().w(400.).h(50.)),
            )
            .child(scroll(free).w(100.).h(50.).child(div().w(100.).h(400.))),
    );
    ui.prepare_frame();
    let paging = ui.scroll_driver_at(50., 25.).expect("the pages");
    assert!(paging.metrics().unwrap().horizontal);
    assert!(paging.metrics().unwrap().paging);
    let free = ui.scroll_driver_at(50., 75.).expect("the free view");
    assert!(!free.metrics().unwrap().paging);
}

#[test]
fn a_view_saying_what_covers_it_tells_the_platforms_scroller_and_follows_it() {
    use crate::scene::Insets;
    let mut ui = Ui::new(100., 100.);
    let (covered, free) = (ui.signal(0.), ui.signal(0.));
    let below = ui.signal(34_f32);
    let read = below.clone();
    ui.mount(
        div()
            .child(
                scroll(covered)
                    .scroll_indicator_insets(move || Insets {
                        top: 116.,
                        bottom: read.get(),
                        ..Insets::default()
                    })
                    .w(100.)
                    .h(50.)
                    .child(div().w(100.).h(400.)),
            )
            .child(scroll(free).w(100.).h(50.).child(div().w(100.).h(400.))),
    );
    ui.prepare_frame();
    let driver = ui.scroll_driver_at(50., 25.).expect("the covered view");
    let insets = driver.metrics().unwrap().indicator_insets.unwrap();
    assert_eq!((insets.top, insets.bottom), (116., 34.));
    below.set(83.);
    ui.prepare_frame();
    let insets = driver.metrics().unwrap().indicator_insets.unwrap();
    assert_eq!((insets.top, insets.bottom), (116., 83.));
    // The system's automatic insets are the others'.
    let free = ui.scroll_driver_at(50., 75.).expect("the free view");
    assert_eq!(free.metrics().unwrap().indicator_insets, None);
}

#[test]
fn a_press_finds_what_acts_on_it_and_not_a_page_around_it() {
    let mut ui = Ui::new(100., 100.);
    let page = ui.signal(0.);
    ui.mount(
        scroll(page).w(100.).h(100.).child(
            div()
                .w(100.)
                .h(400.)
                .child(div().w(100.).h(40.).on_click(|| {}))
                .child(div().w(100.).h(40.)),
        ),
    );
    ui.prepare_frame();
    assert!(ui.pressable_at(50., 20.), "the row that clicks");
    assert!(!ui.pressable_at(50., 60.), "the page around it");
}

#[test]
fn rows_of_a_scroll_view_share_a_finger_and_nothing_else_does() {
    let mut ui = Ui::new(200., 200.);
    let (left, right) = (ui.signal(0.), ui.signal(0.));
    let row = || div().w(100.).h(40.).on_click(|| {}).table_row();
    ui.mount(
        crate::compose::row()
            .child(
                scroll(left)
                    .w(100.)
                    .h(200.)
                    .child(row())
                    .child(row())
                    // A control of its own, and one inside a row.
                    .child(div().w(100.).h(40.).on_click(|| {}))
                    .child(
                        div()
                            .w(100.)
                            .h(40.)
                            .on_click(|| {})
                            .table_row()
                            .child(div().w(100.).h(20.).on_click(|| {})),
                    ),
            )
            .child(scroll(right).w(100.).h(200.).child(row())),
    );
    ui.prepare_frame();
    let (one, two, button) = ((50., 20.), (50., 60.), (50., 100.));
    let (inside, around) = ((50., 130.), (50., 150.));
    let other_table = (150., 20.);
    assert!(ui.presses_share(one, two), "two rows of one table");
    assert!(ui.presses_share(one, around), "the row around a control");
    assert!(!ui.presses_share(one, button), "a row and a button");
    assert!(
        !ui.presses_share(one, inside),
        "a row and the button in one"
    );
    assert!(!ui.presses_share(one, other_table), "rows of two tables");
    assert!(!ui.presses_share(one, (50., 190.)), "a row and nothing");
}
