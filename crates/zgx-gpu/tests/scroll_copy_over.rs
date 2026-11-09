//! A scroll view copies its pixels while something stays put over it, or a
//! see-through fill lies under it (`copy_blocker`): every frame drawn from its
//! damage (the copy among it) must match the frame painted whole, whatever
//! changes over or under the view as it scrolls: an overlay sliding, fading,
//! appearing; a card fading under a list; a card's shadow and edge in the
//! view; two panes side by side under one overlay; at 1x, 1.5x and 2x. And
//! whatever the content holds: its own fades, clips fading at their foot and
//! layers move with it and copy; what reads the pixels under it does not.
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
    gpu: GpuRenderer,
    name: &str,
    scale: f32,
    ui: &mut Ui,
    offset: &Signal<f32>,
    steps: usize,
    dy: f32,
    change: impl FnMut(usize),
) {
    // (Its moves were copies, not repaints: else nothing here was checked.)
    assert!(
        drawn_alike(gpu, name, scale, ui, offset, steps, dy, 0, change) > 0,
        "{name} at {scale}x: no scroll was copied"
    );
}

/// `run_on`'s frames, each checked against a second renderer painting the
/// scene whole (so what a copy gets wrong stays, for the next copy to carry),
/// up to `levels` of colour; how many of them copied.
#[allow(clippy::too_many_arguments)]
fn drawn_alike(
    mut gpu: GpuRenderer,
    name: &str,
    scale: f32,
    ui: &mut Ui,
    offset: &Signal<f32>,
    steps: usize,
    dy: f32,
    levels: u8,
    mut change: impl FnMut(usize),
) -> u64 {
    let (w, h) = ((W * scale) as u32, (H * scale) as u32);
    gpu.set_scale_factor(scale);
    gpu.debug_enable_single_pass();
    // (Copies on every platform, so what a copy leaves is what's checked.)
    gpu.debug_allow_scroll_copies();
    let mut whole = GpuRenderer::new(w, h).unwrap();
    whole.set_scale_factor(scale);
    whole.debug_enable_single_pass();
    let full = Rect::new(0., 0., W, H);
    let mut frame = |ui: &mut Ui, label: &str| {
        ui.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        let partial = gpu.debug_present_offscreen().unwrap();
        whole.render(&ui.scene.borrow(), &[full]).unwrap();
        let fresh = whole.debug_present_offscreen().unwrap();
        let w = w as usize;
        let off = |a: &[u8], b: &[u8]| a.iter().zip(b).any(|(a, b)| a.abs_diff(*b) > levels);
        let wrong = partial
            .chunks(4)
            .zip(fresh.chunks(4))
            .position(|(a, b)| off(a, b));
        assert!(
            wrong.is_none(),
            "{name} at {scale}x, {label}: a stale pixel at {:?} (points {:?})",
            wrong.map(|p| (p % w, p / w)),
            wrong.map(|p| ((p % w) as f32 / scale, (p / w) as f32 / scale)),
        );
    };
    frame(ui, "first");
    frame(ui, "settled");
    for step in 0..steps {
        offset.update(|o| *o += dy);
        change(step);
        frame(ui, &format!("step {step}"));
    }
    gpu.debug_scroll_copies()
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

/// A view scrolling down over `content` on an opaque fill, fading at its own
/// top and bottom if `fade`, `steps` moves of `dy`: each frame checked, and
/// how many copied. (Within a level: a ramp's or a turned layer's sums round
/// differently where the view moved them, as a rounded corner's do.)
#[allow(clippy::too_many_arguments)]
fn in_a_view(
    name: &str,
    scale: f32,
    fade: bool,
    steps: usize,
    dy: f32,
    content: impl FnOnce(&Ui) -> View,
    mut change: impl FnMut(usize),
) -> u64 {
    let (w, h) = ((W * scale) as u32, (H * scale) as u32);
    let gpu = GpuRenderer::new(w, h).unwrap();
    let mut ui = Ui::new(W, H);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let offset = ui.signal(0_f32);
    let view = scroll(offset.clone()).w(W).h(H).bg(rgb(0x101820));
    let view = if fade {
        view.fade_edges(16., 16.)
    } else {
        view
    };
    let content = content(&ui);
    ui.mount(
        overlay()
            .w(W)
            .h(H)
            .bg(rgb(0x000000))
            .child(view.child(content)),
    );
    drawn_alike(
        gpu,
        name,
        scale,
        &mut ui,
        &offset,
        steps,
        dy,
        1,
        &mut change,
    )
}

/// A stripe of cells across `width`, its own colours.
fn stripes(width: f32, height: f32) -> View {
    let mut row = row().w(width).h(height);
    for i in 0..(width / 30.) as u32 {
        row = row.child(div().w(30.).h(height).bg(if i % 2 == 0 {
            rgb(0x7a5a9a)
        } else {
            rgb(0x5a9a7a)
        }));
    }
    row
}

/// Tables in a transcript: each a sideways scroll fading at its right edge
/// while it overflows, its rows and words under that fade, a cell fading under
/// it too (two fades of the content's own, and the view's own past them), in a
/// view with a fade of its own: moves copy (its interior; the bands repaint).
#[test]
fn tables_fading_in_a_fading_view() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let steps = 24;
        let copies = in_a_view(
            "tables",
            scale,
            true,
            steps,
            8.,
            |ui| {
                let mut page = column().w(W).gap(10.);
                for i in 0..12 {
                    page = page.child(
                        scroll_x(ui.signal(i as f32 * 6.))
                            .w(W)
                            .h(70.)
                            .fade_edges_x(24., 24.)
                            .fade_overflow(true)
                            .child(
                                column()
                                    .w(600.)
                                    .child(stripes(600., 24.))
                                    .child(
                                        text("Words in a table's row, running across it")
                                            .w(600.)
                                            .h(20.)
                                            .text_size(14.)
                                            .text_color(rgb(0xffffff)),
                                    )
                                    .child(
                                        div()
                                            .w(600.)
                                            .h(26.)
                                            .overflow_hidden()
                                            .fade_edges(0., 10.)
                                            .child(stripes(600., 40.)),
                                    ),
                            ),
                    );
                }
                page
            },
            |_| {},
        );
        assert_eq!(copies, steps as u64, "tables at {scale}x");
    }
}

/// A view filling the window, its fade cut short by `fade_overflow` near its
/// top (which repaints it whole), and held in full further on, where it copies
/// like any other: painting it whole didn't leave it painting whole for good.
#[test]
fn a_view_fading_as_far_as_it_overflows() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let (w, h) = ((W * scale) as u32, (H * scale) as u32);
        let gpu = GpuRenderer::new(w, h).unwrap();
        let mut ui = Ui::new(W, H);
        let offset = ui.signal(0_f32);
        let mut page = column().w(W);
        for _ in 0..20 {
            page = page.child(stripes(W, 30.));
        }
        ui.mount(
            overlay().w(W).h(H).bg(rgb(0x000000)).child(
                scroll(offset.clone())
                    .w(W)
                    .h(H)
                    .bg(rgb(0x101820))
                    .fade_edges(30., 30.)
                    .fade_overflow(true)
                    .child(page),
            ),
        );
        // From its top (no band there, then one growing) to well past it.
        let copies = drawn_alike(gpu, "overflow", scale, &mut ui, &offset, 20, 6., 1, |_| {});
        // (Five steps in, its top band is whole: of the rest, one is a frame
        // made new by the last of those.)
        assert!(copies >= 13, "overflow at {scale}x: {copies} copied");
    }
}

/// A folded bubble: a clip fading at its foot. Scrolling on, its foot goes
/// past the view's bottom and comes back: while the view cuts it its band
/// stands at the view's edge, not on the bubble, and what it holds repaints.
#[test]
fn clips_fading_at_their_foot() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        for inset in [0., 6.] {
            let steps = 36;
            let copies = in_a_view(
                "foot",
                scale,
                false,
                steps,
                6.,
                |_| {
                    let mut page = column().w(W).gap(12.);
                    for _ in 0..16 {
                        page = page.child(
                            div()
                                .w(W)
                                .h(90.)
                                .overflow_hidden()
                                .fade_edges(0., 30.)
                                .fade_insets(inset, inset)
                                .child(stripes(W, 200.)),
                        );
                    }
                    page
                },
                |_| {},
            );
            assert_eq!(copies, steps as u64, "foot at {scale}x, inset {inset}");
        }
    }
}

/// The same at the top of a clip, moving each way, and sideways under a fade.
#[test]
fn clips_fading_at_their_head_as_the_view_comes_back() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let (w, h) = ((W * scale) as u32, (H * scale) as u32);
        let gpu = GpuRenderer::new(w, h).unwrap();
        let mut ui = Ui::new(W, H);
        let offset = ui.signal(0_f32);
        let mut page = column().w(W).gap(12.);
        for _ in 0..16 {
            page = page.child(
                div()
                    .w(W)
                    .h(90.)
                    .overflow_hidden()
                    .fade_edges(30., 0.)
                    .fade_insets(5., 0.)
                    .child(stripes(W, 200.)),
            );
        }
        ui.mount(
            overlay().w(W).h(H).bg(rgb(0x000000)).child(
                scroll(offset.clone())
                    .w(W)
                    .h(H)
                    .bg(rgb(0x101820))
                    .child(page),
            ),
        );
        let to = offset.clone();
        // (A jump first, which repaints, to come back from.)
        let copies = drawn_alike(gpu, "head", scale, &mut ui, &offset, 31, -6., 1, |step| {
            if step == 0 {
                to.set(600.);
            }
        });
        assert_eq!(copies, 30, "head at {scale}x");
    }
}

/// A node fading its edges and clipping nothing: its box is the view's, its
/// bands the view's edges'. A copy would carry them, so what it holds repaints.
#[test]
fn a_fade_that_clips_nothing_stays_at_the_view() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        in_a_view(
            "unclipped",
            scale,
            false,
            30,
            6.,
            |_| {
                let mut page = column().w(W).fade_edges(24., 24.);
                for _ in 0..30 {
                    page = page.child(stripes(W, 30.));
                }
                page
            },
            |_| {},
        );
    }
}

/// Layers among the content, across the view's edges, each kind `kind` makes
/// of a group of two boxes overlapping. Moves copy.
fn layers_copy(name: &str, fade: bool, kind: impl Fn(View, usize) -> View) {
    let kind = &kind;
    for scale in SCALES {
        let steps = 24;
        let copies = in_a_view(
            name,
            scale,
            fade,
            steps,
            8.,
            |_| {
                let mut page = column().w(W).gap(14.);
                for i in 0..12 {
                    let group = overlay()
                        .w(120.)
                        .h(50.)
                        .child(div().w(80.).h(40.).bg(rgb(0xcc6644)))
                        .child(
                            div()
                                .absolute()
                                .left(40.)
                                .top(10.)
                                .w(80.)
                                .h(40.)
                                .bg(rgb(0x44aacc)),
                        );
                    page = page.child(kind(group, i));
                }
                page
            },
            |_| {},
        );
        assert_eq!(copies, steps as u64, "{name} at {scale}x");
    }
}

/// A group fading as one.
#[test]
fn group_layers_in_the_content() {
    let _gpu = gpu_fixture();
    layers_copy("group", false, |group, i| {
        group.layer().opacity(0.5).left(10. + i as f32)
    });
    layers_copy("faded group", true, |group, _| group.layer().opacity(0.5));
}

/// Blurred, its blur reaching past its box.
#[test]
fn blurred_layers_in_the_content() {
    let _gpu = gpu_fixture();
    layers_copy("blurred", false, |group, _| group.filter_blur(3.).left(30.));
}

/// Through a shader reading only its own layer, away from any fade: it moves
/// with the content.
#[test]
fn shaded_layers_in_the_content() {
    let _gpu = gpu_fixture();
    let shader = zgx::decoration::FillShader::new(
        "fn layer(input: LayerInput) -> vec4<f32> {
            return vec4(input.color.rgb, input.color.a * (0.5 + input.p.x / 400.));
        }",
        None,
    );
    layers_copy("shaded", false, move |group, _| {
        group.layer_shader(shader.clone())
    });
}

/// Turned.
#[test]
fn turned_layers_in_the_content() {
    let _gpu = gpu_fixture();
    layers_copy("turned", false, |group, _| {
        group.layer().left(20.).rotate(0.2)
    });
    layers_copy("faded turned", true, |group, _| {
        group.layer().left(20.).rotate(0.2)
    });
}

/// A layer whose opacity changes as the view scrolls, its damage both where
/// it was and where the copy puts it.
#[test]
fn a_layer_changing_in_the_content() {
    let _gpu = gpu_fixture();
    for (scale, fade) in SCALES
        .into_iter()
        .flat_map(|scale| [(scale, false), (scale, true)])
    {
        let steps = 24;
        let alpha = std::cell::RefCell::new(None);
        let copies = in_a_view(
            "changing",
            scale,
            fade,
            steps,
            8.,
            |ui| {
                let changing = ui.signal(1_f32);
                let read = changing.clone();
                *alpha.borrow_mut() = Some(changing);
                let mut page = column().w(W).gap(14.);
                for _ in 0..24 {
                    let read = read.clone();
                    page = page.child(
                        div()
                            .w(120.)
                            .h(50.)
                            .bg(rgb(0xcc6644))
                            .layer()
                            .reactive_style(move || Styles::new().opacity(read.get())),
                    );
                }
                page
            },
            |step| {
                if let Some(alpha) = &*alpha.borrow() {
                    alpha.set(1. - step as f32 / 80.);
                }
            },
        );
        assert_eq!(copies, steps as u64, "changing at {scale}x");
    }
}

/// Content that doesn't cover the view, each row opaque under a fade of its
/// own, over a card fading under the view: an opaque row under its own fade
/// shows what lies under it, so the card under it is no less in the way.
#[test]
fn opaque_rows_under_their_own_fade_over_a_changing_card() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let (w, h) = ((W * scale) as u32, (H * scale) as u32);
        let gpu = GpuRenderer::new(w, h).unwrap();
        let mut ui = Ui::new(W, H);
        let offset = ui.signal(0_f32);
        let alpha = ui.signal(1_f32);
        let read = alpha.clone();
        let mut page = column().w(W);
        for _ in 0..20 {
            page = page.child(
                div()
                    .w(W)
                    .h(40.)
                    .overflow_hidden()
                    .fade_edges_x(40., 40.)
                    .child(div().w(W).h(40.).bg(rgb(0x7a5a9a))),
            );
        }
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
                        .bg(rgb(0xd0d040))
                        .reactive_style(move || Styles::new().opacity(read.get())),
                )
                .child(scroll(offset.clone()).w(W).h(H).child(page)),
        );
        drawn_alike(
            gpu,
            "see-through rows",
            scale,
            &mut ui,
            &offset,
            12,
            8.,
            1,
            |step| {
                alpha.set(1. - step as f32 / 20.);
            },
        );
    }
}

/// Both scrolling at once: the outer view and a fading table in it, and then
/// the table alone. Each frame as painted whole.
#[test]
fn a_table_scrolling_inside_a_scrolling_view() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let across = std::cell::RefCell::new(None);
        in_a_view(
            "nested",
            scale,
            true,
            30,
            6.,
            |ui| {
                let inner = ui.signal(0_f32);
                *across.borrow_mut() = Some(inner.clone());
                let mut page = column().w(W).gap(10.);
                for i in 0..12 {
                    let offset = if i < 4 {
                        inner.clone()
                    } else {
                        ui.signal(0_f32)
                    };
                    page = page.child(
                        scroll_x(offset)
                            .w(W)
                            .h(60.)
                            .fade_edges_x(24., 24.)
                            .child(stripes(900., 60.)),
                    );
                }
                page
            },
            |step| {
                if step % 3 == 0
                    && let Some(across) = &*across.borrow()
                {
                    across.update(|x| *x += 10.);
                }
            },
        );
    }
}

/// A backdrop blur in the content reads the view's pixels under it: moves that
/// reach it repaint rather than copy. Most of the view here: nothing to copy.
#[test]
fn a_backdrop_blur_in_the_content_still_blocks() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let copies = in_a_view(
            "backdrop blur",
            scale,
            false,
            12,
            8.,
            |_| {
                let mut page = column().w(W).h(2000.);
                page = page.child(stripes(W, 30.));
                page.child(
                    div()
                        .w(W)
                        .h(600.)
                        .bg(rgba(0xffffff20))
                        .backdrop_blur(4.)
                        .child(stripes(W, 30.)),
                )
            },
            |_| {},
        );
        assert_eq!(copies, 0, "backdrop blur at {scale}x copied");
    }
}

/// A layer blending with what's under it the same, however small.
#[test]
fn a_blend_in_the_content_still_blocks() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let copies = in_a_view(
            "blend",
            scale,
            false,
            12,
            8.,
            |_| {
                column().w(W).child(stripes(W, 40.)).child(
                    div()
                        .w(60.)
                        .h(600.)
                        .bg(rgb(0x808080))
                        .blend(zgx::scene::BlendMode::Multiply),
                )
            },
            |_| {},
        );
        assert_eq!(copies, 0, "blend at {scale}x copied");
    }
}

/// A layer shader cutting its layer at a line in the window: what it draws
/// depends on where the window is, so moves don't copy it. (`zgx_rect`, and a
/// pass's `area`, the same.)
#[test]
fn a_shader_reading_the_window_in_the_content_still_blocks() {
    let _gpu = gpu_fixture();
    let shader = zgx::decoration::FillShader::new(
        "fn layer(input: LayerInput) -> vec4<f32> {
            if zgx_window(input.p).y > 90. { return vec4(0.); }
            return input.color;
        }",
        None,
    );
    for scale in SCALES {
        let copies = in_a_view(
            "window shader",
            scale,
            false,
            12,
            8.,
            |_| {
                let mut page = column().w(W).gap(10.);
                for _ in 0..12 {
                    page = page.child(stripes(W, 30.).layer_shader(shader.clone()));
                }
                page
            },
            |_| {},
        );
        assert_eq!(copies, 0, "window shader at {scale}x copied");
    }
}

/// Boxes stacking in a fading view's bands, and a blurred layer in the same
/// view. A run holding such a layer fades item by item where it paints whole
/// (`band_strips`), and a copy's bands, walked without that layer, would fade
/// as one: the view repaints, and its bands look as they do every other frame.
#[test]
fn stacked_boxes_in_the_bands_with_a_blurred_layer_in_view() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let copies = in_a_view(
            "stacked and blurred",
            scale,
            true,
            20,
            8.,
            |_| {
                let mut page = column().w(W).gap(6.);
                for i in 0..24 {
                    let card = div()
                        .w(W)
                        .h(44.)
                        .bg(rgb(0xe8e0d0))
                        .child(div().w(W - 40.).h(36.).bg(rgb(0x304060)));
                    page = page.child(if i % 4 == 2 {
                        card.filter_blur(3.)
                    } else {
                        card
                    });
                }
                page
            },
            |_| {},
        );
        assert_eq!(copies, 0, "stacked and blurred at {scale}x copied");
    }
}

/// A window-sized view that can't copy goes on drawing to the window alone:
/// only a view that could copy needs the frame kept for the next.
#[test]
fn a_view_that_cant_copy_stays_direct() {
    let _gpu = gpu_fixture();
    let mut gpu = GpuRenderer::new(W as u32, H as u32).unwrap();
    gpu.set_scale_factor(1.);
    // (Off macOS, a see-through window's frames don't draw in a single pass.)
    gpu.set_background(zgx::scene::Color(0, 0, 0, 255));
    gpu.debug_enable_single_pass();
    gpu.debug_allow_scroll_copies();
    let mut ui = Ui::new(W, H);
    let offset = ui.signal(0_f32);
    let mut page = column().w(W);
    for _ in 0..40 {
        page = page.child(stripes(W, 40.));
    }
    // (Bands wider than it ever overflows: cut short, always refused.)
    ui.mount(
        overlay().w(W).h(H).bg(rgb(0x000000)).child(
            scroll(offset.clone())
                .w(W)
                .h(H)
                .bg(rgb(0x101820))
                .fade_edges(4000., 4000.)
                .fade_overflow(true)
                .child(page),
        ),
    );
    let steps = 20;
    for _ in 0..steps {
        offset.update(|o| *o += 8.);
        ui.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        gpu.debug_present_offscreen().unwrap();
    }
    assert_eq!(gpu.debug_scroll_copies(), 0);
    let direct = gpu.debug_direct_frames();
    assert!(direct as usize >= steps - 3, "{direct} of {steps} direct");
}

/// A layer shader over a pass keeping state from frame to frame: it runs each
/// frame its box shows, damaged or not, so copied pixels would hold an old
/// result beside the exposed strip's new one. Moves repaint.
#[test]
fn a_shader_with_a_pass_running_each_frame_still_blocks() {
    let _gpu = gpu_fixture();
    for scale in SCALES {
        let shader = zgx::decoration::FillShader::with_inputs(
            "fn layer(input: LayerInput) -> vec4<f32> {
                return vec4(zgx_count(vec2(0.5)).r / 40.0, input.p.y / 800.0, 0.0, 1.0);
            }",
            None,
            zgx::decoration::ShaderInputs {
                passes: vec![zgx::decoration::ShaderPass {
                    name: "count".into(),
                    source: "fn run(input: PassInput) -> vec4<f32> {
                        return vec4(zgx_count(vec2(0.5)).r + 1.0, 0.0, 0.0, 1.0);
                    }"
                    .into(),
                    size: zgx::decoration::PassSize::Pixels(4, 4),
                    format: zgx::decoration::PassFormat::Rgba16Float,
                    feedback: true,
                    steps: 1,
                    once: false,
                }],
                ..Default::default()
            },
        );
        // (Checked by its refusal: a second renderer's pass would have run
        // more often, on the frames the first has nothing to draw.)
        let (w, h) = ((W * scale) as u32, (H * scale) as u32);
        let mut gpu = GpuRenderer::new(w, h).unwrap();
        gpu.set_scale_factor(scale);
        gpu.debug_allow_scroll_copies();
        let mut ui = Ui::new(W, H);
        let offset = ui.signal(0_f32);
        ui.mount(
            overlay().w(W).h(H).bg(rgb(0x000000)).child(
                scroll(offset.clone()).w(W).h(H).bg(rgb(0x101820)).child(
                    column()
                        .w(W)
                        .child(stripes(W, 30.))
                        .child(div().w(W).h(800.).bg(rgb(0x808080)).layer_shader(shader)),
                ),
            ),
        );
        for step in 0..10 {
            offset.set(step as f32 * 8.);
            ui.prepare_frame();
            let damage = ui.scene.borrow_mut().flush().damage;
            gpu.render(&ui.scene.borrow(), &damage).unwrap();
            gpu.debug_present_offscreen().unwrap();
        }
        let copies = gpu.debug_scroll_copies();
        assert_eq!(copies, 0, "running pass at {scale}x copied");
    }
}

/// A last scroll step keeping its frame for a copy doesn't keep the frames
/// after it, once nothing scrolls: they go direct again.
#[test]
fn a_scroll_ending_leaves_whole_frames_direct() {
    let _gpu = gpu_fixture();
    let mut gpu = GpuRenderer::new(W as u32, H as u32).unwrap();
    gpu.set_scale_factor(1.);
    // (Off macOS, a see-through window's frames don't draw in a single pass.)
    gpu.set_background(zgx::scene::Color(0, 0, 0, 255));
    gpu.debug_enable_single_pass();
    gpu.debug_allow_scroll_copies();
    let mut ui = Ui::new(W, H);
    let offset = ui.signal(0_f32);
    let shade = ui.signal(0_u32);
    let read = shade.clone();
    let mut page = column().w(W);
    for _ in 0..40 {
        page = page.child(stripes(W, 40.));
    }
    ui.mount(
        overlay().w(W).h(H).bg(rgb(0x000000)).child(
            scroll(offset.clone())
                .w(W)
                .h(H)
                .reactive_style(move || Styles::new().bg(rgb(0x101820 + read.get())))
                .child(page),
        ),
    );
    let frame = |ui: &mut Ui, gpu: &mut GpuRenderer| {
        ui.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        gpu.debug_present_offscreen().unwrap();
    };
    // Whole frames, the last direct: the next pixels are new.
    for step in 1..=4 {
        shade.set(step);
        frame(&mut ui, &mut gpu);
    }
    let before = gpu.debug_direct_frames();
    assert!(before > 0, "no frame went direct");
    offset.set(8.);
    frame(&mut ui, &mut gpu);
    let before = gpu.debug_direct_frames().max(before);
    let steps = 10;
    for step in 0..steps {
        shade.set(5 + step);
        frame(&mut ui, &mut gpu);
    }
    let direct = gpu.debug_direct_frames() - before;
    assert!(direct >= u64::from(steps) - 2, "{direct} of {steps} direct");
}

/// A dithered picture among a fading table's rows: its dots lie on the screen's
/// grid as it slides under them, which a copy would carry with it.
#[test]
fn a_dithered_picture_in_the_content_still_blocks() {
    let _gpu = gpu_fixture();
    let flat = zgx::image::ImageData::new(8, 8, [200u8, 100, 50, 255].repeat(64)).unwrap();
    let dithered = std::sync::Arc::new(flat.dithered(2.));
    for scale in SCALES {
        let copies = in_a_view(
            "dithered",
            scale,
            false,
            10,
            // (Not a whole number of its pattern's repeats.)
            6.,
            |ui| {
                column()
                    .w(W)
                    .child(
                        scroll_x(ui.signal(0_f32))
                            .w(W)
                            .h(40.)
                            .fade_edges_x(0., 24.)
                            .child(stripes(600., 40.)),
                    )
                    .child(image("dots", dithered.clone()).w(W).h(600.))
            },
            |_| {},
        );
        assert_eq!(copies, 0, "dithered at {scale}x copied");
    }
}

/// Text that gets narrower without scrolling: paragraphs rewrapped as their
/// column narrows (a sidebar opening beside a transcript), a line shortened
/// in place, and a word overflowing its box, then wrapped. Every frame drawn
/// from its damage, a second renderer's whole, by the byte: what a line drew
/// past its new box goes too.
fn narrowing(
    name: &str,
    scale: f32,
    fit: bool,
    change: impl Fn(usize, &Signal<f32>, &Signal<usize>),
) {
    narrowing_in(name, scale, fit, false, false, false, change);
}

/// Line `i` of a story, shortened while `version` isn't 0: plain text, or
/// rich text of spans (a reply's markdown) if `rich`.
fn story_line(line: &'static str, i: usize, version: Signal<usize>, rich: bool) -> View {
    let shown = move || match version.get() {
        0 => line.to_string(),
        // Shorter: the line ends well before it did.
        _ => line.chars().take(line.len() / (2 + i % 2)).collect(),
    };
    match rich {
        false => text_signal(shown).text_size(14.).text_color(rgb(0xe8e8f0)),
        true => rich_text_signal(move || {
            let words = shown();
            let cut = (words.char_indices())
                .nth(words.chars().count() / 3)
                .map_or(words.len(), |(at, _)| at);
            let (head, tail) = words.split_at(cut);
            vec![
                text_span(head.to_string())
                    .text_size(14.)
                    .text_color(rgb(0xe8e8f0)),
                text_span(tail.to_string())
                    .text_size(14.)
                    .text_color(rgb(0xc8d0f0)),
            ]
        }),
    }
}

/// `narrowing`, its lines rows of a measured list if `listed`, rich if
/// `rich`.
fn narrowing_in(
    name: &str,
    scale: f32,
    fit: bool,
    listed: bool,
    rich: bool,
    long: bool,
    change: impl Fn(usize, &Signal<f32>, &Signal<usize>),
) {
    const LINES: [&str; 4] = [
        "She came in and sat down. Arthur poured her tea, and Daniel went into the garden.",
        "“He was complaining about mine.”",
        "Supercalifragilisticexpialidociously-long-unbroken-word-overflowing",
        "“What?” she said, and then nothing more for a while, looking at the window.",
    ];
    let (w, h) = ((W * scale) as u32, (H * scale) as u32);
    let mut gpu = GpuRenderer::new(w, h).unwrap();
    let mut whole = GpuRenderer::new(w, h).unwrap();
    // A frame drawn whole from a kept frame, patched: a window's after most
    // of it changed.
    let mut patched_whole = GpuRenderer::new(w, h).unwrap();
    let mut ui = Ui::new(W, H);
    ui.scene.borrow_mut().set_pixel_scale(scale);
    for gpu in [&mut gpu, &mut whole, &mut patched_whole] {
        gpu.set_scale_factor(scale);
        gpu.debug_enable_single_pass();
    }
    // (The reference: drawn afresh each frame, through no kept frame.)
    whole.debug_never_keep();
    gpu.install_text(&mut ui.scene.borrow_mut());
    let width = ui.signal(W - 20.);
    let version = ui.signal(0_usize);
    let mut page = column().p(10.).gap(6.).items_start().reactive_style({
        let width = width.clone();
        move || Styles::new().w(width.get())
    });
    // Past `COLUMN_KEPT_FROM` (64) lines when `long`: the column keeps its
    // children's ink as one union.
    let copies = if long { 20 } else { 1 };
    for (i, line) in LINES.iter().cycle().take(LINES.len() * copies).enumerate() {
        let version = version.clone();
        let text = story_line(line, i, version, rich);
        // As wide as the column, or as its own lines (`fit`).
        page = page.child(if fit { text } else { text.w_full() });
    }
    let page = if listed {
        // The same lines, each a row of a measured list as wide as the page.
        let (version, width) = (version.clone(), width.clone());
        let heights = VariableHeights::new(&ui.runtime, 0, 30.).anchor_end();
        let offset = ui.signal(0_f32);
        measured_rows(
            offset,
            heights,
            2,
            || (0..LINES.len()).collect(),
            |_, _| 30.,
            move |_, i, _| {
                let version = version.clone();
                let line = LINES[i];
                let text = story_line(line, i, version, rich);
                column()
                    .w_full()
                    .pb(6.)
                    .items_start()
                    .child(if fit { text } else { text.w_full() })
            },
        )
        .h(H - 20.)
        .m(10.)
        .reactive_style(move || Styles::new().w(width.get()))
    } else {
        page
    };
    ui.mount(
        overlay()
            .w(W)
            .h(H)
            .bg(rgb(0x000000))
            .child(div().w(W).h(H).bg(rgb(0x14181f)).child(page)),
    );
    let full = Rect::new(0., 0., W, H);
    let mut patched = 0;
    let mut frame = |ui: &mut Ui, label: &str| {
        ui.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        patched += gpu
            .render(&ui.scene.borrow(), &damage)
            .unwrap()
            .patched_items;
        let partial = gpu.debug_present_offscreen().unwrap();
        whole.render(&ui.scene.borrow(), &[full]).unwrap();
        let fresh = whole.debug_present_offscreen().unwrap();
        patched += (patched_whole.render(&ui.scene.borrow(), &[full]).unwrap()).patched_items;
        let patched_frame = patched_whole.debug_present_offscreen().unwrap();
        let wrong = patched_frame
            .chunks(4)
            .zip(fresh.chunks(4))
            .position(|(a, b)| a != b);
        let w_ = w as usize;
        assert!(
            wrong.is_none(),
            "{name} at {scale}x, fit {fit}, listed {listed}, rich {rich}, long {long}, {label}: \
             drawn whole from its kept frame, a stale pixel at points {:?}",
            wrong.map(|p| ((p % w_) as f32 / scale, (p / w_) as f32 / scale)),
        );
        let w = w as usize;
        let first = partial
            .chunks(4)
            .zip(fresh.chunks(4))
            .position(|(a, b)| a != b);
        let count = partial
            .chunks(4)
            .zip(fresh.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            first.is_none(),
            "{name} at {scale}x, fit {fit}, listed {listed}, rich {rich}, long {long}, {label}: {count} stale pixels, the first at points {:?} \
             (damage {damage:?})",
            first.map(|p| ((p % w) as f32 / scale, (p / w) as f32 / scale)),
        );
    };
    frame(&mut ui, "first");
    frame(&mut ui, "settled");
    for step in 0..12 {
        change(step, &width, &version);
        frame(&mut ui, &format!("step {step}"));
    }
    let _ = frame;
    // (Lines shortened in place patch a kept frame: else nothing here
    // checked those.)
    if name.contains("shortened") && !name.contains("narrowing") && !rich {
        assert!(patched > 0, "{name} at {scale}x: no frame was patched");
    }
}

#[test]
fn text_getting_narrower_leaves_nothing_where_it_was() {
    let _gpu = gpu_fixture();
    for scale in [2., 1.] {
        for fit in [true, false] {
            // A few points at a time (a scrollbar's room), then many.
            narrowing("a narrowing column", scale, fit, |step, width, _| {
                width.set(W - 20. - step as f32 * if step < 6 { 4. } else { 13. });
            });
            narrowing("lines shortened", scale, fit, |step, _, version| {
                version.set(step % 2);
            });
            narrowing(
                "narrowing and shortened",
                scale,
                fit,
                |step, width, version| {
                    width.set(W - 20. - (step % 4) as f32 * 23.);
                    version.set(step / 3 % 2);
                },
            );
            narrowing_in(
                "a narrowing list",
                scale,
                fit,
                true,
                false,
                false,
                |step, width, _| {
                    width.set(W - 20. - step as f32 * if step < 6 { 4. } else { 13. });
                },
            );
            narrowing_in(
                "a list's lines shortened",
                scale,
                fit,
                true,
                false,
                false,
                |step, _, version| {
                    version.set(step % 2);
                },
            );
            for listed in [false, true] {
                narrowing_in(
                    "rich text narrowing",
                    scale,
                    fit,
                    listed,
                    true,
                    false,
                    |step, width, _| {
                        width.set(W - 20. - step as f32 * if step < 6 { 4. } else { 13. });
                    },
                );
                narrowing_in(
                    "rich text shortened",
                    scale,
                    fit,
                    listed,
                    true,
                    false,
                    |step, _, version| {
                        version.set(step % 2);
                    },
                );
            }
            for rich in [false, true] {
                narrowing_in(
                    "a long column narrowing",
                    scale,
                    fit,
                    false,
                    rich,
                    true,
                    |step, width, _| {
                        width.set(W - 20. - step as f32 * if step < 6 { 4. } else { 13. });
                    },
                );
                narrowing_in(
                    "a long column's lines shortened",
                    scale,
                    fit,
                    false,
                    rich,
                    true,
                    |step, _, version| {
                        version.set(step % 2);
                    },
                );
            }
        }
    }
}
