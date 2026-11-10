use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use zgx::{
    canvas::{CanvasText, TextAnchor, TextBaseline},
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

fn frame(ui: &mut Ui) -> zgx::scene::FrameReport {
    ui.prepare_frame();
    ui.scene.borrow_mut().flush()
}

/// A canvas's text whose maker gives its size (`CanvasText::extent`, a
/// terminal's run) is laid out by it and never measured, however often it
/// changes; one without is measured each time, as before, and sized by it.
#[test]
fn a_sized_canvas_text_is_never_measured() {
    let mut ui = Ui::new(320., 200.);
    let measured = Rc::new(RefCell::new(Vec::<String>::new()));
    let seen = measured.clone();
    ui.scene
        .borrow_mut()
        .set_font_text_measurer(move |text, size, _, _| {
            seen.borrow_mut().push(text.to_string());
            (text.chars().count() as f32 * size * 0.5, size)
        });
    let n = ui.signal(0);
    let text = |text: String, y: f32, extent| CanvasText {
        text: text.into(),
        x: 0.,
        y,
        size: 10.,
        color: rgb(0xffffff),
        font: Default::default(),
        anchor: TextAnchor::Start,
        baseline: TextBaseline::Top,
        extent,
    };
    let view = ui.mount(
        canvas({
            let n = n.clone();
            move |_| {
                let n = n.get();
                let mut canvas = Canvas::new();
                canvas.text(text(format!("sized {n}"), 0., Some((120., 14.))));
                canvas.text(text(format!("plain {n}"), 20., None));
                canvas
            }
        })
        .id("canvas")
        .w(300.)
        .h(80.),
    );
    frame(&mut ui);
    for step in 1..5 {
        n.set(step);
        frame(&mut ui);
    }
    let measured = measured.borrow();
    assert!(
        !measured.iter().any(|text| text.starts_with("sized")),
        "{measured:?}"
    );
    for step in 0..5 {
        assert!(measured.contains(&format!("plain {step}")), "{measured:?}");
    }
    // Laid out at the size given, and the other at the size measured.
    let scene = ui.scene.borrow();
    let label = |prefix: &str| {
        scene
            .paint_items()
            .find(|item| matches!(scene.kind(item.id), NodeKind::Text { text, .. } if text.starts_with(prefix)))
            .map(|item| scene.bounds(item.id))
            .unwrap()
    };
    let (sized, plain) = (label("sized"), label("plain"));
    assert_eq!((sized.width, sized.height), (120., 14.));
    assert_eq!((plain.width, plain.height), (35., 10.));
    drop(scene);
    view.unmount();
}

/// A canvas's sized texts changing together (a terminal row's runs: text,
/// colour and font, wider than the canvas, some coming and going) damage
/// where each was and is, ink and all, as one, and measure nothing. A text
/// without a size beside them damages as it changes, as ever.
#[test]
fn sized_canvas_texts_changing_together_are_damaged_as_one() {
    let mut ui = Ui::new(400., 200.);
    let measured = Rc::new(RefCell::new(Vec::<String>::new()));
    let seen = measured.clone();
    ui.scene
        .borrow_mut()
        .set_font_text_measurer(move |text, size, _, _| {
            seen.borrow_mut().push(text.to_string());
            (text.chars().count() as f32 * size * 0.5, size)
        });
    let n = ui.signal(0);
    let text = |text: String, x: f32, y: f32, step: i32, extent| CanvasText {
        text: text.into(),
        x,
        y,
        size: 10.,
        color: rgb(0x10_20_30 * (step as u32 % 4 + 1)),
        font: zgx::text_layout::FontStyle {
            weight: if step % 2 == 0 { 400 } else { 700 },
            ..Default::default()
        },
        anchor: TextAnchor::Start,
        baseline: TextBaseline::Top,
        extent,
    };
    let view = ui.mount(
        canvas({
            let n = n.clone();
            move |_| {
                let n = n.get();
                let mut canvas = Canvas::new();
                canvas.text(text(format!("plain {n}"), 0., 30., n, None));
                // Each run as wide as the row, from where it starts: past
                // the canvas's right edge, as a terminal sizes them.
                let runs = 2 + n % 2;
                for run in 0..runs {
                    let x = (run * 40 + n % 3 * 8) as f32;
                    canvas.text(text(
                        format!("sized {n} {run}"),
                        x,
                        0.,
                        n + run,
                        Some((200., 14.)),
                    ));
                }
                canvas
            }
        })
        .absolute()
        .left(30.)
        .top(40.)
        .w(150.)
        .h(60.),
    );
    frame(&mut ui);
    for step in 1..7 {
        n.set(step);
        let report = frame(&mut ui);
        let damaged = |x: f32, y: f32| {
            let point = Rect::new(x, y, 0.5, 0.5);
            report.damage.iter().any(|rect| rect.intersects(point))
        };
        // Each run's ink: its box, from where it starts to 200 past, padded
        // 3 (`text_ink` at size 10), where it was and where it is.
        let left = |n: i32| 30. + (n % 3 * 8) as f32 - 3.;
        let right = |n: i32| 30. + ((1 + n % 2) * 40 + n % 3 * 8) as f32 + 203.;
        let (left, right) = (
            left(step - 1).min(left(step)),
            right(step - 1).max(right(step)),
        );
        for (x, y) in [
            (left + 0.25, 37.25),
            (left + 0.25, 56.25),
            (right - 1., 37.25),
            (right - 1., 56.25),
        ] {
            assert!(
                damaged(x, y),
                "step {step}: ({x}, {y}) not in {:?}",
                report.damage
            );
        }
        // Past their ink, nothing.
        for (x, y) in [(right + 2., 45.), (left.min(27.) - 3., 45.), (100., 34.)] {
            assert!(
                !damaged(x, y),
                "step {step}: ({x}, {y}) in {:?}",
                report.damage
            );
        }
    }
    let measured = measured.borrow();
    assert!(
        !measured.iter().any(|text| text.starts_with("sized")),
        "{measured:?}"
    );
    for step in 0..7 {
        assert!(measured.contains(&format!("plain {step}")), "{measured:?}");
    }
    drop(measured);
    view.unmount();
}
