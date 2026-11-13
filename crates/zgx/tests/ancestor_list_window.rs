//! Lists that scroll with the view around them (`virtualize_in_ancestor`):
//! those the view has scrolled wholly past build nothing, those it shows
//! build what shows, as the view moves either way.
use std::{cell::RefCell, rc::Rc};
use zgx::{compose::prelude::*, widgets::Ui};

const LISTS: usize = 5;
const ROWS: usize = 50;
/// Each list's rows built so far.
type Built = Rc<RefCell<Vec<Vec<usize>>>>;

/// Five lists of 50 rows of 20 points, one under the other in a scroll
/// view 200 points high: each list's rows built so far.
fn transcript(ui: &mut Ui) -> (zgx::reactive::Signal<f32>, Built) {
    let offset = ui.signal(0.);
    let built = Rc::new(RefCell::new(vec![Vec::new(); LISTS]));
    let mut lists = column().w_full();
    for list in 0..LISTS {
        let built = built.clone();
        lists = lists.child(
            virtual_list(
                ui.signal(0.),
                20.,
                0,
                || ROWS,
                |n| n,
                move |_, n, _| {
                    built.borrow_mut()[list].push(n);
                    div().w_full().h(20.)
                },
            )
            .virtualize_in_ancestor()
            .w_full(),
        );
    }
    ui.mount(scroll(offset.clone()).w(300.).h(200.).child(lists));
    ui.prepare_frame();
    (offset, built)
}

fn take(built: &Built) -> Vec<Vec<usize>> {
    let mut taken = std::mem::replace(&mut *built.borrow_mut(), vec![Vec::new(); LISTS]);
    for rows in &mut taken {
        rows.sort_unstable();
        rows.dedup();
    }
    taken
}

#[test]
fn lists_the_view_has_scrolled_past_build_nothing_either_way() {
    let mut ui = Ui::new(400., 300.);
    let (offset, built) = transcript(&mut ui);
    let first = take(&built);
    assert_eq!(first[0], (0..10).collect::<Vec<_>>(), "{first:?}");
    assert!(first[1..].iter().all(Vec::is_empty), "{first:?}");
    // Into the middle of the third list: only its shown rows build.
    offset.set(2500.);
    ui.prepare_frame();
    let middle = take(&built);
    assert_eq!(middle[2], (25..35).collect::<Vec<_>>(), "{middle:?}");
    for (list, rows) in middle.iter().enumerate() {
        if list != 2 {
            assert!(rows.is_empty(), "list {list} built {rows:?}");
        }
    }
    // Across the seam of the fourth and fifth, then back to the top.
    offset.set(3900.);
    ui.prepare_frame();
    let seam = take(&built);
    assert_eq!(seam[3], (45..50).collect::<Vec<_>>(), "{seam:?}");
    assert_eq!(seam[4], (0..5).collect::<Vec<_>>(), "{seam:?}");
    assert!(seam[..3].iter().all(Vec::is_empty), "{seam:?}");
    offset.set(0.);
    ui.prepare_frame();
    let top = take(&built);
    assert_eq!(top[0], (0..10).collect::<Vec<_>>(), "{top:?}");
    assert!(top[1..].iter().all(Vec::is_empty), "{top:?}");
}
