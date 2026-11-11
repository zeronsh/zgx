//! A platform's own scroller driving a scroll view (iOS's `UIScrollView`,
//! headless here): it finds what is under a finger, its positions move the
//! view, and past either end the offset an app reads stays in range while the
//! rest shows as stretch.
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
