//! A ready-made effect's clock: it runs on the frame clock while its node
//! shows and it moves, and otherwise asks for nothing, so a window with one
//! off screen, finished or at no intensity goes idle.
use std::cell::RefCell;
use std::rc::Rc;
use std::task::{Context as TaskContext, Poll, Waker};
use zgx::compose::prelude::*;
use zgx::compose::{TaskRunner, TaskToken};
use zgx::frame::{Frame, FrameClock};
use zgx::widgets::Ui;
use zgx_fx::*;

thread_local! {
    static INDEX: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
type Tasks =
    Rc<RefCell<Vec<Rc<RefCell<std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>>>>>>;
fn harness(view: View) -> (Ui, FrameClock, Tasks) {
    let tasks: Tasks = Rc::default();
    let spawned = tasks.clone();
    let runner = TaskRunner::new(move |future| {
        let cell = Rc::new(RefCell::new(future));
        spawned.borrow_mut().push(cell.clone());
        TaskToken::new(cell, |_| true)
    });
    let clock = FrameClock::new();
    let mut ui = Ui::new(400., 300.);
    ui.mount(provide(runner, provide(clock.clone(), view)));
    (ui, clock, tasks)
}
/// Lay out, tell what shows, and poll the tasks alive (those whose tokens
/// went are dropped).
fn settle(ui: &mut Ui, tasks: &Tasks) {
    ui.prepare_frame();
    let _ = ui.scene.borrow_mut().flush();
    ui.prepare_frame();
    poll(tasks);
}
fn poll(tasks: &Tasks) {
    let mut context = TaskContext::from_waker(Waker::noop());
    let mut list = tasks.borrow_mut();
    list.retain(|task| Rc::strong_count(task) > 1);
    list.retain(|task| match task.borrow_mut().as_mut().poll(&mut context) {
        Poll::Ready(()) => false,
        Poll::Pending => true,
    });
}
fn frame(clock: &FrameClock, tasks: &Tasks) {
    INDEX.with(|index| {
        index.set(index.get() + 1);
        clock.deliver(Frame {
            index: index.get(),
            time: std::time::Instant::now(),
            interval: std::time::Duration::from_millis(16),
        });
    });
    poll(tasks);
}

#[test]
fn a_moving_effect_asks_for_frames_only_while_it_shows() {
    let (mut ui, clock, tasks) = harness(
        div()
            .absolute()
            .left(10.)
            .top(10.)
            .w(100.)
            .h(100.)
            .fx(Pulse::radar()),
    );
    settle(&mut ui, &tasks);
    assert!(clock.wants_frame(), "shown, it steps");
    frame(&clock, &tasks);
    assert!(clock.wants_frame(), "and goes on");
    ui.set_presented(false);
    settle(&mut ui, &tasks);
    assert!(!clock.wants_frame(), "its window hidden, nothing wakes");
}

#[test]
fn off_screen_it_asks_for_no_frames() {
    let (mut ui, clock, tasks) = harness(
        div()
            .absolute()
            .left(4000.)
            .top(10.)
            .w(100.)
            .h(100.)
            .fx(Pulse::radar()),
    );
    settle(&mut ui, &tasks);
    assert!(!clock.wants_frame());
}

#[test]
fn a_still_or_idle_effect_asks_for_no_frames() {
    for fx in [
        Fx::from(Glow::soft()),
        Fx::from(Pulse {
            intensity: 0.,
            ..Default::default()
        }),
        Fx::from(Lens::clear()),
        Fx::from(Ripple::default()),
    ] {
        let (mut ui, clock, tasks) = harness(div().w(100.).h(100.).fx(fx.clone()));
        settle(&mut ui, &tasks);
        assert!(!clock.wants_frame(), "{}", fx.name());
    }
}

#[test]
fn a_one_shot_runs_while_alive_then_stops() {
    let ripple: Fx = Ripple {
        life: 0.01,
        ..Default::default()
    }
    .into();
    let (mut ui, clock, tasks) = harness(div().w(100.).h(100.).fx(ripple.clone()));
    settle(&mut ui, &tasks);
    assert!(!clock.wants_frame());
    ripple.fire(10., 10.);
    settle(&mut ui, &tasks);
    assert!(clock.wants_frame(), "fired, it steps");
    frame(&clock, &tasks);
    std::thread::sleep(std::time::Duration::from_millis(30));
    frame(&clock, &tasks);
    assert!(!ripple.shots().unwrap().alive());
    settle(&mut ui, &tasks);
    assert!(!clock.wants_frame(), "over, nothing wakes");
    assert!(
        ui.scene
            .borrow()
            .layer_shader(zgx_node(&ui))
            .is_none_or(|l| l.shader.is_none())
    );
}

/// The one node under the root.
fn zgx_node(ui: &Ui) -> zgx::scene::NodeId {
    let scene = ui.scene.borrow();
    scene.children(scene.root())[0]
}
