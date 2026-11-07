//! A driven shader's clock: it runs on the frame clock while its node shows and
//! it's asked to, and otherwise asks for nothing, so a window with one off
//! screen or stopped goes idle.
use std::cell::RefCell;
use std::rc::Rc;
use std::task::{Context as TaskContext, Poll, Waker};
use zgx::compose::prelude::*;
use zgx::compose::{TaskRunner, TaskToken};
use zgx::decoration::FillShader;
use zgx::frame::{Frame, FrameClock};
use zgx::shader_drive::ShaderDrive;
use zgx::widgets::Ui;

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
fn drive(running: bool) -> ShaderDrive {
    ShaderDrive {
        shader: Some(FillShader::new(
            "fn layer(input: LayerInput) -> vec4<f32> { return input.color; }",
            None,
        )),
        running,
        ..Default::default()
    }
}
/// The one node under the root.
fn node(ui: &Ui) -> zgx::scene::NodeId {
    let scene = ui.scene.borrow();
    scene.children(scene.root())[0]
}

#[test]
fn running_it_asks_for_frames_only_while_it_shows() {
    let moving = drive(true);
    let (mut ui, clock, tasks) = harness(
        div()
            .absolute()
            .left(10.)
            .top(10.)
            .w(100.)
            .h(100.)
            .shader_drive(move || moving.clone()),
    );
    settle(&mut ui, &tasks);
    assert!(clock.wants_frame(), "shown, it steps");
    std::thread::sleep(std::time::Duration::from_millis(2));
    frame(&clock, &tasks);
    let time = ui
        .scene
        .borrow()
        .layer_shader(node(&ui))
        .unwrap()
        .uniforms
        .time;
    assert!(clock.wants_frame(), "and goes on");
    std::thread::sleep(std::time::Duration::from_millis(2));
    frame(&clock, &tasks);
    let later = ui
        .scene
        .borrow()
        .layer_shader(node(&ui))
        .unwrap()
        .uniforms
        .time;
    assert!(later > time, "{later} after {time}");
    ui.set_presented(false);
    settle(&mut ui, &tasks);
    assert!(!clock.wants_frame(), "its window hidden, nothing wakes");
}

#[test]
fn off_screen_it_asks_for_no_frames() {
    let moving = drive(true);
    let (mut ui, clock, tasks) = harness(
        div()
            .absolute()
            .left(4000.)
            .top(10.)
            .w(100.)
            .h(100.)
            .shader_drive(move || moving.clone()),
    );
    settle(&mut ui, &tasks);
    assert!(!clock.wants_frame());
}

#[test]
fn still_or_with_no_shader_it_asks_for_no_frames_and_makes_no_layer() {
    let still = drive(false);
    let (mut ui, clock, tasks) = harness(div().w(100.).h(100.).shader_drive(move || still.clone()));
    settle(&mut ui, &tasks);
    assert!(!clock.wants_frame());
    assert!(ui.scene.borrow().is_layer(node(&ui)));
    let (mut ui, clock, tasks) = harness(div().w(100.).h(100.).shader_drive(|| ShaderDrive {
        running: true,
        ..Default::default()
    }));
    settle(&mut ui, &tasks);
    assert!(!clock.wants_frame());
    assert!(!ui.scene.borrow().is_layer(node(&ui)));
}

#[test]
fn it_follows_what_it_reads() {
    let mut ui = Ui::new(400., 300.);
    let on = ui.signal(false);
    let read = on.clone();
    let shown = drive(true);
    let tasks: Tasks = Rc::default();
    let spawned = tasks.clone();
    let runner = TaskRunner::new(move |future| {
        let cell = Rc::new(RefCell::new(future));
        spawned.borrow_mut().push(cell.clone());
        TaskToken::new(cell, |_| true)
    });
    let clock = FrameClock::new();
    ui.mount(provide(
        runner,
        provide(
            clock.clone(),
            div()
                .w(100.)
                .h(100.)
                .shader_drive(move || match read.get() {
                    true => shown.clone(),
                    false => ShaderDrive::default(),
                }),
        ),
    ));
    settle(&mut ui, &tasks);
    assert!(!clock.wants_frame());
    on.set(true);
    settle(&mut ui, &tasks);
    assert!(clock.wants_frame());
    assert!(ui.scene.borrow().is_layer(node(&ui)));
    on.set(false);
    settle(&mut ui, &tasks);
    assert!(!clock.wants_frame());
    assert!(!ui.scene.borrow().is_layer(node(&ui)));
}
