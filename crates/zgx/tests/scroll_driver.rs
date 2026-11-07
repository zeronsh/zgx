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

/// A row that scrolls across, inside a page that scrolls down, as UIKit's
/// nested scroll views measured in a reference app: a finger that first
/// moves across past the slop scrolls the row, down the page.
#[test]
fn nested_views_give_a_finger_to_the_one_scrolling_its_way() {
    let mut ui = Ui::new(100., 100.);
    let (page, row) = (ui.signal(0.), ui.signal(0.));
    ui.mount(
        scroll(page).w(100.).h(100.).child(
            div()
                .w(100.)
                .h(1000.)
                .child(scroll_x(row).w(100.).h(40.).child(div().w(400.).h(40.))),
        ),
    );
    ui.prepare_frame();
    let across = ui.scroll_driver_along(50., 20., true).expect("the row");
    let down = ui.scroll_driver_along(50., 20., false).expect("the page");
    assert!(across.metrics().unwrap().horizontal);
    assert!(!down.metrics().unwrap().horizontal);
    assert!(!across.same(&down));
    // Either way: the innermost, the row.
    assert!(ui.scroll_driver_at(50., 20.).unwrap().same(&across));
    // Below the row only the page scrolls, and nothing across.
    assert!(ui.scroll_driver_along(50., 70., false).unwrap().same(&down));
    assert!(ui.scroll_driver_along(50., 70., true).is_none());
}

/// iOS's scroller lies over the view it stands for, where UIKit draws the
/// view's scroll indicator: the finder gives that view's box in the window.
#[test]
fn a_driven_view_is_found_where_it_shows() {
    let mut ui = Ui::new(100., 200.);
    let (page, row) = (ui.signal(0.), ui.signal(0.));
    ui.mount(
        scroll(page).w(100.).h(200.).child(
            div()
                .w(100.)
                .h(1000.)
                .child(div().h(30.))
                .child(scroll_x(row).w(100.).h(40.).child(div().w(400.).h(40.))),
        ),
    );
    ui.prepare_frame();
    let frame_of = ui.scroll_driver_frame_finder();
    let across = ui.scroll_driver_along(50., 50., true).expect("the row");
    let down = ui.scroll_driver_along(50., 50., false).expect("the page");
    let rect = frame_of(&across).unwrap();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (0., 30., 100., 40.)
    );
    let rect = frame_of(&down).unwrap();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (0., 0., 100., 200.)
    );
    // Scrolled, the row shows higher.
    down.drive(20.);
    ui.prepare_frame();
    assert_eq!(frame_of(&across).unwrap().y, 10.);
}
