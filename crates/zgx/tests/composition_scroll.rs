use std::{cell::Cell, rc::Rc};
use zgx::{compose::prelude::*, input::InputEvent, semantics::Role, widgets::Ui};
fn wheel(ui: &mut Ui, x: f32, y: f32, delta_y: f32) {
    ui.dispatch(InputEvent::Scroll {
        x,
        y,
        delta_x: 0.,
        delta_y,
    });
}
#[test]
fn ordinary_scroll_measures_dynamic_children_and_reclamps_on_resize() {
    let mut ui = Ui::new(600., 400.);
    let offset = ui.signal(0.);
    let expanded = ui.signal(false);
    let read_expanded = expanded.clone();
    let height = ui.signal(120.);
    let read_height = height.clone();
    let view = ui.mount(
        scroll(offset.clone())
            .w(220.)
            .p(10.)
            .reactive_style(move || Styles::new().h(read_height.get()))
            .child(div().h(150.))
            .child(switch(
                move || read_expanded.get(),
                |expanded, _| {
                    if expanded {
                        div().h(250.)
                    } else {
                        div().h(50.)
                    }
                },
            )),
    );
    ui.prepare_frame();
    offset.set(10000.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 100.);
    expanded.set(true);
    ui.prepare_frame();
    offset.set(10000.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 300.);
    height.set(220.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 200.);
    expanded.set(false);
    ui.prepare_frame();
    assert_eq!(offset.get(), 0.);
    assert_eq!(
        ui.semantics.borrow().get(view.node()).unwrap().role,
        Role::ScrollView
    );
}
#[test]
fn nested_scroll_consumes_motion_until_edge_then_bubbles() {
    let mut ui = Ui::new(500., 400.);
    let outer = ui.signal(0.);
    let inner = ui.signal(0.);
    ui.mount(
        scroll(outer.clone())
            .w(300.)
            .h(160.)
            .child(
                scroll(inner.clone())
                    .w(200.)
                    .h(100.)
                    .child(div().w(200.).h(300.)),
            )
            .child(div().h(400.)),
    );
    ui.prepare_frame();
    wheel(&mut ui, 20., 20., 100.);
    assert_eq!((inner.get(), outer.get()), (100., 0.));
    wheel(&mut ui, 20., 20., 100.);
    assert_eq!((inner.get(), outer.get()), (200., 0.));
    // Each a gesture of its own: one latches to the view it scrolls.
    let pause = || std::thread::sleep(std::time::Duration::from_millis(350));
    pause();
    wheel(&mut ui, 20., 20., 40.);
    assert_eq!((inner.get(), outer.get()), (200., 40.));
    pause();
    wheel(&mut ui, 20., 20., -50.);
    assert_eq!((inner.get(), outer.get()), (150., 40.));
}
#[test]
fn offset_updates_are_compositor_only_and_children_retain_local_state() {
    let mut ui = Ui::new(500., 300.);
    let offset = ui.signal(0.);
    let mounts = Rc::new(Cell::new(0));
    let observed = mounts.clone();
    let view = ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .child(component(move |cx| {
                observed.set(observed.get() + 1);
                let count = cx.state(42);
                column()
                    .h(500.)
                    .child(text_signal(move || count.get().to_string()).id("local"))
            })),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let child = view.find("local").unwrap();
    for n in [25., 100., 350., 0.] {
        offset.set(n);
        ui.prepare_frame();
        let frame = ui.scene.borrow_mut().flush();
        assert_eq!(frame.layout_nodes, 0);
        assert_eq!(view.find("local"), Some(child));
    }
    assert_eq!(mounts.get(), 1);
}
#[test]
fn disabled_scroll_blocks_input_and_unmount_releases_observers() {
    let mut ui = Ui::new(500., 300.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .disabled(true)
            .child(div().h(500.)),
    );
    ui.prepare_frame();
    assert!(ui.semantics.borrow().get(view.node()).unwrap().disabled);
    wheel(&mut ui, 20., 20., 50.);
    assert_eq!(offset.get(), 0.);
    offset.set(f32::INFINITY);
    ui.prepare_frame();
    assert_eq!(offset.get(), 400.);
    offset.set(f32::NAN);
    assert_eq!(offset.get(), 0.);
    view.unmount();
    assert!(!ui.input.has_listeners(view.node()));
    assert!(ui.semantics.borrow().get(view.node()).is_none());
    offset.set(900.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 900.);
}
#[test]
fn fractional_scrolling_moves_content_by_whole_device_pixels() {
    // A trackpad's deltas are fractions of a point: the offset keeps them,
    // while what shows moves by whole device pixels (as Safari's does), so
    // edges and hairlines never straddle pixels and the renderer can move
    // what it drew by copying it.
    let mut ui = Ui::new(500., 300.);
    ui.scene.borrow_mut().set_pixel_scale(2.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .child(div().id("row").h(500.)),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let row = view.find("row").unwrap();
    let top = ui.scene.borrow().bounds(row).y;
    let mut moved = 0.;
    for delta in [0.3, 1.45, 2.8, 0.15, 3.3] {
        wheel(&mut ui, 20., 20., delta);
        moved += delta;
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        assert!((offset.get() - moved).abs() < 1e-4, "{}", offset.get());
        let shown = top - ui.scene.borrow().bounds(row).y;
        assert_eq!(shown, (moved * 2_f32).round() / 2., "after {moved}");
        let moves = ui.scene.borrow().scroll_moves().to_vec();
        for m in moves {
            assert_eq!((m.dy * 2.).fract(), 0., "{m:?}");
        }
        assert!(report.layout_nodes == 0);
    }
    // At another scale, what shows moves to its grid.
    ui.scene.borrow_mut().set_pixel_scale(1.);
    ui.prepare_frame();
    assert_eq!(top - ui.scene.borrow().bounds(row).y, moved.round());
}

/// How far a row moved in a scroll view shown at `start`, scrolled to `raw`,
/// then shown at `end`.
fn shown_after_scale_change(start: f32, end: f32, raw: f32) -> f32 {
    let mut ui = Ui::new(500., 300.);
    ui.scene.borrow_mut().set_pixel_scale(start);
    let offset = ui.signal(0.);
    let view = ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .child(div().id("row").h(500.)),
    );
    ui.prepare_frame();
    let row = view.find("row").unwrap();
    let top = ui.scene.borrow().bounds(row).y;
    offset.set(raw);
    ui.prepare_frame();
    ui.scene.borrow_mut().set_pixel_scale(end);
    ui.prepare_frame();
    assert_eq!(offset.get(), raw);
    top - ui.scene.borrow().bounds(row).y
}

#[test]
fn dpi_change_uses_the_exact_retained_scroll_offset() {
    let live = shown_after_scale_change(1., 2., 0.4);
    let fresh = shown_after_scale_change(2., 2., 0.4);
    assert_eq!(fresh, 0.5);
    assert_eq!(live, fresh);
}

#[test]
fn fractional_dpi_change_uses_the_exact_retained_scroll_offset() {
    let live = shown_after_scale_change(2., 1.5, 0.3);
    let fresh = shown_after_scale_change(1.5, 1.5, 0.3);
    assert_eq!(fresh, 0.);
    assert_eq!(live, fresh);
}
/// Scroll content that repaints rather than copies when it moves (a layer of
/// its own: a fade, an isolated group) damages what went or moved in it
/// after the move where it was painted, before the move: a node gone (a
/// hovered row's lane under a still pointer) or a box laid out again.
#[test]
fn a_moved_layer_repaints_what_changed_after_the_move_where_it_was() {
    use zgx::scene::Rect;
    let mut ui = Ui::new(300., 300.);
    let offset = ui.signal(0.);
    let shown = ui.signal(true);
    let read_shown = shown.clone();
    let wide = ui.signal(false);
    let read_wide = wide.clone();
    let view = ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(200.)
            .child(div().h(100.))
            .child(switch(
                move || read_shown.get(),
                |shown, _| match shown {
                    true => div().w(60.).h(16.).bg(rgb(0xff2020)),
                    false => div().h(16.),
                },
            ))
            .child(
                div()
                    .h(20.)
                    .reactive_style(move || {
                        Styles::new().w(if read_wide.get() { 120. } else { 40. })
                    })
                    .bg(rgb(0x2020ff)),
            )
            .child(div().h(600.)),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    ui.scene.borrow_mut().set_isolated(view.node(), true);
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    // Moved, then (a frame's later work) the lane goes and the bar narrows
    // from where it was laid out.
    wide.set(true);
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    offset.set(30.);
    ui.prepare_frame();
    shown.set(false);
    wide.set(false);
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    let covered = |painted: Rect| {
        // Every point of it, to the pixel.
        (0..painted.height as i32).all(|y| {
            (0..painted.width as i32).all(|x| {
                let pixel = Rect::new(painted.x + x as f32, painted.y + y as f32, 1., 1.);
                damage.iter().any(|rect| rect.intersects(pixel))
            })
        })
    };
    assert!(
        covered(Rect::new(0., 100., 60., 16.)),
        "the lane: {damage:?}"
    );
    assert!(
        covered(Rect::new(0., 116., 120., 20.)),
        "the bar: {damage:?}"
    );
}

/// A view following its end, its content `height` tall, 100 tall itself.
fn following(ui: &mut Ui, height: f32) -> (zgx::reactive::Signal<f32>, zgx::reactive::Signal<f32>) {
    let offset = ui.signal(0.);
    let extent = ui.signal(height);
    let read = extent.clone();
    ui.mount(
        scroll(offset.clone())
            .follow_end(true)
            .w(200.)
            .h(100.)
            .child(div().reactive_style(move || Styles::new().h(read.get()))),
    );
    ui.prepare_frame();
    (offset, extent)
}

#[test]
fn a_followed_view_left_by_the_wheel_stays_where_it_was_left_as_its_content_grows_and_shrinks() {
    let mut ui = Ui::new(600., 400.);
    let (offset, extent) = following(&mut ui, 500.);
    assert_eq!(offset.get(), 400., "it starts at its end");
    extent.set(600.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 500., "at its end, growth follows");

    wheel(&mut ui, 10., 10., -40.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 460.);
    extent.set(700.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 460., "left, growth doesn't move it");

    // A fold closing under the reader: the end comes up to them and they
    // are put at it, but they didn't ask to follow.
    extent.set(520.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 420., "shrinking past it clamps");
    extent.set(900.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 420., "a clamp doesn't start following again");

    // A move of theirs to the end does.
    wheel(&mut ui, 10., 10., 10000.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 800.);
    extent.set(1000.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 900., "the wheel at the end follows again");
}

#[test]
fn a_wheel_at_a_clamped_end_follows_again_without_moving() {
    let mut ui = Ui::new(600., 400.);
    let (offset, extent) = following(&mut ui, 500.);
    wheel(&mut ui, 10., 10., -40.);
    ui.prepare_frame();
    extent.set(400.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 300., "clamped at the new end");
    // Nowhere further to go: a notch down moves nothing, and still says to
    // stay at the end.
    wheel(&mut ui, 10., 10., 40.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 300.);
    extent.set(600.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 500., "a notch down at the end follows again");
}

#[test]
fn a_script_sending_a_followed_view_past_its_end_makes_it_follow() {
    let mut ui = Ui::new(600., 400.);
    let (offset, extent) = following(&mut ui, 500.);
    wheel(&mut ui, 10., 10., -200.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 200.);
    offset.set(f32::INFINITY);
    extent.set(900.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 800., "at the end of what's there now");
    extent.set(1000.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 900., "and following");
}
