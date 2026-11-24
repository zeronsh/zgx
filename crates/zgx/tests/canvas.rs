use std::{cell::Cell, rc::Rc};
use zgx::{
    canvas::{CanvasText, KnownSize, TextAnchor, TextBaseline},
    compose::prelude::*,
    scene::{NodeKind, Rect},
    widgets::Ui,
};
#[test]
fn canvas_callback_tracks_content_and_size_with_owned_disposal() {
    let mut ui = Ui::new(320., 200.);
    let color = ui.signal(rgb(0xff0000));
    let width = ui.signal(100.);
    let calls = Rc::new(Cell::new(0));
    let view = ui.mount(
        canvas({
            let color = color.clone();
            let calls = calls.clone();
            move |(w, h)| {
                calls.set(calls.get() + 1);
                let mut canvas = Canvas::new();
                canvas.fill(Path::rectangle(Rect::new(0., 0., w, h)), color.get());
                canvas
            }
        })
        .id("canvas")
        .h(80.)
        .reactive_style({
            let width = width.clone();
            move || Styles::new().w(width.get())
        }),
    );
    frame(&mut ui);
    let first = calls.get();
    assert!(first > 0);
    let root = view.find("canvas").unwrap();
    let node = ui.scene.borrow().children(root)[0];
    assert!(matches!(ui.scene.borrow().kind(node), NodeKind::Canvas(_)));
    assert!(frame(&mut ui).is_idle());
    assert_eq!(calls.get(), first);
    color.set(rgb(0x00ff00));
    let report = frame(&mut ui);
    assert_eq!(report.layout_nodes, 0);
    assert_eq!(calls.get(), first + 1);
    width.set(150.);
    frame(&mut ui);
    assert_eq!(ui.scene.borrow().bounds(node).width, 150.);
    let before = calls.get();
    view.unmount();
    color.set(rgb(0x0000ff));
    frame(&mut ui);
    assert_eq!(calls.get(), before);
}

/// A canvas's lines that come and go are laid out in the box that holds them
/// alone (of a fixed size, a layout boundary), and what moves the canvas
/// later finds them as they are: a terminal's rows, beside a sidebar that
/// closes.
#[test]
fn a_canvas_moved_after_its_lines_changed_places_the_lines_it_has() {
    let mut ui = Ui::new(400., 100.);
    let (lines, beside) = (ui.signal(1), ui.signal(100.));
    let view = ui.mount(
        row().w(400.).h(100.).children([
            div().h(100.).shrink_0().reactive_style({
                let beside = beside.clone();
                move || Styles::new().w(beside.get())
            }),
            canvas({
                let lines = lines.clone();
                move |_| {
                    let mut canvas = Canvas::new();
                    for line in 0..lines.get() {
                        canvas.text(CanvasText {
                            text: format!("line {line}").into(),
                            x: 0.,
                            y: line as f32 * 20.,
                            size: 12.,
                            color: rgb(0x000000),
                            font: Default::default(),
                            anchor: TextAnchor::Start,
                            baseline: TextBaseline::Top,
                            known_size: KnownSize::NONE,
                            width: f32::INFINITY,
                        });
                    }
                    canvas
                }
            })
            .id("canvas")
            .w(200.)
            .h(80.),
        ]),
    );
    frame(&mut ui);
    let root = view.find("canvas").unwrap();
    // Where each line's text is, across.
    let placed = |ui: &Ui| {
        let scene = ui.scene.borrow();
        let mut stack = vec![root];
        let mut at = Vec::new();
        while let Some(id) = stack.pop() {
            stack.extend(scene.children(id).iter().copied());
            if matches!(scene.kind(id), NodeKind::Text { .. }) {
                assert!(scene.bounds(id).width > 0., "a line laid out");
                at.push(scene.bounds(id).x);
            }
        }
        at
    };
    assert_eq!(placed(&ui), [100.]);
    // Gone, then moved; come back, and more, then moved again.
    for (count, left) in [(0, 0.), (1, 60.), (3, 100.), (1, 0.), (0, 100.), (2, 40.)] {
        lines.set(count);
        frame(&mut ui);
        assert_eq!(placed(&ui).len(), count as usize, "its lines as drawn");
        beside.set(left);
        frame(&mut ui);
        assert_eq!(
            placed(&ui),
            vec![left; count as usize],
            "its lines as moved"
        );
    }
}

fn frame(ui: &mut Ui) -> zgx::scene::FrameReport {
    ui.prepare_frame();
    ui.scene.borrow_mut().flush()
}
