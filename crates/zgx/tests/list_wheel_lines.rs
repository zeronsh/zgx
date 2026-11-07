//! A wheel's line over a list is a row's height (`Ui::wheel_line_at`), as a
//! table's rows set an `NSTableView`'s; elsewhere the platform's.
use zgx::{compose::prelude::*, widgets::Ui};

#[test]
fn over_a_list_a_wheels_line_is_a_row() {
    let mut ui = Ui::new(600., 400.);
    let offset = ui.signal(0_f32);
    let heights = VariableHeights::new(&ui.runtime, 50, 30.);
    let measured = ui.signal(0_f32);
    let fixed = ui.mount(
        div()
            .flex_row()
            .child(
                virtual_list(
                    offset.clone(),
                    24.,
                    1,
                    || 100,
                    |index| index,
                    |_, _, _| div().h(24.),
                )
                .size(200., 300.),
            )
            .child(
                measured_rows(
                    measured.clone(),
                    heights,
                    1,
                    || (0..50_u32).collect::<Vec<_>>(),
                    |_: &[u32], _| 30.,
                    |_, _, _| div().h(30.),
                )
                .size(200., 300.),
            )
            .child(div().size(200., 300.)),
    );
    ui.prepare_frame();
    assert_eq!(
        ui.wheel_line_at(100., 100.),
        Some(24.),
        "fixed rows: their height"
    );
    assert_eq!(
        ui.wheel_line_at(300., 100.),
        Some(30.),
        "measured: the estimate"
    );
    assert_eq!(
        ui.wheel_line_at(500., 100.),
        None,
        "no list: the platform's"
    );
    assert_eq!(ui.wheel_line_at(100., 350.), None, "below it");
    fixed.unmount();
    ui.prepare_frame();
    assert_eq!(ui.wheel_line_at(100., 100.), None, "gone with the list");
}
