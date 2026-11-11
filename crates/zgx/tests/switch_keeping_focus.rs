//! A switch that builds its branch again keeps focus where it was in it
//! (`switch_keeping_focus`); held (`switch_keeping_focus_held`), where it
//! was as the hold was taken, wherever focus moved before the switch.
use zgx::{
    compose::{FocusHold, ViewHandle, prelude::*, switch_keeping_focus_held},
    widgets::Ui,
};

fn focused(ui: &Ui, view: &ViewHandle) -> Option<&'static str> {
    let id = ui.input.focused()?;
    ["a", "b"]
        .into_iter()
        .find(|name| view.find(name) == Some(id))
}

#[test]
fn a_held_switch_keeps_focus_where_it_was_as_the_hold_was_taken() {
    let mut ui = Ui::new(400., 300.);
    let generation = ui.signal(0_u32);
    let hold = FocusHold::default();
    let select = generation.clone();
    let view = ui.mount(switch_keeping_focus_held(
        hold.clone(),
        move || select.get(),
        |_, _| {
            column()
                .child(button().id("a").child(text("A")))
                .child(button().id("b").child(text("B")))
        },
    ));
    ui.prepare_frame();
    let switch = |ui: &mut Ui| {
        generation.update(|g| *g += 1);
        ui.prepare_frame();
        ui.prepare_frame();
    };
    ui.input.focus(&ui.scene, view.find("a"));
    hold.take();
    // Moved before the switch (as a script's load can move it).
    ui.input.focus(&ui.scene, view.find("b"));
    switch(&mut ui);
    assert_eq!(focused(&ui, &view), Some("a"), "where it was as taken");
    // Not taken: where it is as it switches.
    ui.input.focus(&ui.scene, view.find("b"));
    switch(&mut ui);
    assert_eq!(focused(&ui, &view), Some("b"), "as it switches");
    // Taken once only, and let go: as it switches.
    ui.input.focus(&ui.scene, view.find("a"));
    hold.take();
    hold.release();
    ui.input.focus(&ui.scene, view.find("b"));
    switch(&mut ui);
    assert_eq!(focused(&ui, &view), Some("b"), "released");
}
