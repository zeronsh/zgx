//! A view that snaps (`View::pixel_snap`) rests on whole device pixels,
//! whichever writer moves it: its style (static or reactive), a drag, a new
//! pixel scale. A view that doesn't keeps its exact offset.
use zgx::{
    compose::prelude::*,
    input::{InputEvent, PointerButton},
    reactive::Signal,
    style::Styles,
    widgets::Ui,
};

fn at_2x() -> Ui {
    let ui = Ui::new(400., 200.);
    ui.scene.borrow_mut().set_pixel_scale(2.);
    ui
}
/// The mounted column's first child's translation, as the frame paints it
/// (the scene's flush, as a renderer's, puts snapping nodes on pixels).
fn first_child(ui: &Ui, root: zgx::scene::NodeId) -> (f32, f32) {
    ui.scene.borrow_mut().flush();
    let scene = ui.scene.borrow();
    let child = scene.children(root)[0];
    let t = scene.transform(child);
    (t.x, t.y)
}

#[test]
fn a_snapping_views_static_translate_rests_on_a_device_pixel() {
    let mut ui = at_2x();
    let snapping = ui.mount(column().child(div().size(20., 20.).translate(10.3, 4.2).pixel_snap()));
    let exact = ui.mount(column().child(div().size(20., 20.).translate(10.3, 4.2)));
    ui.prepare_frame();
    assert_eq!(first_child(&ui, snapping.node()), (10.5, 4.));
    assert_eq!(
        first_child(&ui, exact.node()),
        (10.3, 4.2),
        "a view that doesn't snap is exact"
    );
}

#[test]
fn a_snapping_views_reactive_translate_rests_on_a_device_pixel() {
    let mut ui = at_2x();
    let x: Signal<f32> = ui.signal(7.3);
    let read = x.clone();
    let view = ui.mount(
        column().child(
            div()
                .size(20., 20.)
                .reactive_style(move || Styles::new().translate(read.get(), 0.))
                .pixel_snap(),
        ),
    );
    ui.prepare_frame();
    assert_eq!(first_child(&ui, view.node()), (7.5, 0.));
    x.set(7.2);
    ui.prepare_frame();
    assert_eq!(first_child(&ui, view.node()), (7., 0.));
}

#[test]
fn a_new_pixel_scale_puts_a_snapping_view_on_its_grid() {
    let mut ui = at_2x();
    let view = ui.mount(column().child(div().size(20., 20.).translate(10.3, 0.).pixel_snap()));
    ui.prepare_frame();
    assert_eq!(first_child(&ui, view.node()), (10.5, 0.));
    ui.scene.borrow_mut().set_pixel_scale(4.);
    ui.prepare_frame();
    let (x, _) = first_child(&ui, view.node());
    assert_eq!((x * 4.).fract(), 0., "on the 4x grid: {x}");
}

#[derive(Clone)]
struct Card;
fn pointer(ui: &Ui, event: InputEvent) {
    ui.input.dispatch(&ui.scene, event);
    ui.prepare_frame();
}

#[test]
fn a_snapping_drag_preview_follows_on_device_pixels() {
    let mut ui = at_2x();
    ui.mount(
        div()
            .size(100., 80.)
            .on_drag(|| Card)
            .drag_preview(|_: &Card| div().size(20., 20.).pixel_snap()),
    );
    ui.prepare_frame();
    let before = ui.scene.borrow().children(ui.root()).len();
    let button = PointerButton::Primary;
    pointer(
        &ui,
        InputEvent::PointerDown {
            x: 20.,
            y: 20.,
            button,
        },
    );
    pointer(&ui, InputEvent::PointerMove { x: 60.3, y: 30.2 });
    pointer(&ui, InputEvent::PointerMove { x: 80.3, y: 40.1 });
    assert!(ui.input.is_dragging());
    ui.scene.borrow_mut().flush();
    let scene = ui.scene.borrow();
    let preview = *scene.children(ui.root()).last().unwrap();
    assert_eq!(
        scene.children(ui.root()).len(),
        before + 1,
        "the preview is up"
    );
    let t = scene.transform(preview);
    assert_eq!(
        ((t.x * 2.).fract(), (t.y * 2.).fract()),
        (0., 0.),
        "on device pixels: {t:?}"
    );
}

/// Two scenes at 2x, each with one box: `snaps` marks the first's box
/// (`Scene::set_pixel_snap`); the second is a control its writer rounds itself.
fn boxes() -> (zgx::scene::Scene, zgx::scene::NodeId) {
    use zgx::scene::{Layout, NodeKind, Scene, Style};
    let mut scene = Scene::new(200., 100.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let node = scene.append(
        scene.root(),
        NodeKind::Rect(zgx::scene::Color(200, 40, 40, 255)),
        Style {
            width: Some(20.),
            height: Some(20.),
            absolute: true,
            ..Default::default()
        },
    );
    scene.set_pixel_scale(2.);
    scene.flush();
    (scene, node)
}
fn at(x: f32) -> zgx::scene::Transform {
    zgx::scene::Transform { x, y: 0. }
}

/// A write after the frame's layout (a handler's, a layout callback's) is on
/// device pixels when the frame paints: the scene's pass runs as it flushes,
/// after every write and before layout and paint.
#[test]
fn a_write_after_layout_still_paints_on_device_pixels() {
    let (mut scene, node) = boxes();
    scene.set_pixel_snap(node, true);
    scene.set_transform(node, at(10.3));
    scene.prepare_layout();
    scene.set_transform(node, at(12.3));
    scene.flush();
    assert_eq!(scene.transform(node), at(12.5), "painted on device pixels");
}

/// What a snapping box's frames damage, against a box its writer rounds
/// itself (both 20 points wide at 2x), as it is: a frame where it moves may
/// damage up to a point more (its writer's fractional place, damaged before
/// the frame puts it on pixels); a writer setting the same fractional place
/// again re-damages it (the scene holds the rounded one); with nothing
/// written, nothing. A change that damages more fails here.
#[test]
fn what_snapping_damages_against_one_rounded_write() {
    use zgx::scene::Rect;
    let (mut snapping, a) = boxes();
    let (mut control, b) = boxes();
    snapping.set_pixel_snap(a, true);
    let rect = |x: f32, width: f32| vec![Rect::new(x, 0., width, 20.)];
    let frames: [(f32, Vec<Rect>, Vec<Rect>); 5] = [
        (3.3, rect(0., 24.), rect(0., 24.)),
        (7.1, rect(3., 25.), rect(3., 24.)),
        (10.3, rect(7., 24.), rect(7., 24.)),
        (10.3, rect(10., 21.), vec![]),
        (10.3, rect(10., 21.), vec![]),
    ];
    for (x, snapped, rounded) in frames {
        snapping.set_transform(a, at(x));
        control.set_transform(b, at((x * 2.).round() / 2.));
        assert_eq!(snapping.flush().damage, snapped, "snapping at {x}");
        assert_eq!(
            control.flush().damage,
            rounded,
            "rounded by its writer at {x}"
        );
        assert_eq!(
            snapping.transform(a),
            control.transform(b),
            "shown in the same place"
        );
    }
    for _ in 0..3 {
        assert!(
            snapping.flush().damage.is_empty(),
            "nothing written: nothing damaged"
        );
    }
}

/// A snapping view's reactive style whose inputs don't change doesn't run
/// again, so a box at rest is damaged no more, frame after frame.
#[test]
fn a_still_reactive_snapping_view_damages_nothing() {
    let mut ui = at_2x();
    let x: Signal<f32> = ui.signal(7.3);
    let read = x.clone();
    let other = ui.signal(0_u32);
    let shown = other.clone();
    let view = ui.mount(
        column()
            .child(
                div()
                    .size(20., 20.)
                    .reactive_style(move || Styles::new().translate(read.get(), 0.))
                    .pixel_snap(),
            )
            .child(text_signal(move || format!("{}", shown.get()))),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    assert_eq!(first_child(&ui, view.node()), (7.5, 0.));
    for _ in 0..3 {
        ui.prepare_frame();
        assert!(
            ui.scene.borrow_mut().flush().damage.is_empty(),
            "at rest: nothing damaged"
        );
    }
    // Something else changes: the box is left alone.
    other.set(1);
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    let child = ui.scene.borrow().children(view.node())[0];
    let shown_at = ui.scene.borrow().bounds(child);
    assert!(
        damage.iter().all(|r| !r.intersects(shown_at)),
        "the box isn't damaged by another view's change: {damage:?} vs {shown_at:?}"
    );
    assert_eq!(first_child(&ui, view.node()), (7.5, 0.));
}

/// Between a write and the frame's flush, a read of the box's translation is
/// what its writer set (fractional), layout and hit tests too; from the flush
/// on, where it shows. A hit test in that window may disagree with the paint
/// by under a device pixel.
#[test]
fn a_read_before_the_frame_sees_what_the_writer_set() {
    let (mut scene, node) = boxes();
    scene.set_pixel_snap(node, true);
    scene.set_transform(node, at(10.3));
    assert_eq!(scene.transform(node), at(10.3));
    scene.prepare_layout();
    assert_eq!(scene.transform(node), at(10.3), "laid out, not yet painted");
    scene.flush();
    assert_eq!(scene.transform(node), at(10.5));
}
