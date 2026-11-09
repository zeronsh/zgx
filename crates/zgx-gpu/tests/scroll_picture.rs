//! Scroll content drawn from a picture (`pictures.rs`) draws what painting it
//! whole draws, every frame: a list scrolled both ways past the picture's
//! wrap, rows mounting and changing, what fades and what floats over it, at
//! 1x, 1.5x and 2x.
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

/// A list of `row`s over a dark fill.
fn list(offset: Signal<f32>, row: impl Fn(usize) -> View + 'static) -> View {
    virtual_list(offset, 24., 2, || 1000, |i| i, move |_, key, _| row(key))
        .bg(rgb(0x101820))
        .w(W)
        .h(H)
}

fn striped(key: usize) -> View {
    div()
        .w(200.)
        .h(20.)
        .rounded(4.)
        .bg(if key.is_multiple_of(2) {
            rgb(0x5a7a9a)
        } else {
            rgb(0x9a5a7a)
        })
        .child(text(format!("row {key}")).text_color(rgb(0xffffff)))
}

/// A row's line fading at its right edge: moves with the row.
fn fading(key: usize) -> View {
    div()
        .w(200.)
        .h(20.)
        .overflow_hidden()
        .fade_edges_x(0., 28.)
        .child(
            text(format!(
                "row {key} is a line far wider than the row it is in, so it fades"
            ))
            .text_color(rgb(0xffffff))
            .truncate(),
        )
}

/// Moves two `Ui`s built alike by `steps`, the one drawn from pictures and
/// the other painted whole, and compares every frame.
fn run(name: &str, scale: f32, steps: &[f32], build: impl Fn(&mut Ui, Signal<f32>)) {
    run_within(name, scale, steps, 0, 0., build);
}
/// `run`, each channel within `levels` of what painting it whole draws, but
/// for squares of `corners` points at the window's corners.
fn run_within(
    name: &str,
    scale: f32,
    steps: &[f32],
    levels: u8,
    corners: f32,
    build: impl Fn(&mut Ui, Signal<f32>),
) {
    let (w, h) = ((W * scale) as u32, (H * scale) as u32);
    let side = |pictures: bool| {
        let mut ui = Ui::new(W, H);
        ui.scene.borrow_mut().set_pixel_scale(scale);
        let offset = ui.signal(0_f32);
        build(&mut ui, offset.clone());
        let mut gpu = GpuRenderer::new(w, h).unwrap();
        gpu.set_scale_factor(scale);
        gpu.set_scroll_pictures(pictures);
        (ui, offset, gpu)
    };
    let (ui, offset, mut gpu) = side(true);
    let (plain, plain_offset, mut whole) = side(false);
    let full = Rect::new(0., 0., W, H);
    let mut held = 0;
    for (index, dy) in std::iter::once(&0.).chain(steps).enumerate() {
        offset.update(|o| *o += dy);
        plain_offset.update(|o| *o += dy);
        ui.prepare_frame();
        plain.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        let drawn = gpu.readback().unwrap();
        plain.scene.borrow_mut().flush();
        whole.render(&plain.scene.borrow(), &[full]).unwrap();
        let expected = whole.readback().unwrap();
        held = held.max(gpu.debug_pictures());
        let wrong = drawn
            .chunks(4)
            .zip(expected.chunks(4))
            .enumerate()
            .position(|(at, (a, b))| {
                let (x, y) = (
                    (at % w as usize) as f32 / scale,
                    (at / w as usize) as f32 / scale,
                );
                let corner = (x < corners || x >= W - corners) && (y < corners || y >= H - corners);
                !corner && a.iter().zip(b).any(|(a, b)| a.abs_diff(*b) > levels)
            });
        assert!(
            wrong.is_none(),
            "{name} at {scale}x, frame {index}: wrong at {:?}: {:?} for {:?}",
            wrong.map(|p| (p % w as usize, p / w as usize)),
            wrong.map(|p| &drawn[p * 4..p * 4 + 4]),
            wrong.map(|p| &expected[p * 4..p * 4 + 4]),
        );
    }
    assert!(held > 0, "{name} at {scale}x: no picture was drawn from");
}

const SCALES: [f32; 3] = [1., 1.5, 2.];
/// Down past the texture's height, back up past where it began, by steps
/// small and large (even points: whole pixels at every scale here).
const WALK: [f32; 16] = [
    2., 8., 40., 100., 180., 240., 300., 6., -4., -120., -300., -280., 2., 60., 400., -500.,
];

#[test]
fn a_list_moved_both_ways_past_its_wrap() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        run("list", scale, &WALK, |ui, offset| {
            ui.mount(list(offset, striped));
        });
    }
}

#[test]
fn rows_fading_across_move_with_the_list() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        run("fades", scale, &WALK, |ui, offset| {
            ui.mount(list(offset, fading));
        });
    }
}

#[test]
fn glass_over_the_list_stays_as_painted() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        run("glass", scale, &WALK, |ui, offset| {
            ui.mount(
                overlay().w(W).h(H).child(list(offset, striped)).child(
                    div()
                        .absolute()
                        .left(30.)
                        .top(120.)
                        .size(180., 44.)
                        .rounded(12.)
                        .backdrop_blur(8.)
                        .bg(rgba(0x20202080)),
                ),
            );
        });
    }
}

#[test]
fn a_row_changing_as_the_list_moves_shows_its_change() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let steps = [12_f32; 30];
        run("changing", scale, &steps, |ui, offset| {
            // Each row's fill follows the offset: every frame, what shows
            // changes as well as moves.
            let read = offset.clone();
            ui.mount(list(offset, move |key| {
                let read = read.clone();
                striped(key).reactive_style(move || {
                    let on = ((read.get() / 48.) as usize).is_multiple_of(2);
                    Styles::new().bg(if on { rgb(0x3a8a5a) } else { rgb(0x8a5a3a) })
                })
            }));
        });
    }
}

/// Rows past the view change (one grows, moving those after it; one changes
/// colour) while the picture holds them: scrolled back to, they show it.
#[test]
fn rows_changing_out_of_view_show_it_when_scrolled_to() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let steps = [
            20_f32, 20., 20., 20., 40., 40., -40., -40., -20., -20., -20., -20.,
        ];
        run("out of view", scale, &steps, |ui, offset| {
            let read = offset.clone();
            let row = move |key: usize| {
                let read = read.clone();
                div()
                    .w(200.)
                    .h(20.)
                    .mb(4.)
                    .bg(rgb(0x5a7a9a))
                    .child(text(format!("row {key}")).text_color(rgb(0xffffff)))
                    .reactive_style(move || {
                        let past = read.get() >= 120.;
                        match key {
                            1 if past => Styles::new().h(44.),
                            14 if past => Styles::new().bg(rgb(0xc04040)),
                            _ => Styles::new(),
                        }
                    })
            };
            ui.mount(
                scroll(offset)
                    .w(W)
                    .h(H)
                    .bg(rgb(0x101820))
                    .children((0..40).map(row)),
            );
        });
    }
}

/// With no one colour under it, an edge's coverage rounds a level off in the
/// picture, as in any layer: no further.
#[test]
fn a_list_over_what_varies_stays_within_a_level() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        run_within("over boxes", scale, &WALK, 2, 0., |ui, offset| {
            ui.mount(
                overlay()
                    .w(W)
                    .h(H)
                    .bg(rgb(0x000000))
                    .child(
                        div()
                            .absolute()
                            .left(20.)
                            .top(20.)
                            .size(100., 100.)
                            .bg(rgb(0x204060)),
                    )
                    .child(
                        virtual_list(offset, 24., 2, || 1000, |i| i, |_, key, _| striped(key))
                            .w(W)
                            .h(H),
                    ),
            );
        });
    }
}

/// A rounded clip around the view cuts the picture as it cuts what it holds.
/// (On its arc the picture takes its coverage once, as a group; painted, each
/// item stacked there takes it again, so those pixels aren't compared.)
#[test]
fn a_rounded_clip_around_the_view_cuts_it() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        run_within("rounded", scale, &WALK, 2, 17., |ui, offset| {
            ui.mount(
                div().w(W).h(H).bg(rgb(0x000000)).child(
                    div()
                        .w(W)
                        .h(H)
                        .rounded(16.)
                        .overflow_hidden()
                        .child(list(offset, striped)),
                ),
            );
        });
    }
}

/// Rows measured as they come, taller than their estimate: scrolling back up,
/// the list moves its content and the rows the other way in one pass, and
/// what shows stays where it is. The picture holds rows relative to the
/// content, so what it holds of them has to follow.
#[test]
fn a_measured_list_re_anchoring_stays_as_painted() {
    let _gpu = gpu_fixture();
    // Far down at a jump, then up past rows never measured.
    let steps = [
        4000_f32, 20., -40., -60., -60., -80., -100., -100., -60., -120., -40., -200., -300., -80.,
        60., -400.,
    ];
    for scale in SCALES {
        run("measured", scale, &steps, |ui, offset| {
            let heights = VariableHeights::new(&ui.runtime, 300, 24.);
            ui.mount(
                measured_virtual_list(
                    offset,
                    heights,
                    2,
                    |index| index,
                    |_, key, _| {
                        div()
                            .w(200.)
                            .h(30. + (key % 5) as f32 * 14.)
                            .bg(if key.is_multiple_of(2) {
                                rgb(0x5a7a9a)
                            } else {
                                rgb(0x9a5a7a)
                            })
                            .child(text(format!("row {key}")).text_color(rgb(0xffffff)))
                    },
                )
                .bg(rgb(0x101820))
                .w(W)
                .h(H),
            );
        });
    }
}
