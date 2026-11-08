//! A split between six panes dragged, a width a frame, headless and
//! undrawn: each frame lays the page out again (`prepare_frame`, `flush`),
//! its text measured by zgx's own fallback, so what's counted is layout.
//! CI's instruction gate counts it (`layout-drag`); the layout test
//! `a_dragged_split_measures_within_its_budget` counts its passes and
//! measures on the same page (`support/layout_drag_page.rs`).
//! Run: cargo run -p zgx --release --example layout_drag_bench
//! (`ROUNDS` sets the drags there and back, 3 by default.)
use zgx::widgets::Ui;

#[path = "support/layout_drag_page.rs"]
mod layout_drag_page;

fn main() {
    let rounds: usize = std::env::var("ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let mut ui = Ui::new(2000., 1100.);
    let split = ui.signal(320_f32);
    let runtime = ui.runtime.clone();
    let _handle = ui.mount(layout_drag_page::page(split.clone(), &runtime));
    for _ in 0..3 {
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
    }
    let (mut frames, mut placed) = (0, 0);
    for _ in 0..rounds {
        for width in layout_drag_page::widths(30, 12.) {
            split.set(width);
            ui.prepare_frame();
            placed += ui.scene.borrow_mut().flush().layout_nodes;
            frames += 1;
        }
    }
    println!(
        "{frames} widths, {:.0} nodes placed a frame, scene nodes {}",
        placed as f64 / frames as f64,
        ui.scene.borrow().len()
    );
}
