//! A list that scrolls with the view around it (`virtualize_in_ancestor`),
//! mounted under a box fading in from nothing (a presence's `enter`): it
//! builds the rows in view on its first frame, as one not fading does, not
//! once something around it moves. Which rows a list builds is geometry:
//! its rows under a fade are about to show.
use std::{cell::RefCell, rc::Rc};
use zgx::{compose::prelude::*, scene::Effects, widgets::Ui};

/// A list of 50 rows of 20 points in a scroll view 200 points high, under a
/// box at `opacity`: the box, and the rows built so far.
fn faded_list(ui: &mut Ui, opacity: f32) -> (zgx::scene::NodeId, Rc<RefCell<Vec<usize>>>) {
    let built = Rc::new(RefCell::new(Vec::new()));
    let list = {
        let built = built.clone();
        virtual_list(
            ui.signal(0.),
            20.,
            0,
            || 50,
            |n| n,
            move |_, n, _| {
                built.borrow_mut().push(n);
                div().w_full().h(20.)
            },
        )
        .virtualize_in_ancestor()
        .w_full()
    };
    let view = ui.mount(
        scroll(ui.signal(0.)).w(300.).h(200.).child(
            column()
                .w_full()
                .child(div().h(30.))
                .child(div().id("fade").w_full().opacity(opacity).child(list)),
        ),
    );
    ui.prepare_frame();
    (view.find("fade").expect("the fading box"), built)
}

fn rows(built: &Rc<RefCell<Vec<usize>>>) -> Vec<usize> {
    let mut rows = built.borrow().clone();
    rows.sort_unstable();
    rows.dedup();
    rows
}

#[test]
fn a_list_under_a_box_fading_in_builds_its_rows_in_view_at_once() {
    let mut ui = Ui::new(400., 300.);
    let (fade, built) = faded_list(&mut ui, 0.);
    // 200 points of view, the list 30 down it: rows 0 to 8 show.
    let first = rows(&built);
    assert_eq!(
        first,
        (0..9).collect::<Vec<_>>(),
        "first frame built {first:?}"
    );
    // The fade rising moves nothing: the rows built stay, none more.
    ui.scene.borrow_mut().set_effects(
        fade,
        Effects {
            opacity: 1.,
            ..Effects::default()
        },
    );
    ui.prepare_frame();
    assert_eq!(rows(&built), first);
}

#[test]
fn a_list_under_a_box_shown_builds_the_same_rows() {
    let mut ui = Ui::new(400., 300.);
    let (_, built) = faded_list(&mut ui, 1.);
    assert_eq!(rows(&built), (0..9).collect::<Vec<_>>());
}
