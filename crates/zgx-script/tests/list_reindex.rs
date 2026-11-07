//! A measured list's rows moving under it: each row's height goes with it, by
//! the pass after it moves, whether the rows are reordered or more arrive.
use zgx_script::Value;
use zgx_script::testing::TestApp;

const SCRIPT: &str = r##"
local rows = ui.shared("list.rows", {})
return ui.list {
    id = "list",
    w_full = true,
    h_full = true,
    data = rows,
    key = "id",
    measured = true,
    row = function(r)
        return ui.div { id = "row-" .. r.id, w_full = true, h = r.h }
    end,
}
"##;
fn rows(list: &[(&str, f64)]) -> Value {
    Value::List(std::rc::Rc::from(
        list.iter()
            .map(|(id, h)| Value::record([("id", Value::from(*id)), ("h", Value::Num(*h))]))
            .collect::<Vec<_>>(),
    ))
}
/// Each row where the rows above it put it.
fn assert_stacked(app: &mut TestApp, list: &[(&str, f64)]) {
    let top = app.bounds("#list").y;
    let mut y = top;
    for (id, h) in list {
        let row = app.bounds(&format!("#row-{id}"));
        assert_eq!(row.height, *h as f32, "{id}'s height");
        assert_eq!(row.y, y, "{id} under what's above it, in {list:?}");
        y += *h as f32;
    }
}

#[test]
fn rows_moving_under_a_measured_list_take_their_heights_with_them() {
    let mut app = TestApp::inline(SCRIPT);
    app.ui.scene.borrow_mut().resize(400., 800.);
    let order = [("a", 10.), ("b", 40.), ("c", 90.), ("d", 20.)];
    app.host.shared("list.rows", Value::Nil).set(rows(&order));
    app.frames(10);
    assert_stacked(&mut app, &order);
    // Reordered: the same rows, none new, so nothing else asks for a pass.
    let reversed = [("d", 20.), ("c", 90.), ("b", 40.), ("a", 10.)];
    app.host
        .shared("list.rows", Value::Nil)
        .set(rows(&reversed));
    app.frames(10);
    assert_stacked(&mut app, &reversed);
    // One more above them all.
    let more = [("e", 70.), ("d", 20.), ("c", 90.), ("b", 40.), ("a", 10.)];
    app.host.shared("list.rows", Value::Nil).set(rows(&more));
    app.frames(10);
    assert_stacked(&mut app, &more);
}

/// Sizes published elsewhere in the window wake a measured list's pass,
/// which has nothing to do then; its own rows resizing, and its scroll
/// moving, still move what it shows.
#[test]
fn a_measured_list_woken_by_sizes_elsewhere_still_follows_its_own() {
    const OTHER: &str = r##"
local rows = ui.shared("list.rows", {})
local note = ui.shared("note", "short")
local offset = ui.shared("list.offset", 0)
return ui.column {
    w_full = true,
    h_full = true,
    ui.text { "{note}", id = "note", wrap = true, w = 120 },
    ui.list {
        id = "list",
        w_full = true,
        h = 200,
        data = rows,
        key = "id",
        offset = offset,
        measured = true,
        row = function(r)
            return ui.div { id = "row-" .. r.id, w_full = true, h = r.h }
        end,
    },
}
"##;
    let mut app = TestApp::inline(OTHER);
    app.ui.scene.borrow_mut().resize(400., 800.);
    let list: Vec<(String, f64)> = (0..40).map(|i| (format!("r{i}"), 30.)).collect();
    let as_rows = |list: &[(String, f64)]| {
        Value::List(std::rc::Rc::from(
            list.iter()
                .map(|(id, h)| {
                    Value::record([("id", Value::from(id.as_str())), ("h", Value::Num(*h))])
                })
                .collect::<Vec<_>>(),
        ))
    };
    app.host.shared("list.rows", Value::Nil).set(as_rows(&list));
    app.frames(10);
    let first = app.bounds("#row-r0").y;
    // Something else resizes, again and again.
    let note = app.host.shared("note", Value::Nil);
    for words in 1..6 {
        note.set(Value::from("word ".repeat(words * 8).as_str()));
        app.frames(4);
    }
    let first_now = app.bounds("#row-r0").y;
    let moved = first_now - first;
    // Its own row resized: the next row follows it.
    let mut taller = list.clone();
    taller[0].1 = 70.;
    app.host
        .shared("list.rows", Value::Nil)
        .set(as_rows(&taller));
    app.frames(10);
    assert_eq!(app.bounds("#row-r1").y, app.bounds("#row-r0").y + 70.);
    // Scrolled: what it shows moves.
    app.host
        .shared("list.offset", Value::Nil)
        .set(Value::Num(300.));
    app.frames(10);
    assert!(
        app.find("#row-r0").is_none() || app.bounds("#row-r0").y < first_now - 200.,
        "scrolled"
    );
    assert!(moved > 0., "the note above it grew, so the list moved down");
}
