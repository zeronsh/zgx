//! Scroll content drawn from a picture (`pictures.rs`) draws what painting it
//! whole draws, every frame: a list scrolled both ways past the picture's
//! wrap, rows mounting and changing, what fades and what floats over it, at
//! 1x and 2x. (At a fractional scale a view takes no picture.)
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

/// The scene's flush, told first what `gpu` last found of scroll pictures,
/// as the desktop host tells it (`GpuRenderer::pictures_for_scene`).
fn flush(ui: &Ui, gpu: &mut GpuRenderer) -> zgx::scene::FrameReport {
    let mut scene = ui.scene.borrow_mut();
    if let Some((allowed, wanted)) = gpu.pictures_for_scene() {
        scene.set_pictures_allowed(allowed);
        scene.want_pictures(wanted);
    }
    scene.flush()
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
        // Pictures draw what a copy can't move: no copy, on any platform
        // (on macOS an offscreen renderer copies none anyway; Vulkan would).
        if pictures {
            gpu.debug_refuse_scroll_copies();
        }
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
        let damage = flush(&ui, &mut gpu).damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        let drawn = gpu.readback().unwrap();
        flush(&plain, &mut whole);
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

const SCALES: [f32; 2] = [1., 2.];
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

/// A box in a row fading down its height, as the view's edges cross it: its
/// band keeps to where the view cuts it, so while crossing it doesn't move
/// with the content, and every frame draws as painted.
#[test]
fn boxes_fading_down_their_height_cross_the_views_edges_as_painted() {
    let _gpu = gpu_fixture();
    // By steps small and large, both ways past both edges, and in one jump
    // from out of view to wholly in it.
    let steps = [
        6_f32, 6., 8., 10., 40., 90., 8., 6., -4., -8., -30., -60., 300., -300., 8., 500., -500.,
    ];
    for scale in SCALES {
        run("fading down", scale, &steps, |ui, offset| {
            let row = |key: usize| {
                div()
                    .w(200.)
                    .h(if key.is_multiple_of(3) { 96. } else { 24. })
                    .mb(4.)
                    .bg(rgb(0x5a7a9a))
                    .overflow_hidden()
                    .fade_edges(16., 24.)
                    .child(text(format!("row {key}")).text_color(rgb(0xffffff)))
                    .child(div().w(180.).h(80.).bg(rgb(0x9a5a7a)))
            };
            ui.mount(
                scroll(offset)
                    .w(W)
                    .h(H)
                    .bg(rgb(0x101820))
                    .children((0..60).map(row)),
            );
        });
    }
}

/// A fade down the height of nothing it clips has the view's box: no picture
/// can hold that, and the view goes on as it was.
#[test]
fn a_fade_with_the_views_box_refuses_a_picture() {
    let _gpu = gpu_fixture();
    let mut ui = Ui::new(W, H);
    ui.scene.borrow_mut().set_pixel_scale(1.);
    let offset = ui.signal(0_f32);
    ui.mount(
        scroll(offset.clone())
            .w(W)
            .h(H)
            .bg(rgb(0x101820))
            .children((0..40).map(|key| {
                div()
                    .w(200.)
                    .h(24.)
                    .fade_edges(8., 8.)
                    .bg(rgb(0x5a7a9a))
                    .child(text(format!("row {key}")).text_color(rgb(0xffffff)))
            })),
    );
    let mut gpu = GpuRenderer::new(W as u32, H as u32).unwrap();
    gpu.set_scroll_pictures(true);
    // (Refused a copy too: no picture for its own reason, not a copy's.)
    gpu.debug_refuse_scroll_copies();
    for _ in 0..4 {
        offset.update(|o| *o += 8.);
        ui.prepare_frame();
        let damage = flush(&ui, &mut gpu).damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        assert_eq!(gpu.debug_pictures(), 0);
    }
}

/// A dithered image takes its dots from the window's pixels, not its own:
/// moved in a picture they would swim with it. It refuses one.
#[test]
fn an_image_drawn_by_window_position_refuses_a_picture() {
    let _gpu = gpu_fixture();
    let flat = std::sync::Arc::new(
        zgx::image::ImageData::new(8, 8, [200u8, 100, 50, 255].repeat(64)).unwrap(),
    );
    let mut ui = Ui::new(W, H);
    ui.scene.borrow_mut().set_pixel_scale(1.);
    let offset = ui.signal(0_f32);
    ui.mount(
        scroll(offset.clone())
            .w(W)
            .h(H)
            .bg(rgb(0x101820))
            .children((0..40).map(|key| {
                div()
                    .w(200.)
                    .h(24.)
                    .bg(rgb(0x5a7a9a))
                    .child(text(format!("row {key}")))
            })),
    );
    let content = {
        let scene = ui.scene.borrow();
        let mut stack = vec![scene.root()];
        let mut found = None;
        while let Some(id) = stack.pop() {
            if scene.is_scroll_content(id) {
                found = Some(id);
            }
            stack.extend_from_slice(scene.children(id));
        }
        found.expect("scroll content")
    };
    {
        let mut scene = ui.scene.borrow_mut();
        scene.append(
            content,
            zgx::scene::NodeKind::Image(std::sync::Arc::new(flat.dithered(2.))),
            zgx::scene::Style {
                width: Some(8.),
                height: Some(8.),
                ..Default::default()
            },
        );
    }
    let mut gpu = GpuRenderer::new(W as u32, H as u32).unwrap();
    gpu.set_scroll_pictures(true);
    // (Refused a copy too: no picture for its own reason, not a copy's.)
    gpu.debug_refuse_scroll_copies();
    for _ in 0..4 {
        offset.update(|o| *o += 8.);
        ui.prepare_frame();
        let damage = flush(&ui, &mut gpu).damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        assert_eq!(gpu.debug_pictures(), 0);
    }
}

/// A layer in a run of boxes under one fade: bands fade stacked boxes apart
/// or together by what the run holds, and a picture walks what a paint walks.
#[test]
fn a_layer_in_a_faded_run_draws_as_painted() {
    let _gpu = gpu_fixture();
    let steps = [6_f32, 8., 40., 90., 8., -4., -30., -60., 300., -300.];
    for scale in SCALES {
        run("layer under a fade", scale, &steps, |ui, offset| {
            let row = |key: usize| {
                div()
                    .w(200.)
                    .h(if key.is_multiple_of(2) { 96. } else { 40. })
                    .mb(4.)
                    .bg(rgb(0x5a7a9a))
                    .overflow_hidden()
                    .fade_edges(16., 24.)
                    .child(div().w(180.).h(30.).bg(rgb(0x9a5a7a)))
                    .child(div().w(120.).h(30.).bg(rgb(0x3a8a5a)).filter_blur(2.))
                    .child(text(format!("row {key}")).text_color(rgb(0xffffff)))
            };
            ui.mount(
                scroll(offset)
                    .w(W)
                    .h(H)
                    .bg(rgb(0x101820))
                    .children((0..60).map(row)),
            );
        });
    }
}

/// At a fractional scale a texture's rows aren't whole points, and text could
/// snap a pixel off: the view goes on without a picture.
#[test]
fn a_fractional_scale_refuses_a_picture() {
    let _gpu = gpu_fixture();
    let scale = 1.5;
    let mut ui = Ui::new(W, H);
    ui.scene.borrow_mut().set_pixel_scale(scale);
    let offset = ui.signal(0_f32);
    ui.mount(list(offset.clone(), striped));
    let mut gpu = GpuRenderer::new((W * scale) as u32, (H * scale) as u32).unwrap();
    gpu.set_scale_factor(scale);
    gpu.set_scroll_pictures(true);
    // (Refused a copy too: no picture for its own reason, not a copy's.)
    gpu.debug_refuse_scroll_copies();
    for _ in 0..4 {
        offset.update(|o| *o += 8.);
        ui.prepare_frame();
        let damage = flush(&ui, &mut gpu).damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        assert_eq!(gpu.debug_pictures(), 0);
    }
}

/// Content a copy moves stays a copy: cheaper than a picture, which repaints
/// all its view shows each frame.
#[test]
fn content_a_copy_moves_takes_no_picture() {
    let _gpu = gpu_fixture();
    let mut ui = Ui::new(W, H);
    ui.scene.borrow_mut().set_pixel_scale(1.);
    let offset = ui.signal(0_f32);
    ui.mount(list(offset.clone(), |key| {
        div().w(200.).h(20.).bg(if key.is_multiple_of(2) {
            rgb(0x5a7a9a)
        } else {
            rgb(0x9a5a7a)
        })
    }));
    let mut gpu = GpuRenderer::new(W as u32, H as u32).unwrap();
    gpu.set_scroll_pictures(true);
    gpu.debug_allow_scroll_copies();
    for _ in 0..6 {
        offset.update(|o| *o += 8.);
        ui.prepare_frame();
        let damage = flush(&ui, &mut gpu).damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        assert_eq!(gpu.debug_pictures(), 0);
    }
    assert!(gpu.debug_scroll_copies() > 0, "it moved by copies");
}
