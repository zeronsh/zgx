//! A 400-line text area whose marks a Rust closure gives (`line_marks`: a
//! wash, a rule and a number beside every line), laid out by its lines (a
//! highlighter), scrolled a line a frame (`marks-rust`): what a draw costs a
//! Rust app's marks, made at each call. Laid out and flushed, undrawn. The
//! text is the bench's own and left uncounted (`gate_marks`).
//! Run: cargo run -p zgx-script --release --example marks_bench
//! (`FRAMES` sets the frames, 600 by default.)
#[path = "support/gate_marks.rs"]
mod gate_marks;
use zgx::{
    compose::prelude::*,
    scene::Color,
    widgets::{LineMark, Ui},
};

fn frame(ui: &Ui) {
    ui.prepare_frame();
    let flushed = ui.scene.borrow_mut().flush();
    if zgx_fingerprint::on() {
        zgx_fingerprint::frame(&ui.scene.borrow(), &flushed);
    }
}

fn main() {
    let frames: usize = std::env::var("FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let text = gate_marks::outside(|| {
        let lines: Vec<String> = (0..400).map(|i| format!("line {i} of the note")).collect();
        lines.join("\n")
    });
    let mut ui = Ui::new(400., 600.);
    let value = ui.signal(text);
    let view = ui.mount(
        text_area("Note", value)
            .w(380.)
            .h(560.)
            .p(0.)
            .text_size(14.)
            .line_height(20.)
            .text_wrap(true)
            .span_highlighter(|_: &str, state| (Vec::new(), state))
            .line_marks(|i, _: &str, _| {
                vec![
                    LineMark::new(
                        0.,
                        0.,
                        f32::INFINITY,
                        f32::INFINITY,
                        Color(240, 240, 240, 255),
                    ),
                    LineMark::new(2., 0., 3., f32::INFINITY, Color(120, 120, 120, 255)).radius(1.),
                    LineMark::new(0., 0., 20., 20., Color(0, 0, 0, 0))
                        .label((format!("{}", i + 1).into(), Color(90, 90, 90, 255), 10.))
                        .tabular(true),
                ]
            }),
    );
    frame(&ui);
    let editor = ui.editor(view.node()).expect("its editor");
    for n in 0..frames {
        editor.set_scroll_offset((n % 300) as f32 * 20.);
        frame(&ui);
    }
    // It drew them: a wash a line in view, at least.
    let scene = ui.scene.borrow();
    let mut stack = vec![editor.node];
    let mut washes = 0;
    while let Some(id) = stack.pop() {
        if let zgx::scene::NodeKind::Quad(quad) = scene.kind(id) {
            washes += usize::from(quad.fill == Color(240, 240, 240, 255));
        }
        stack.extend(scene.children(id).iter().copied());
    }
    assert!(washes > 10, "{washes} marks drawn");
}
