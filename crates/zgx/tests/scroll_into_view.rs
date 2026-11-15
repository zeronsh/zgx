//! A node scrolled into view as the web's `element.scrollIntoView`
//! (`NodeHandle::scroll_into_view`, `Ui::scroll_node_into_view`): each
//! scroll view around it, innermost first, moves so it shows where asked
//! (`Nearest` leaving one that shows it alone), within what it can scroll;
//! whether any moved; easing there unless the system reduces motion.
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
use zgx::{
    compose::{NodeHandle, RowAlign, ScrollIntoView, prelude::*},
    input::InputEvent,
    theme::{SystemPreferences, Themes},
    widgets::Ui,
};

fn how(block: RowAlign, inline: RowAlign) -> ScrollIntoView {
    ScrollIntoView {
        block,
        inline,
        animate: Some(false),
    }
}
fn down(block: RowAlign) -> ScrollIntoView {
    how(block, RowAlign::Nearest)
}

/// A 200 × 100 scroll view of ten rows 40 high, `r0` to `r9`: it scrolls
/// 300 at most.
fn rows(ui: &mut Ui) -> (zgx::reactive::Signal<f32>, zgx::compose::ViewHandle) {
    let offset = ui.signal(0.);
    let view = ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .children((0..10).map(|n| div().id(format!("r{n}")).h(40.))),
    );
    ui.prepare_frame();
    (offset, view)
}

#[test]
fn each_alignment_puts_the_row_where_it_says() {
    let mut ui = Ui::new(400., 300.);
    let (offset, view) = rows(&mut ui);
    let r5 = view.find("r5").unwrap();
    // r5 is 200 to 240 down the rows.
    assert!(ui.scroll_node_into_view(r5, down(RowAlign::Start)));
    assert_eq!(offset.get(), 200.);
    assert!(
        !ui.scroll_node_into_view(r5, down(RowAlign::Start)),
        "already there: none moved"
    );
    assert!(ui.scroll_node_into_view(r5, down(RowAlign::Center)));
    assert_eq!(offset.get(), 170.);
    assert!(ui.scroll_node_into_view(r5, down(RowAlign::End)));
    assert_eq!(offset.get(), 140.);
    assert!(
        !ui.scroll_node_into_view(r5, down(RowAlign::Nearest)),
        "it shows: nearest leaves it"
    );
    offset.set(0.);
    ui.prepare_frame();
    assert!(ui.scroll_node_into_view(r5, down(RowAlign::Nearest)));
    assert_eq!(
        offset.get(),
        140.,
        "as little as shows it: its bottom at the bottom"
    );
    // As far as it scrolls, and no further.
    assert!(ui.scroll_node_into_view(view.find("r9").unwrap(), down(RowAlign::Start)));
    assert_eq!(offset.get(), 300.);
    offset.set(0.);
    ui.prepare_frame();
    assert!(
        !ui.scroll_node_into_view(view.find("r0").unwrap(), down(RowAlign::Start)),
        "at its top already"
    );
}

#[test]
fn across_a_horizontal_scroll_view_too() {
    let mut ui = Ui::new(400., 300.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        scroll_x(offset.clone())
            .w(100.)
            .h(40.)
            .children((0..10).map(|n| div().id(format!("c{n}")).w(40.).h(40.).shrink_0())),
    );
    ui.prepare_frame();
    let c5 = view.find("c5").unwrap();
    assert!(ui.scroll_node_into_view(c5, how(RowAlign::Nearest, RowAlign::Start)));
    assert_eq!(offset.get(), 200.);
    assert!(ui.scroll_node_into_view(c5, how(RowAlign::Nearest, RowAlign::End)));
    assert_eq!(offset.get(), 140.);
    assert!(!ui.scroll_node_into_view(c5, how(RowAlign::Nearest, RowAlign::Nearest)));
}

#[test]
fn nested_scroll_views_each_move_innermost_first() {
    let mut ui = Ui::new(500., 400.);
    let (outer, inner) = (ui.signal(0.), ui.signal(0.));
    let view = ui.mount(
        scroll(outer.clone())
            .w(250.)
            .h(120.)
            .child(div().h(150.))
            .child(
                scroll(inner.clone())
                    .w(200.)
                    .h(100.)
                    .child(div().h(200.))
                    .child(div().id("target").h(20.))
                    .child(div().h(100.)),
            )
            .child(div().h(200.)),
    );
    ui.prepare_frame();
    let target = view.find("target").unwrap();
    assert!(ui.scroll_node_into_view(target, down(RowAlign::Start)));
    ui.prepare_frame();
    // The inner view brings it to its top, the outer brings that to its own.
    assert_eq!(inner.get(), 200.);
    assert_eq!(outer.get(), 150.);
    let (shown, frame) = {
        let scene = ui.scene.borrow();
        (
            scene.transformed_bounds(target),
            scene.transformed_bounds(view.node()),
        )
    };
    assert_eq!(shown.y, frame.y, "at the top of what shows");
    assert!(
        !ui.scroll_node_into_view(target, down(RowAlign::Nearest)),
        "showing in both: nearest moves neither"
    );
}

#[test]
fn inside_a_popovers_scroll_view() {
    let mut ui = Ui::new(400., 300.);
    let (open, offset) = (ui.signal(false), ui.signal(0.));
    let view = ui.mount(
        column().p(20.).child(
            popover(
                "Options",
                open.clone(),
                button()
                    .id("anchor")
                    .size(40., 20.)
                    .p(0.)
                    .child(text("Open")),
            )
            .size(120., 80.)
            .p(0.)
            .child(
                scroll(offset.clone())
                    .w(120.)
                    .h(80.)
                    .children((0..8).map(|n| div().id(format!("o{n}")).h(30.))),
            ),
        ),
    );
    open.set(true);
    ui.prepare_frame();
    let o6 = view.find("o6").expect("the popover shows");
    // o6 is 180 to 210 in the list: its bottom at the view's bottom.
    assert!(ui.scroll_node_into_view(o6, down(RowAlign::End)));
    assert_eq!(offset.get(), 130.);
}

#[test]
fn a_virtual_row_scrolled_out_of_its_list_moves_nothing() {
    let mut ui = Ui::new(400., 300.);
    let offset = ui.signal(0.);
    let handles: Rc<RefCell<Vec<(usize, NodeHandle)>>> = Rc::default();
    let kept = handles.clone();
    ui.mount(
        virtual_list(
            offset.clone(),
            20.,
            0,
            || 1000,
            |i| i,
            move |_, key, _| {
                let kept = kept.clone();
                div()
                    .h(20.)
                    .on_mount(move |handle| kept.borrow_mut().push((key, handle)))
            },
        )
        .w(200.)
        .h(100.),
    );
    ui.prepare_frame();
    let row2 = handles
        .borrow()
        .iter()
        .find(|(key, _)| *key == 2)
        .map(|(_, handle)| handle.clone())
        .expect("row 2 mounted");
    // Scrolled far away: its row is no longer mounted.
    offset.set(15_000.);
    ui.prepare_frame();
    assert!(!row2.scroll_into_view(down(RowAlign::Start)));
    assert_eq!(offset.get(), 15_000., "nothing moved");
}

#[test]
fn it_eases_there_unless_the_system_reduces_motion() {
    let mut ui = Ui::new(400., 300.);
    let (offset, view) = rows(&mut ui);
    let r5 = view.find("r5").unwrap();
    let eased = ScrollIntoView {
        block: RowAlign::Start,
        ..ScrollIntoView::default()
    };
    // Motion as usual: it eases there over a few frames.
    let start = Instant::now();
    assert!(ui.scroll_node_into_view(r5, eased));
    assert_eq!(offset.get(), 0., "not there at once");
    ui.advance_interactions(start + Duration::from_millis(150))
        .unwrap();
    let midway = offset.get();
    assert!(midway > 0. && midway < 200., "on its way: {midway}");
    ui.advance_interactions(start + Duration::from_millis(400))
        .unwrap();
    assert_eq!(offset.get(), 200.);
    assert!(
        ui.next_interaction_deadline().is_none(),
        "nothing left to step"
    );
    // Reduced motion: there at once.
    offset.set(0.);
    ui.prepare_frame();
    Themes::of(&ui.runtime).set_system_preferences(SystemPreferences {
        reduced_motion: true,
        ..SystemPreferences::default()
    });
    assert!(ui.scroll_node_into_view(r5, eased));
    assert_eq!(offset.get(), 200.);
    // Asked to animate, it does, reduced motion or not.
    offset.set(0.);
    ui.prepare_frame();
    let start = Instant::now();
    assert!(ui.scroll_node_into_view(
        r5,
        ScrollIntoView {
            animate: Some(true),
            ..eased
        }
    ));
    assert_eq!(offset.get(), 0.);
    ui.advance_interactions(start + Duration::from_millis(400))
        .unwrap();
    assert_eq!(offset.get(), 200.);
}

#[test]
fn the_person_scrolling_stops_an_ease_where_it_is() {
    let mut ui = Ui::new(400., 300.);
    let (offset, view) = rows(&mut ui);
    let start = Instant::now();
    assert!(ui.scroll_node_into_view(
        view.find("r5").unwrap(),
        ScrollIntoView {
            animate: Some(true),
            ..down(RowAlign::Start)
        }
    ));
    ui.advance_interactions(start + Duration::from_millis(100))
        .unwrap();
    ui.dispatch(InputEvent::Scroll {
        x: 50.,
        y: 50.,
        delta_x: 0.,
        delta_y: 10.,
    });
    let after = offset.get();
    ui.advance_interactions(start + Duration::from_millis(400))
        .unwrap();
    assert_eq!(offset.get(), after, "the wheel's, not the reveal's");
}
