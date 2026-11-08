//! Style transitions running (`zgx::transition`), headless and undrawn: a
//! pointer moving down a list of rows whose background and opacity ease
//! on hover, each frame's transitions stepped as a window's host steps them
//! (`Ui::advance_interactions` at its deadline), then the frame flushed.
//! CI's instruction gate counts it: what a transition costs the frame that
//! has one (the other cells say what it costs those that have none).
//! Run: cargo run -p zgx --release --example transition_bench
//! (`FRAMES` sets the frames, 600 by default.)
use std::time::{Duration, Instant};
use zgx::{
    compose::{column, row, text},
    input::InputEvent,
    style::{Styled, rgb},
    transition::{Ease, Transition, TransitionProp},
    widgets::Ui,
};

fn main() {
    let frames: usize = std::env::var("FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let mut ui = Ui::new(600., 800.);
    let list = column().children((0..40).map(|i| {
        row()
            .h(20.)
            .bg(rgb(0x18202c))
            .opacity(0.8)
            .hover(|s| s.bg(rgb(0x2a3a50)).opacity(1.))
            .transition(
                Transition::of([TransitionProp::Background, TransitionProp::Opacity])
                    .duration(150.)
                    .ease(Ease::CubicBezier(0.2, 0., 0., 1.)),
            )
            .child(text(format!("Row {i}")))
    }));
    let _handle = ui.mount(list);
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    // Down the list and back, a row every few frames: each row eases in as
    // the pointer comes, out as it leaves, a few at once.
    let start = Instant::now();
    let mut stepped = 0;
    for frame in 0..frames {
        let row = (frame / 4) % 80;
        let y = 10. + (if row < 40 { row } else { 79 - row }) as f32 * 20.;
        ui.dispatch(InputEvent::PointerMove { x: 20., y });
        ui.prepare_frame();
        let now = start + Duration::from_micros(16_667 * frame as u64);
        stepped += usize::from(ui.advance_interactions(now).unwrap_or(false));
        ui.scene.borrow_mut().flush();
    }
    println!("{frames} frames, transitions stepped on {stepped}");
}
