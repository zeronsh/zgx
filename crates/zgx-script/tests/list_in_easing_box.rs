//! A list of a set height in a column, in a box that clips while its
//! height eases shut and open (a fold): each frame of the ease lays out in
//! one go. The column keeps the list's height (its min-content, as CSS's),
//! and the box clips it; before, the list's floor followed the rows it had
//! built for the room it was given, and flipped between none and all of
//! them until layout gave up.
mod common;
use common::Harness;

const APP: &str = r#"
local rows = {}
for i = 1, 200 do
    table.insert(rows, { id = `r{i}`, title = `Row {i}` })
end
local data = ui.state("rows", rows)
local amount = ui.state("amount", 1)
ui.command { id = "shut", run = function() anime.animate(amount, { value = 0, duration = 240, ease = "out(3)" }) end }
ui.command { id = "open", run = function() anime.animate(amount, { value = 1, duration = 240, ease = "out(3)" }) end }
local LIST_H = 12 * 32
return ui.column {
    w_full = true,
    ui.div {
        id = "box",
        w_full = true,
        overflow = "hidden",
        h = function(): number return math.floor(LIST_H * amount:get() + 0.5) end,
        ui.column {
            id = "column",
            w_full = true,
            ui.list {
                id = "list",
                data = data,
                key = function(r: any): string return r.id end,
                row = function(_r: any, _i: State<number>, now: State<any>): Node
                    return ui.row { w_full = true, h = 30,
                        ui.row { w_full = true, h_full = true,
                            ui.text { function(): string return now:get().title end } } }
                end,
                height = 32,
                w_full = true,
                h = LIST_H,
            },
        },
    },
}
"#;

#[test]
fn a_list_in_a_box_easing_shut_and_open_lays_out_every_frame() {
    let mut h = Harness::inline(APP);
    h.ui.scene.borrow_mut().resize(1000., 900.);
    h.frames(5);
    let mut heights = Vec::new();
    for command in ["shut", "open"] {
        assert!(h.host.run_command(command));
        for _ in 0..30 {
            h.frames(1);
            // Each frame settles: never an error, the list its own height
            // and the column holding it, whatever the box gives them.
            assert_eq!(h.error(), None);
            assert_eq!(h.ui.try_prepare_frame().ok(), Some(()));
            let (bx, column, list) = (h.bounds("box"), h.bounds("column"), h.bounds("list"));
            assert_eq!(list.height, 384., "box {}", bx.height);
            assert_eq!(column.height, 384., "box {}", bx.height);
            heights.push(bx.height);
        }
    }
    // The box went all the way shut and open again.
    assert!(heights.contains(&0.), "{heights:?}");
    assert_eq!(heights.last(), Some(&384.));
    assert!(h.find("\"Row 1\"").is_some());
}
