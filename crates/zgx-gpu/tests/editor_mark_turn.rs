//! A text area's icon mark turned (`LineMark::rotate`, a toggle's chevron
//! open) draws as the same icon turned by hand: its node's transform, about
//! its centre.
use std::{f32::consts::FRAC_PI_2, rc::Rc, sync::Arc};
use zgx::{
    compose::prelude::*,
    scene::{Color, LocalTransform, NodeId, NodeKind},
    svg::SvgData,
    widgets::{LineMark, Ui},
};
use zgx_gpu::GpuRenderer;

/// A chevron pointing right: turned 90°, it points down.
fn chevron() -> Arc<SvgData> {
    Arc::new(
        SvgData::new(
            br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M5 2 L12 8 L5 14 Z" fill="#000"/></svg>"##
                .to_vec(),
        )
        .unwrap(),
    )
}

/// A text area with a chevron mark on its first line, turned `rotate`
/// degrees; `by_hand` then turns the mark's node 90° itself. Its pixels.
fn frame(rotate: f32, by_hand: bool) -> Vec<u8> {
    let mut gpu = GpuRenderer::new(160, 80).unwrap();
    let mut ui = Ui::new(160., 80.);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let value = ui.signal("\n".to_string());
    let view = ui.mount(
        div().size(160., 80.).bg(rgb(0xffffff)).child(
            text_area("Note", value)
                .id("note")
                .w(140.)
                .h(60.)
                .p(0.)
                .text_size(14.)
                .line_height(24.),
        ),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.find("note").unwrap()).unwrap();
    let svg = chevron();
    editor.set_line_marks(Some(Rc::new(move |i, _, _| match i {
        0 => vec![
            LineMark::new(20., 4., 16., 16., Color(0, 0, 0, 0))
                .svg(svg.clone())
                .rotate(rotate),
        ],
        _ => Vec::new(),
    })));
    ui.prepare_frame();
    if by_hand {
        let id = svg_node(&ui, editor.node);
        ui.scene.borrow_mut().set_local_transform(
            id,
            LocalTransform {
                rotate: FRAC_PI_2,
                ..LocalTransform::IDENTITY
            },
        );
    }
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    gpu.render(&ui.scene.borrow(), &damage).unwrap();
    gpu.readback().unwrap()
}

fn svg_node(ui: &Ui, root: NodeId) -> NodeId {
    let scene = ui.scene.borrow();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        if matches!(scene.kind(id), NodeKind::Svg(_)) {
            return id;
        }
        pending.extend_from_slice(scene.children(id));
    }
    panic!("no icon mark");
}

#[test]
fn a_turned_icon_mark_draws_as_the_icon_turned_by_hand() {
    let turned = frame(90., false);
    let by_hand = frame(0., true);
    let unturned = frame(0., false);
    assert_eq!(turned.len(), by_hand.len());
    let off = turned
        .iter()
        .zip(&by_hand)
        .filter(|(a, b)| a.abs_diff(**b) > 2)
        .count();
    assert_eq!(off, 0, "{off} channels differ from the icon turned by hand");
    let moved = turned
        .iter()
        .zip(&unturned)
        .filter(|(a, b)| a.abs_diff(**b) > 2)
        .count();
    assert!(moved > 20, "turned, it draws otherwise: {moved} channels");
}
