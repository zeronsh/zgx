//! Rows built ahead of a list's view in spare time, and kept as it scrolls
//! away (`zgx::spare`).
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};
use zgx::{compose::prelude::*, reactive::Signal, scene::NodeId, widgets::Ui};

/// All the spare time asked for.
fn spare(ui: &Ui) {
    let mut turns = 0;
    while ui.spare_work_wanted() {
        ui.run_spare_work(Instant::now(), Duration::from_secs(60))
            .unwrap();
        turns += 1;
        assert!(turns < 10_000, "spare work never ends");
    }
}

fn hosted() -> Ui {
    let ui = Ui::new(400., 100.);
    ui.host_spare_time();
    ui
}

/// Rows built so far and those still alive, by index.
#[derive(Clone, Default)]
struct Rows {
    built: Rc<RefCell<Vec<usize>>>,
    alive: Rc<Cell<usize>>,
}
struct Alive(Rc<Cell<usize>>);
impl Drop for Alive {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}
impl Rows {
    fn row(&self, index: usize, height: f32, cx: &mut Context) -> View {
        self.built.borrow_mut().push(index);
        self.alive.set(self.alive.get() + 1);
        cx.retain(Alive(self.alive.clone()));
        div().w_full().h(height)
    }
    fn take(&self) -> Vec<usize> {
        let mut built = self.built.take();
        built.sort_unstable();
        built
    }
}

/// A measured list of `count` rows of `height`, estimated at 10.
fn measured(
    ui: &mut Ui,
    count: usize,
    height: f32,
) -> (Signal<f32>, VariableHeights, Rows, NodeId) {
    let offset = ui.signal(0.);
    let heights = VariableHeights::new(&ui.runtime, count, 10.);
    let rows = Rows::default();
    let built = rows.clone();
    let list = ui.mount(
        measured_virtual_list(
            offset.clone(),
            heights.clone(),
            0,
            |i| i,
            move |_, i, cx| built.row(i, height, cx),
        )
        .w(400.)
        .h(100.),
    );
    ui.prepare_frame();
    moves(ui, &offset);
    (offset, heights, rows, list.node())
}

fn fixed(ui: &mut Ui, count: usize) -> (Signal<f32>, Rows) {
    let offset = ui.signal(0.);
    let rows = Rows::default();
    let built = rows.clone();
    ui.mount(
        virtual_list(
            offset.clone(),
            20.,
            0,
            move || count,
            |i| i,
            move |_, i, cx| built.row(i, 20., cx),
        )
        .w(400.)
        .h(100.),
    );
    ui.prepare_frame();
    moves(ui, &offset);
    (offset, rows)
}

/// A list builds ahead once its view has moved.
fn moves(ui: &Ui, offset: &Signal<f32>) {
    offset.set(1.);
    ui.prepare_frame();
    offset.set(0.);
    ui.prepare_frame();
}

fn scroll_to(ui: &Ui, offset: &Signal<f32>, to: f32) {
    offset.set(to);
    ui.prepare_frame();
}

#[test]
fn rows_a_viewport_beyond_the_view_are_built_before_they_show() {
    let mut ui = hosted();
    let (offset, _, rows, _) = measured(&mut ui, 1000, 20.);
    // (A row's height settling can build it twice, as ever.)
    let mut at_first = rows.take();
    at_first.dedup();
    assert!(at_first.len() < 12, "the view's rows: {at_first:?}");
    spare(&ui);
    let ahead = rows.take();
    assert!(!ahead.is_empty());
    assert!(
        (ahead.iter().max().copied().unwrap() as f32 + 1.) * 20. >= 200.,
        "a viewport beyond the view, measured: {ahead:?}"
    );
    // Within what was built, the frame builds nothing.
    scroll_to(&ui, &offset, 90.);
    assert_eq!(rows.take(), Vec::<usize>::new());
}

#[test]
fn building_ahead_goes_the_way_the_view_went_first() {
    let mut ui = hosted();
    let (offset, rows) = fixed(&mut ui, 1000);
    scroll_to(&ui, &offset, 4000.);
    spare(&ui);
    rows.take();
    // Back up, a long way: the rows above come first.
    scroll_to(&ui, &offset, 2000.);
    rows.built.borrow_mut().clear();
    ui.run_spare_work(Instant::now(), Duration::from_secs(60))
        .ok();
    let built = rows.built.borrow().clone();
    assert!(built.first().is_some_and(|row| *row < 100), "{built:?}");
}

#[test]
fn scrolling_back_builds_nothing() {
    for list in ["measured", "fixed"] {
        let mut ui = hosted();
        let (offset, rows) = match list {
            "measured" => {
                let (offset, _, rows, _) = measured(&mut ui, 1000, 20.);
                (offset, rows)
            }
            _ => fixed(&mut ui, 1000),
        };
        spare(&ui);
        for to in [100., 200., 300.] {
            scroll_to(&ui, &offset, to);
            spare(&ui);
        }
        rows.take();
        for to in [200., 100., 0., 100., 200., 300.] {
            scroll_to(&ui, &offset, to);
        }
        assert_eq!(rows.take(), Vec::<usize>::new(), "{list}");
    }
}

#[test]
fn rows_past_the_distance_kept_are_dropped_farthest_first() {
    let mut ui = hosted();
    let (offset, rows) = fixed(&mut ui, 10_000);
    spare(&ui);
    let near = rows.alive.get();
    // Rows of 20 in a view of 100: 1 + 1 + 4 viewports below, at most.
    assert!(near <= 11, "{near}");
    scroll_to(&ui, &offset, 400.);
    spare(&ui);
    let kept = rows.alive.get();
    assert!(kept > near, "rows left behind stay: {kept}");
    scroll_to(&ui, &offset, 100_000.);
    assert!(
        rows.alive.get() <= 6,
        "a jump drops all it keeps: {}",
        rows.alive.get()
    );
}

#[test]
fn a_list_keeps_at_most_so_many_rows() {
    let mut ui = Ui::new(400., 4000.);
    ui.host_spare_time();
    let offset = ui.signal(0.);
    let rows = Rows::default();
    let built = rows.clone();
    ui.mount(
        virtual_list(
            offset.clone(),
            2.,
            0,
            || 100_000,
            |i| i,
            move |_, i, cx| built.row(i, 2., cx),
        )
        .w(400.)
        .h(100.),
    );
    ui.prepare_frame();
    moves(&ui, &offset);
    let mut to = 0.;
    for _ in 0..80 {
        to += 10.;
        scroll_to(&ui, &offset, to);
        spare(&ui);
    }
    // (Counted before a pass's new row.)
    assert!(rows.alive.get() <= 257, "{}", rows.alive.get());
}

#[test]
fn a_list_that_never_scrolled_builds_nothing_ahead() {
    let mut ui = hosted();
    let offset = ui.signal(0.);
    let rows = Rows::default();
    let built = rows.clone();
    ui.mount(
        virtual_list(
            offset,
            20.,
            2,
            || 1000,
            |i| i,
            move |_, i, cx| built.row(i, 20., cx),
        )
        .w(400.)
        .h(100.),
    );
    ui.prepare_frame();
    spare(&ui);
    // What shows and its overscan: what a list with no spare time holds.
    assert_eq!(rows.take(), vec![0, 1, 2, 3, 4, 5, 6]);
}

#[test]
fn at_rest_a_list_goes_back_and_frees_what_it_let_go_of_with_no_frame() {
    let mut ui = hosted();
    let (offset, rows) = fixed(&mut ui, 10_000);
    spare(&ui);
    for to in [100., 200., 300., 400.] {
        scroll_to(&ui, &offset, to);
        spare(&ui);
    }
    assert!(rows.alive.get() > 10, "{}", rows.alive.get());
    let nodes = ui.scene.borrow().len();
    // No frame: as a covered window's.
    ui.release_kept_rows().unwrap();
    assert!(rows.alive.get() <= 6, "what shows: {}", rows.alive.get());
    assert!(ui.scene.borrow().len() < nodes);
    assert!(!ui.reclaiming());
    assert!(!ui.spare_work_wanted(), "nothing wakes it");
    // Moved again, it builds ahead again.
    scroll_to(&ui, &offset, 500.);
    assert!(ui.spare_work_wanted());
}

#[test]
fn a_row_that_does_not_fit_waits_for_a_frame_it_fits() {
    let mut ui = hosted();
    let (_, rows) = fixed(&mut ui, 1000);
    rows.take();
    // A frame over its budget already builds nothing and asks for another.
    let late = Instant::now() - Duration::from_millis(100);
    let left = ui.run_spare_work(late, Duration::from_millis(8)).unwrap();
    assert_eq!(left, Duration::ZERO);
    assert_eq!(rows.take(), Vec::<usize>::new());
    assert!(ui.spare_work_wanted());
    // A frame with room builds, and says what is left once done.
    let left = ui
        .run_spare_work(Instant::now(), Duration::from_secs(60))
        .unwrap();
    assert!(!rows.take().is_empty());
    assert!(left > Duration::ZERO);
    assert!(!ui.spare_work_wanted());
}

#[test]
fn a_row_too_long_for_any_budget_still_gets_its_turn() {
    let mut ui = hosted();
    let offset = ui.signal(0.);
    let rows = Rows::default();
    let built = rows.clone();
    ui.mount(
        virtual_list(
            offset.clone(),
            20.,
            0,
            || 1000,
            |i| i,
            move |_, i, cx| {
                // Longer than half this test's refresh.
                std::thread::sleep(Duration::from_millis(3));
                built.row(i, 20., cx)
            },
        )
        .w(400.)
        .h(100.),
    );
    ui.prepare_frame();
    moves(&ui, &offset);
    rows.take();
    let interval = Duration::from_millis(4);
    // The first is built on trust; then what it cost says it doesn't fit.
    ui.run_spare_work(Instant::now(), interval).unwrap();
    assert_eq!(rows.take().len(), 1);
    let mut built = 0;
    for _ in 0..6 {
        ui.run_spare_work(Instant::now(), interval).unwrap();
        built += rows.take().len();
    }
    assert!(built >= 1, "it waits a few frames, not for ever");
    assert!(built <= 2, "and doesn't take every frame: {built}");
}

#[test]
fn a_list_can_say_it_builds_nothing_ahead() {
    let mut ui = hosted();
    let offset = ui.signal(0.);
    let rows = Rows::default();
    let built = rows.clone();
    ui.mount(
        virtual_list(
            offset.clone(),
            20.,
            0,
            || 1000,
            |i| i,
            move |_, i, cx| built.row(i, 20., cx),
        )
        .list_ahead(0.)
        .w(400.)
        .h(100.),
    );
    ui.prepare_frame();
    assert!(!ui.spare_work_wanted());
    scroll_to(&ui, &offset, 1000.);
    assert!(rows.alive.get() <= 6, "{}", rows.alive.get());
}

#[test]
fn without_a_host_of_spare_time_a_list_is_as_it_was() {
    let mut ui = Ui::new(400., 100.);
    let (offset, rows) = fixed(&mut ui, 1000);
    assert!(!ui.spare_work_wanted());
    scroll_to(&ui, &offset, 1000.);
    assert!(rows.alive.get() <= 6, "{}", rows.alive.get());
}

#[test]
fn rows_measured_ahead_keep_what_shows_where_it_is() {
    let mut ui = hosted();
    let (offset, heights, _, list) = measured(&mut ui, 1000, 20.);
    // Far down, by estimates: rows above are 10 tall until measured.
    scroll_to(&ui, &offset, 2000.);
    ui.prepare_frame();
    let shown_row = heights.row_at(offset.get()).unwrap();
    let shown_within = offset.get() - heights.row_offset(shown_row);
    spare(&ui);
    // Rows above it were measured 20 tall: the view moved with them.
    assert_eq!(heights.row_at(offset.get()), Some(shown_row));
    assert!(
        (offset.get() - heights.row_offset(shown_row) - shown_within).abs() < 0.01,
        "the row in view stays put"
    );
    assert_eq!(heights.row_height(shown_row.saturating_sub(1)), Some(20.));
    assert!(ui.scene.borrow().contains(list));
}

#[test]
fn rows_kept_find_their_places_when_rows_come_in_above() {
    let mut ui = hosted();
    let offset = ui.signal(0.);
    let keys = ui.signal((0..100_u32).collect::<Vec<u32>>());
    let heights = VariableHeights::new(&ui.runtime, 0, 20.);
    let rows = Rows::default();
    let built = rows.clone();
    let places: Rc<RefCell<std::collections::HashMap<u32, Signal<usize>>>> = Rc::default();
    let seen = places.clone();
    let order = keys.clone();
    ui.mount(
        measured_rows(
            offset.clone(),
            heights.clone(),
            0,
            move || order.get(),
            |_, _| f32::NAN,
            move |index, key, cx| {
                seen.borrow_mut().insert(key, index);
                built.row(key as usize, 20., cx)
            },
        )
        .w(400.)
        .h(100.),
    );
    ui.prepare_frame();
    moves(&ui, &offset);
    spare(&ui);
    scroll_to(&ui, &offset, 400.);
    spare(&ui);
    rows.take();
    // Two rows above everything: the rows kept are two further on, unbuilt.
    keys.update(|keys| {
        keys.insert(0, 1001);
        keys.insert(0, 1000);
    });
    ui.prepare_frame();
    let kept_below = places.borrow()[&30].with_untracked(|index| *index);
    assert_eq!(kept_below, 32, "a row kept out of view follows its key");
    let kept_above = places.borrow()[&10].with_untracked(|index| *index);
    assert_eq!(kept_above, 12);
    let built = rows.take();
    assert!(!built.contains(&30) && !built.contains(&10), "{built:?}");
}

#[test]
fn a_row_written_in_place_keeps_rows_kept() {
    let mut ui = hosted();
    let offset = ui.signal(0.);
    let keys = ui.signal((0..100_u32).collect::<Vec<u32>>());
    let heights = VariableHeights::new(&ui.runtime, 0, 20.);
    let rows = Rows::default();
    let built = rows.clone();
    let order = keys.clone();
    ui.mount(
        measured_rows(
            offset.clone(),
            heights,
            0,
            move || order.get(),
            |_, _| f32::NAN,
            move |_, key, cx| built.row(key as usize, 20., cx),
        )
        .w(400.)
        .h(100.),
    );
    ui.prepare_frame();
    spare(&ui);
    rows.take();
    let alive = rows.alive.get();
    keys.update(|_| {});
    ui.prepare_frame();
    assert_eq!(rows.alive.get(), alive);
    assert_eq!(rows.take(), Vec::<usize>::new());
}

#[test]
fn rows_read_in_order_with_those_kept_among_them() {
    let mut ui = hosted();
    let (offset, heights, _, list) = measured(&mut ui, 1000, 20.);
    spare(&ui);
    scroll_to(&ui, &offset, 200.);
    spare(&ui);
    ui.prepare_frame();
    // Every row at its place, top to bottom, in order.
    let scene = ui.scene.borrow();
    let mut tops = Vec::new();
    collect_rows(&scene, list, &mut tops);
    assert!(tops.len() > 5);
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "{tops:?}");
    assert_eq!(heights.row_height(0), Some(20.));
}

/// The tops of the list's rows: the children of its deepest column of
/// overlays, as its viewport holds its content.
fn collect_rows(scene: &zgx::scene::Scene, node: NodeId, tops: &mut Vec<f32>) {
    let children = scene.children(node);
    if children.len() > 3 {
        tops.extend(children.iter().map(|row| scene.bounds(*row).y));
        return;
    }
    for child in children.iter().copied() {
        collect_rows(scene, child, tops);
    }
}

#[test]
fn a_new_width_drops_rows_out_of_view_to_be_built_again() {
    let mut ui = hosted();
    let offset = ui.signal(0.);
    let heights = VariableHeights::new(&ui.runtime, 1000, 10.);
    let rows = Rows::default();
    let built = rows.clone();
    ui.mount(
        measured_virtual_list(
            offset.clone(),
            heights,
            0,
            |i| i,
            move |_, i, cx| built.row(i, 20., cx),
        )
        .w_full()
        .h(100.),
    );
    ui.prepare_frame();
    moves(&ui, &offset);
    spare(&ui);
    let all = rows.alive.get();
    assert!(all >= 10, "{all}");
    ui.scene.borrow_mut().resize(300., 100.);
    ui.prepare_frame();
    assert!(rows.alive.get() < all, "{} of {all}", rows.alive.get());
    assert!(ui.spare_work_wanted(), "and built again in spare time");
    spare(&ui);
    assert!(rows.alive.get() >= 10);
}

#[test]
fn a_row_with_a_popover_open_goes_as_it_leaves() {
    let mut ui = hosted();
    let offset = ui.signal(0.);
    let open = ui.signal(false);
    let rows = Rows::default();
    let built = rows.clone();
    let showing = open.clone();
    let view = ui.mount(
        virtual_list(
            offset.clone(),
            20.,
            0,
            || 1000,
            |i| i,
            move |_, i, cx| {
                built.row(i, 20., cx);
                if i != 0 {
                    return div().w_full().h(20.);
                }
                popover(
                    "Details",
                    showing.clone(),
                    button().id("anchor").size(100., 20.).child(text("Open")),
                )
                .id("popup")
                .size(100., 40.)
            },
        )
        .w(400.)
        .h(100.),
    );
    ui.prepare_frame();
    moves(&ui, &offset);
    spare(&ui);
    let anchor = view.find("anchor").unwrap();
    ui.input.focus(&ui.scene, Some(anchor));
    open.set(true);
    ui.prepare_frame();
    assert!(view.find("popup").is_some());
    // Well within what would be kept.
    scroll_to(&ui, &offset, 200.);
    assert!(view.find("popup").is_none());
}

#[test]
fn a_list_in_a_scroll_view_builds_ahead_of_what_that_view_shows() {
    let mut ui = hosted();
    let rows = Rows::default();
    let built = rows.clone();
    let offset = ui.signal(0.);
    let inner = ui.signal(0.);
    ui.mount(
        scroll(offset.clone())
            .child(
                virtual_list(
                    inner,
                    20.,
                    0,
                    || 1000,
                    |i| i,
                    move |_, i, cx| built.row(i, 20., cx),
                )
                .virtualize_in_ancestor()
                .w(400.),
            )
            .w(400.)
            .h(100.),
    );
    ui.prepare_frame();
    moves(&ui, &offset);
    let at_first = rows.take();
    assert!(at_first.len() <= 6, "{at_first:?}");
    spare(&ui);
    let ahead = rows.take();
    assert!(ahead.contains(&9), "a viewport below: {ahead:?}");
    assert!(!ahead.contains(&30), "{ahead:?}");
    // Over what was built, the view's scroll builds nothing.
    offset.set(90.);
    ui.prepare_frame();
    assert_eq!(rows.take(), Vec::<usize>::new());
}

#[test]
fn overscan_is_built_in_spare_time_with_any_to_spare() {
    let mut ui = hosted();
    let offset = ui.signal(0.);
    let rows = Rows::default();
    let built = rows.clone();
    // Rows taller than the view: 3 of them go further than a viewport.
    ui.mount(
        virtual_list(
            offset.clone(),
            500.,
            3,
            || 1000,
            |i| i,
            move |_, i, cx| built.row(i, 500., cx),
        )
        .w(400.)
        .h(100.),
    );
    ui.prepare_frame();
    assert_eq!(rows.take(), vec![0], "the frame builds what shows");
    spare(&ui);
    assert_eq!(rows.take(), vec![1, 2, 3]);
    assert!(!ui.spare_work_wanted());
    // Scrolled onto them, the frame builds nothing.
    scroll_to(&ui, &offset, 600.);
    assert_eq!(rows.take(), Vec::<usize>::new());
    spare(&ui);
    assert_eq!(rows.take(), vec![4]);
}

/// The files the kept pictures below decoded, by name, in order.
static DECODED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
/// Held by a test of pictures decoded ahead: the decodes waiting to be made
/// ahead are the process's (`AHEAD_MOST`), so two such tests at once would
/// take each other's room.
static PICTURES: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn pictures_alone() -> std::sync::MutexGuard<'static, ()> {
    PICTURES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
/// The names decoded so far (whatever a failed test left them).
fn decoded_names() -> Vec<String> {
    DECODED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}
/// A 300 by 300 picture's file (larger than `SYNC_PIXELS`, so decoded off
/// the main thread), its name kept as it decodes.
fn picture(bytes: &[u8]) -> Result<std::sync::Arc<[u8]>, String> {
    DECODED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(String::from_utf8_lossy(bytes).into_owned());
    Ok([90, 120, 200, 255].repeat(300 * 300).into())
}
/// Whether `row`'s picture has decoded, waiting for it a while.
fn decoded(row: usize) -> bool {
    let name = format!("row {row}");
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if decoded_names().contains(&name) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

/// A picture in a row built ahead, out of view, starts decoding in spare
/// time, so it is there as the row shows: the rows the view goes towards,
/// nearest first, a few at a time (`AHEAD_MOST`), the rest once there's
/// room again. One in view decodes as it is drawn, never ahead.
#[test]
fn pictures_in_rows_built_ahead_decode_before_they_show() {
    use std::sync::Arc;
    use zgx::image::{AHEAD_MOST, ImageData};
    let _alone = pictures_alone();
    let mut ui = hosted();
    let pictures: Vec<Arc<ImageData>> = (0..1000)
        .map(|row| {
            let file: Arc<[u8]> = format!("row {row}").into_bytes().into();
            Arc::new(ImageData::kept(300, 300, file, picture).unwrap())
        })
        .collect();
    let offset = ui.signal(0.);
    ui.mount(
        virtual_list(
            offset.clone(),
            20.,
            0,
            move || 1000,
            |i| i,
            move |_, i, _| {
                div()
                    .w_full()
                    .h(20.)
                    .child(image("", pictures[i].clone()).w(400.).h(20.))
            },
        )
        .w(400.)
        .h(100.),
    );
    ui.prepare_frame();
    moves(&ui, &offset);
    spare(&ui);
    // Rows 0 to 4 show; 5 to 9 are a viewport ahead.
    for row in 5..5 + AHEAD_MOST {
        assert!(
            decoded(row),
            "row {row}'s picture, ahead: {:?}",
            decoded_names()
        );
    }
    let asked = decoded_names();
    assert!(
        (0..5).all(|row| !asked.contains(&format!("row {row}"))),
        "a picture in view decoded ahead: {asked:?}"
    );
    // The view moves on; the next turn asks for the next row's.
    scroll_to(&ui, &offset, 20.);
    spare(&ui);
    assert!(decoded(9), "row 9's picture: {:?}", decoded_names());
}
