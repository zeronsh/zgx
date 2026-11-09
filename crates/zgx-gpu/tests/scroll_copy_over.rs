//! A scroll view copies its pixels while something stays put over it, or a
//! see-through fill lies under it (`copy_blocker`): every frame drawn from its
//! damage (the copy among it) must match the frame painted whole, whatever
//! changes over or under the view as it scrolls: an overlay sliding, fading,
//! appearing; a card fading under a list; a card's shadow and edge in the
//! view; two panes side by side under one overlay; at 1x, 1.5x and 2x.
use zgx::{
    compose::prelude::{rgba, *},
    reactive::Signal,
    scene::Rect,
    widgets::Ui,
};
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const W: f32 = 240.;
const H: f32 = 180.;

/// A list of `n` striped rows, its fill `bg` (none: see-through).
fn list(offset: Signal<f32>, bg: Option<u32>) -> View {
    let rows = virtual_list(
        offset,
        24.,
        2,
        || 1000,
        |i| i,
        |_, key, _| {
            div().size(120., 14.).bg(if key % 2 == 0 {
                rgb(0x5a7a9a)
            } else {
                rgb(0x9a5a7a)
            })
        },
    );
    match bg {
        Some(bg) => rows.bg(rgb(bg)),
        None => rows,
    }
}

/// (Positions and moves land on whole device pixels at every scale here,
/// even points at 1.5x, so each move copies rather than repaints.)
/// Runs `steps` frames at `scale`: each moves `offset` by `dy` and then
/// `change(step)` (what moves over or under the list), and checks the frame
/// drawn from its damage against the frame painted whole.
fn run(
    name: &str,
    scale: f32,
    ui: &mut Ui,
    offset: &Signal<f32>,
    steps: usize,
    dy: f32,
    change: impl FnMut(usize),
) {
    let (w, h) = ((W * scale) as u32, (H * scale) as u32);
    let gpu = GpuRenderer::new(w, h).unwrap();
    run_on(gpu, name, scale, ui, offset, steps, dy, change);
}

#[allow(clippy::too_many_arguments)]
fn run_on(
    mut gpu: GpuRenderer,
    name: &str,
    scale: f32,
    ui: &mut Ui,
    offset: &Signal<f32>,
    steps: usize,
    dy: f32,
    mut change: impl FnMut(usize),
) {
    let w = (W * scale) as u32;
    gpu.set_scale_factor(scale);
    gpu.debug_enable_single_pass();
    // (Copies on every platform, so what a copy leaves is what's checked.)
    gpu.debug_allow_scroll_copies();
    let full = Rect::new(0., 0., W, H);
    let mut frame = |ui: &mut Ui, label: &str| {
        ui.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        let partial = gpu.debug_present_offscreen().unwrap();
        gpu.render(&ui.scene.borrow(), &[full]).unwrap();
        let fresh = gpu.debug_present_offscreen().unwrap();
        let wrong = partial
            .chunks(4)
            .zip(fresh.chunks(4))
            .position(|(a, b)| a != b);
        assert!(
            wrong.is_none(),
            "{name} at {scale}x, {label}: a stale pixel at {:?} (points {:?})",
            wrong.map(|p| (p % w as usize, p / w as usize)),
            wrong.map(|p| (
                (p % w as usize) as f32 / scale,
                (p / w as usize) as f32 / scale
            )),
        );
    };
    frame(ui, "first");
    frame(ui, "settled");
    for step in 0..steps {
        offset.update(|o| *o += dy);
        change(step);
        frame(ui, &format!("step {step}"));
    }
    drop(frame);
    // (Its moves were copies, not repaints: else nothing here was checked.)
    assert!(
        gpu.debug_scroll_copies() > 0,
        "{name} at {scale}x: no scroll was copied"
    );
}

const SCALES: [f32; 3] = [1., 1.5, 2.];

/// A square over the list slides across it as the list scrolls (a pill
/// easing in over a transcript).
#[test]
fn an_overlay_sliding_over_a_scrolling_list() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let mut ui = Ui::new(W, H);
        let offset = ui.signal(0_f32);
        let at = ui.signal(20_f32);
        let read = at.clone();
        ui.mount(
            overlay()
                .w(W)
                .h(H)
                .bg(rgb(0x000000))
                .child(list(offset.clone(), Some(0x101820)).w(W).h(H))
                .child(
                    div()
                        .absolute()
                        .left(30.)
                        .top(60.)
                        .size(40., 30.)
                        .rounded(8.)
                        .bg(rgba(0xffffffc0))
                        .reactive_style(move || {
                            Styles::new().translate(read.get(), read.get() / 3.)
                        }),
                ),
        );
        run("sliding overlay", scale, &mut ui, &offset, 12, 8., |step| {
            at.set(20. + step as f32 * 7.);
        });
    }
}

/// A square over the list fades as it scrolls (a toast going).
#[test]
fn an_overlay_fading_over_a_scrolling_list() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let mut ui = Ui::new(W, H);
        let offset = ui.signal(0_f32);
        let alpha = ui.signal(1_f32);
        let read = alpha.clone();
        ui.mount(
            overlay()
                .w(W)
                .h(H)
                .bg(rgb(0x000000))
                .child(list(offset.clone(), Some(0x101820)).w(W).h(H))
                .child(
                    div()
                        .absolute()
                        .left(60.)
                        .top(50.)
                        .size(80., 40.)
                        .rounded(10.)
                        .bg(rgb(0xeeeeee))
                        .reactive_style(move || Styles::new().opacity(read.get())),
                ),
        );
        run("fading overlay", scale, &mut ui, &offset, 10, 6., |step| {
            alpha.set(1. - step as f32 / 10.);
        });
    }
}

/// A square over the list comes and goes as it scrolls.
#[test]
fn an_overlay_appearing_and_going_over_a_scrolling_list() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let mut ui = Ui::new(W, H);
        let offset = ui.signal(0_f32);
        let shown = ui.signal(false);
        let read = shown.clone();
        ui.mount(
            overlay()
                .w(W)
                .h(H)
                .bg(rgb(0x000000))
                .child(list(offset.clone(), Some(0x101820)).w(W).h(H))
                .child(
                    div()
                        .absolute()
                        .left(50.)
                        .top(70.)
                        .size(60., 30.)
                        .bg(rgb(0xff8800))
                        .reactive_style(move || {
                            if read.get() {
                                Styles::new()
                            } else {
                                Styles::new().hidden()
                            }
                        }),
                ),
        );
        run(
            "appearing overlay",
            scale,
            &mut ui,
            &offset,
            8,
            10.,
            |step| {
                shown.set(step % 3 != 1);
            },
        );
    }
}

/// A list with no fill on a see-through rounded card, its edge and shadow in
/// the view, the card fading as the list scrolls.
#[test]
fn a_see_through_card_fading_under_a_scrolling_list() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let mut ui = Ui::new(W, H);
        let offset = ui.signal(0_f32);
        let alpha = ui.signal(1_f32);
        let read = alpha.clone();
        ui.mount(
            overlay()
                .w(W)
                .h(H)
                .bg(rgb(0x334455))
                .child(
                    div()
                        .absolute()
                        .left(10.)
                        .top(10.)
                        .size(220., 160.)
                        .rounded(14.)
                        .border(1.)
                        .border_color(rgba(0xffffff30))
                        .bg(rgba(0x202020b0))
                        .shadow(zgx::scene::BoxShadow {
                            color: rgba(0x00000080),
                            offset: zgx::scene::Transform { x: 0., y: 3. },
                            blur_radius: 8.,
                            spread: 0.,
                        })
                        .reactive_style(move || Styles::new().opacity(read.get())),
                )
                .child(
                    list(offset.clone(), None)
                        .absolute()
                        .left(12.)
                        .top(12.)
                        .w(216.)
                        .h(156.),
                ),
        );
        run("fading card", scale, &mut ui, &offset, 10, 8., |step| {
            alpha.set(1. - step as f32 / 20.);
        });
    }
}

/// The same card holding still, its rounded corners and edge in the view.
#[test]
fn a_see_through_card_under_a_scrolling_list() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let mut ui = Ui::new(W, H);
        let offset = ui.signal(0_f32);
        ui.mount(
            overlay()
                .w(W)
                .h(H)
                .bg(rgb(0x334455))
                .child(
                    div()
                        .absolute()
                        .left(10.)
                        .top(10.)
                        .size(220., 160.)
                        .rounded(14.)
                        .border(1.)
                        .border_color(rgba(0xffffff30))
                        .bg(rgba(0x202020b0)),
                )
                .child(
                    list(offset.clone(), None)
                        .absolute()
                        .left(12.)
                        .top(12.)
                        .w(216.)
                        .h(156.),
                ),
        );
        run("still card", scale, &mut ui, &offset, 10, 10., |_| {});
    }
}

/// Two panes side by side, one overlay across both (a composer over a
/// split), one pane scrolling, then the other.
#[test]
fn two_panes_under_one_overlay() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let mut ui = Ui::new(W, H);
        let left = ui.signal(0_f32);
        let right = ui.signal(0_f32);
        ui.mount(
            overlay()
                .w(W)
                .h(H)
                .bg(rgb(0x000000))
                .child(
                    list(left.clone(), Some(0x101820))
                        .absolute()
                        .left(0.)
                        .top(0.)
                        .w(118.)
                        .h(H),
                )
                .child(
                    list(right.clone(), Some(0x182010))
                        .absolute()
                        .left(122.)
                        .top(0.)
                        .w(118.)
                        .h(H),
                )
                .child(
                    div()
                        .absolute()
                        .left(40.)
                        .top(130.)
                        .size(160., 36.)
                        .rounded(12.)
                        .bg(rgba(0xddddddd0)),
                ),
        );
        let right_too = right.clone();
        run("split", scale, &mut ui, &left, 12, 6., |step| {
            if step >= 6 {
                right_too.update(|o| *o += 12.);
            }
        });
    }
}

/// A label over the list holding still, its words running past its box (a
/// narrow badge, no clip): the copy moves all of its glyphs, so all of them
/// repaint, not only those in its box.
#[test]
fn a_label_running_past_its_box_over_a_scrolling_list() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let (w, h) = ((W * scale) as u32, (H * scale) as u32);
        let gpu = GpuRenderer::new(w, h).unwrap();
        let mut ui = Ui::new(W, H);
        gpu.install_text(&mut ui.scene.borrow_mut());
        let offset = ui.signal(0_f32);
        ui.mount(
            overlay()
                .w(W)
                .h(H)
                .bg(rgb(0x000000))
                .child(list(offset.clone(), Some(0x101820)).w(W).h(H))
                .child(
                    text("A label much wider than its box")
                        .absolute()
                        .left(20.)
                        .top(60.)
                        .w(30.)
                        .h(20.)
                        .text_size(16.)
                        .text_color(rgb(0xffffff)),
                ),
        );
        run_on(
            gpu,
            "overflowing label",
            scale,
            &mut ui,
            &offset,
            8,
            10.,
            |_| {},
        );
    }
}
