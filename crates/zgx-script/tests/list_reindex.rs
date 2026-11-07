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
