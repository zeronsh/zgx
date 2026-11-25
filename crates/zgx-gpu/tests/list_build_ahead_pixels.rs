//! A list whose rows are built ahead of its view in spare time (`zgx::spare`),
//! drawn as a window draws it (each frame only where it is damaged, its
//! layers and scroll copies reused), shows exactly what the same list drawn
//! whole each frame shows: at its end as it opens, scrolled, at rest as its
//! kept rows are let go, and scrolled again. As Zeron's transcript is: rows
//! of natural heights the estimate doesn't know, held at the end, its edges
//! faded.
use std::time::{Duration, Instant};
use zgx::{compose::prelude::*, input::InputEvent, scene::Rect, widgets::Ui};
use zgx_gpu::GpuRenderer;

const W: u32 = 240;
const H: u32 = 200;

fn window(faded: bool, scale: f32) -> Ui {
    let mut ui = Ui::new(W as f32, H as f32);
    ui.scene.borrow_mut().set_pixel_scale(scale);
    ui.host_spare_time();
    let offset = ui.signal(f32::MAX);
    let heights = VariableHeights::new(&ui.runtime, 400, 40.).anchor_end();
    let list = measured_virtual_list(
        offset,
        heights,
        3,
        |i| i,
        |_, i, _| {
            // Short prompts and replies far taller than the estimate, each
            // painting lines down its height.
            let tall = i % 2 == 1;
            let lines = if tall { 6 + (i % 5) * 3 } else { 1 };
            let mut row = column().w_full().gap(8.);
            for k in 0..lines {
                row = row.child(
                    div()
                        .w(30. + ((i * 7 + k * 13) % 150) as f32)
                        .h(10.)
                        .bg(rgb(0x4070b0 + (i as u32 % 7) * 0x101010)),
                );
            }
            row
        },
    )
    .w_full()
    .h_full();
    let list = if faded {
        list.reactive_style(|| Styles::new().fade_edges(24., 40.))
    } else {
        list
    };
    ui.mount(list);
    ui
}

/// Drives two copies of the same window the same way: one drawn as a window
/// draws (damage only), one drawn whole each frame. After each frame their
/// pixels agree.
struct Pair {
    live: Ui,
    whole: Ui,
    drawn: GpuRenderer,
    fresh: GpuRenderer,
    width: u32,
}

impl Pair {
    fn new(faded: bool, scale: f32) -> Self {
        let (w, h) = ((W as f32 * scale) as u32, (H as f32 * scale) as u32);
        let mut drawn = GpuRenderer::new(w, h).unwrap();
        let mut fresh = GpuRenderer::new_with_context(w, h, &drawn.context()).unwrap();
        drawn.set_scale_factor(scale);
        fresh.set_scale_factor(scale);
        Self {
            live: window(faded, scale),
            whole: window(faded, scale),
            drawn,
            fresh,
            width: w,
        }
    }
    fn both(&mut self, f: impl Fn(&mut Ui)) {
        f(&mut self.live);
        f(&mut self.whole);
    }
    /// A refresh: spare time (all it asks for, the same in both), then its
    /// frame.
    fn refresh(&mut self, what: &str) {
        for ui in [&self.live, &self.whole] {
            while ui.spare_work_wanted() {
                ui.run_spare_work(Instant::now(), Duration::from_secs(60))
                    .unwrap();
            }
            ui.try_prepare_frame().unwrap();
        }
        let report = self.live.scene.borrow_mut().flush();
        self.drawn
            .render(&self.live.scene.borrow(), &report.damage)
            .unwrap();
        self.whole.scene.borrow_mut().flush();
        self.fresh
            .render(
                &self.whole.scene.borrow(),
                &[Rect::new(0., 0., W as f32, H as f32)],
            )
            .unwrap();
        let (a, b) = (
            self.drawn.readback().unwrap(),
            self.fresh.readback().unwrap(),
        );
        let differing: Vec<usize> = a
            .chunks(4)
            .zip(b.chunks(4))
            .enumerate()
            .filter(|(_, (a, b))| a.iter().zip(b.iter()).any(|(a, b)| a.abs_diff(*b) > 2))
            .map(|(i, _)| i)
            .collect();
        assert!(
            differing.is_empty(),
            "{what}: {} pixels differ from a whole repaint, first at {:?}",
            differing.len(),
            differing
                .first()
                .map(|i| (i % self.width as usize, i / self.width as usize))
        );
    }
}

fn wheel(ui: &mut Ui, dy: f32) {
    // (Trackpad steps land on fractions of a point.)
    ui.dispatch(InputEvent::Scroll {
        x: 100.,
        y: 100.,
        delta_x: 0.,
        delta_y: dy,
    });
}

fn opened_built_ahead_rested_and_scrolled(faded: bool, scale: f32) {
    // (One test drawing at a time, as the other pixel tests do.)
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _gpu = GPU.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut pair = Pair::new(faded, scale);
    for step in 0..30 {
        pair.refresh(&format!("opening {step}"));
    }
    for step in 0..40 {
        pair.both(|ui| wheel(ui, -30.));
        pair.refresh(&format!("up {step}"));
    }
    for step in 0..60 {
        pair.both(|ui| wheel(ui, 30.));
        pair.refresh(&format!("down {step}"));
    }
    for step in 0..10 {
        pair.refresh(&format!("rest {step}"));
    }
    // A while at rest: the lists let go of what they kept, with no frame
    // after (a window draws none until something changes).
    pair.both(|ui| ui.release_kept_rows().unwrap());
    for step in 0..60 {
        pair.both(|ui| wheel(ui, -12.25));
        pair.refresh(&format!("up after the rest {step}"));
    }
}

#[test]
fn a_list_built_ahead_draws_what_a_whole_repaint_draws() {
    opened_built_ahead_rested_and_scrolled(false, 1.);
    opened_built_ahead_rested_and_scrolled(false, 2.);
}

#[test]
fn a_faded_list_built_ahead_draws_what_a_whole_repaint_draws() {
    opened_built_ahead_rested_and_scrolled(true, 1.);
    opened_built_ahead_rested_and_scrolled(true, 2.);
}
