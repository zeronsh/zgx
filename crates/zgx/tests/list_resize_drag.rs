//! A measured list's width changing, as a splitter drag does it: only the rows in view
//! are laid out again each frame (the rows kept out of view are let go and, with spare time
//! offered each frame, none are built until the width has stood still).
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};
use zgx::{compose::prelude::*, widgets::Ui};

fn spare(ui: &Ui) {
    let mut turns = 0;
    while ui.spare_work_wanted() {
        ui.run_spare_work(Instant::now(), Duration::from_secs(60))
            .unwrap();
        turns += 1;
        assert!(turns < 10_000);
    }
}

#[test]
fn a_width_drag_lays_out_the_rows_in_view_and_builds_none_ahead() {
    let mut ui = Ui::new(600., 100.);
    ui.host_spare_time();
    let offset = ui.signal(0.);
    let width = ui.signal(400.);
    let heights = VariableHeights::new(&ui.runtime, 100, 40.);
    let built = Rc::new(Cell::new(0_usize));
    let b = built.clone();
    let w = width.clone();
    ui.mount(
        column()
            .reactive_style(move || Styles::new().w(w.get()))
            .child(
                measured_virtual_list(
                    offset.clone(),
                    heights,
                    0,
                    |i| i,
                    move |_, i, _| {
                        b.set(b.get() + 1);
                        text(format!(
                            "row {i} {}",
                            "lorem ipsum dolor sit amet ".repeat(8)
                        ))
                        .text_wrap(true)
                        .w_full()
                    },
                )
                .w_full()
                .h(100.),
            ),
    );
    ui.prepare_frame();
    offset.set(1.);
    ui.prepare_frame();
    offset.set(0.);
    ui.prepare_frame();
    spare(&ui);
    for i in 1..=30 {
        offset.set(i as f32 * 40.);
        ui.prepare_frame();
        spare(&ui);
    }
    offset.set(600.);
    ui.prepare_frame();
    spare(&ui);
    ui.scene.borrow_mut().flush();
    let before = built.get();
    for step in 1..=8 {
        width.set(400. - 6. * step as f32);
        ui.prepare_frame();
        ui.run_spare_work(Instant::now(), Duration::from_secs(60))
            .unwrap();
        let report = ui.scene.borrow_mut().flush();
        // A hundred rows of eight nodes or so: only the few in view and the
        // one built ahead are laid out, not every row built so far.
        assert!(
            report.layout_nodes <= 30,
            "step {step}: {} nodes",
            report.layout_nodes
        );
        assert!(
            built.get() <= before + step,
            "step {step}: {} built",
            built.get()
        );
    }
}

/// While the width changes no row is built ahead (the rows in view reflow
/// live); once it has stood still for a while, they are.
#[test]
fn rows_ahead_wait_for_the_width_to_stand_still() {
    let clock = zgx::clock::VirtualClock::new();
    let mut ui = Ui::new(600., 100.);
    ui.host_spare_time();
    let offset = ui.signal(0.);
    let width = ui.signal(400.);
    let heights = VariableHeights::new(&ui.runtime, 100, 40.);
    let built = Rc::new(Cell::new(0_usize));
    let b = built.clone();
    let w = width.clone();
    ui.mount(
        column()
            .reactive_style(move || Styles::new().w(w.get()))
            .child(
                measured_virtual_list(
                    offset.clone(),
                    heights,
                    0,
                    |i| i,
                    move |_, i, _| {
                        b.set(b.get() + 1);
                        text(format!(
                            "row {i} {}",
                            "lorem ipsum dolor sit amet ".repeat(8)
                        ))
                        .text_wrap(true)
                        .w_full()
                    },
                )
                .w_full()
                .h(100.),
            ),
    );
    ui.prepare_frame();
    offset.set(1.);
    ui.prepare_frame();
    offset.set(0.);
    ui.prepare_frame();
    later(&clock, Duration::from_secs(1));
    spare_on(&ui, &clock);
    let before = built.get();
    // A drag: the width changes every frame, spare time is offered each time.
    for step in 1..=6 {
        width.set(400. - 6. * step as f32);
        ui.prepare_frame();
        later(&clock, Duration::from_millis(16));
        ui.run_spare_work(Instant::now(), Duration::from_secs(60))
            .unwrap();
        assert_eq!(built.get(), before, "step {step}: a row built mid-drag");
    }
    // Still for a while: they come.
    later(&clock, Duration::from_millis(150));
    spare_on(&ui, &clock);
    assert!(built.get() > before, "nothing built once it stood still");
}

/// `spare` on a virtual clock, which moves 10 ms a turn.
fn spare_on(ui: &Ui, clock: &zgx::clock::VirtualClock) {
    let mut turns = 0;
    while ui.spare_work_wanted() {
        ui.run_spare_work(Instant::now(), Duration::from_secs(60))
            .unwrap();
        later(&clock, Duration::from_millis(10));
        turns += 1;
        assert!(turns < 10_000);
    }
}

/// Move `clock` on by `by`.
fn later(clock: &zgx::clock::VirtualClock, by: Duration) {
    clock.advance(clock.elapsed() + by, || {});
}
