//! Runs fading in (`Scene::show_fades`) draw on the GPU at their time: each
//! frame's pixels are, byte for byte, those of the paragraph recoloured at
//! that time and drawn as before (its runs' alphas as `TextFade::alpha` has
//! them, the way a fade drew a frame at a time), on the wgpu path and with
//! Metal directly. A frame whose only change is the fades' time draws the
//! last one again with that time (`fade_replays`); one with anything else
//! changed draws as usual.
use std::sync::Arc;

use zgx::{
    compose::{TextFade, prelude::*},
    reactive::Signal,
    rich_text::RichText,
    scene::{Color, NodeId, NodeKind, Rect},
    theme::{LIGHT, tok},
    widgets::Ui,
};
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const W: f32 = 320.;
const H: f32 = 160.;

/// A fade starting at `start` (ms), `ms` long, easing out by 1.6.
fn fade(start: f64, ms: f32) -> TextFade {
    TextFade {
        start,
        ms,
        power: 1.6,
    }
}

/// The first paragraph under `id`.
fn paragraph(ui: &Ui, id: NodeId) -> Option<NodeId> {
    let scene = ui.scene.borrow();
    if let NodeKind::RichText { .. } = scene.kind(id) {
        return Some(id);
    }
    let children = scene.children(id).to_vec();
    drop(scene);
    children.into_iter().find_map(|child| paragraph(ui, child))
}

/// Its text, as its kind holds it.
fn text_of(ui: &Ui, node: NodeId) -> Arc<RichText> {
    match ui.scene.borrow().kind(node) {
        NodeKind::RichText { text } => text.clone(),
        _ => panic!("not a paragraph"),
    }
}

/// The index of the run of `text` reading `words`.
fn run(text: &RichText, words: &str) -> usize {
    text.runs()
        .iter()
        .position(|run| &text.text()[run.range.clone()] == words)
        .unwrap_or_else(|| panic!("no run {words:?} in {:?}", text.text()))
}

/// A window drawn twice: `gpu` with the fades on the GPU (`show_fades`),
/// and `today` from the same view, its paragraph recoloured at each time and
/// drawn as before; each frame's pixels compared.
struct Pair {
    gpu: GpuRenderer,
    ui: Ui,
    today: GpuRenderer,
    reference: Ui,
    /// The reference's paragraph as its view made it, and the recoloured
    /// one it shows (by address): a new one made since is its view's again.
    base: Option<(usize, Arc<RichText>)>,
    /// Each one's step (`step`) and scroll offset (`scroll`), if its view
    /// takes them.
    steps: Vec<(Signal<u32>, Signal<f32>)>,
    /// Its targets' width, in pixels.
    width: usize,
    direct: bool,
}

impl Pair {
    fn new(direct: bool, view: impl Fn() -> View) -> Self {
        Self::stepped(direct, |_| view())
    }
    /// With a view of a step each frame can move on (`step`).
    fn stepped(direct: bool, view: impl Fn(Signal<u32>) -> View) -> Self {
        Self::scrolled(direct, move |step, _| view(step))
    }
    /// With a view of a step and a scroll offset (`scroll`).
    fn scrolled(direct: bool, view: impl Fn(Signal<u32>, Signal<f32>) -> View) -> Self {
        let mut steps = Vec::new();
        let mut make = || {
            let mut gpu = GpuRenderer::new(W as u32, H as u32).unwrap();
            gpu.set_direct_metal(direct);
            let mut ui = Ui::new(W, H);
            gpu.install_text(&mut ui.scene.borrow_mut());
            ui.refresh_text_geometry();
            let (step, offset) = (ui.signal(0), ui.signal(0.));
            steps.push((step.clone(), offset.clone()));
            ui.mount(view(step, offset));
            (gpu, ui)
        };
        let (gpu, ui) = make();
        let (today, reference) = make();
        Self {
            gpu,
            ui,
            today,
            reference,
            base: None,
            steps,
            width: W as usize,
            direct,
        }
    }
    /// Its view's step, in both, as of the next frame.
    fn step(&self, step: u32) {
        for (signal, _) in &self.steps {
            signal.set(step);
        }
    }
    /// Its view's scroll offset, in both, as of the next frame.
    fn scroll(&self, offset: f32) {
        for (_, signal) in &self.steps {
            signal.set(offset);
        }
    }
    /// Both windows (and their targets) resized to `width` by `height`
    /// points at `scale` device pixels each.
    fn resize(&mut self, width: f32, height: f32, scale: f32) {
        self.width = (width * scale) as usize;
        for (gpu, ui) in [
            (&mut self.gpu, &self.ui),
            (&mut self.today, &self.reference),
        ] {
            gpu.set_scale_factor(scale);
            gpu.resize((width * scale) as u32, (height * scale) as u32);
            ui.scene.borrow_mut().resize(width, height);
        }
    }
    /// The paragraph's node in each.
    fn nodes(&self) -> (NodeId, NodeId) {
        let root = |ui: &Ui| ui.scene.borrow().root();
        (
            paragraph(&self.ui, root(&self.ui)).expect("a paragraph"),
            paragraph(&self.reference, root(&self.reference)).expect("a paragraph"),
        )
    }
    /// A frame with `fades` (runs by their words) showing at `now`: drawn
    /// both ways from each one's damage, compared; whether `gpu` drew it
    /// again from the last (`fade_replays`).
    fn frame(&mut self, fades: &[(&str, TextFade)], now: f64, label: &str) -> bool {
        self.ui.prepare_frame();
        self.reference.prepare_frame();
        let (node, reference) = self.nodes();
        let text = text_of(&self.ui, node);
        let shown: Vec<(usize, TextFade)> = fades
            .iter()
            .map(|(words, fade)| (run(&text, words), *fade))
            .collect();
        self.ui
            .scene
            .borrow_mut()
            .show_fades(node, &text, &shown, now);
        // As a fade drew before: the paragraph recoloured at `now`.
        let base = text_of(&self.reference, reference);
        let base = match self.base.take() {
            Some(kept) if kept.0 == Arc::as_ptr(&base) as usize => kept.1,
            _ => base,
        };
        let recoloured =
            Arc::new(
                base.recoloured(|i, color| match shown.iter().find(|(at, _)| *at == i) {
                    Some((_, fade)) => Color(color.0, color.1, color.2, fade.alpha(color.3, now)),
                    None => color,
                }),
            );
        self.reference
            .scene
            .borrow_mut()
            .set_kind(reference, NodeKind::RichText { text: recoloured });
        // (What it shows now: one alike in colour leaves the last.)
        let shows = Arc::as_ptr(&text_of(&self.reference, reference)) as usize;
        self.base = Some((shows, base));
        let replays = self.gpu.fade_replays();
        let damage = self.ui.scene.borrow_mut().flush().damage;
        self.gpu.render(&self.ui.scene.borrow(), &damage).unwrap();
        let damage = self.reference.scene.borrow_mut().flush().damage;
        self.today
            .render(&self.reference.scene.borrow(), &damage)
            .unwrap();
        let (pixels, expected) = (self.gpu.readback().unwrap(), self.today.readback().unwrap());
        let wrong = pixels
            .chunks(4)
            .zip(expected.chunks(4))
            .position(|(a, b)| a != b);
        let width = self.width;
        assert!(
            pixels.len() == expected.len() && wrong.is_none(),
            "direct {}, {label} (at {now}): pixel {:?} is {:?}, drawn as before {:?}",
            self.direct,
            wrong.map(|p| (p % width, p / width)),
            wrong.map(|p| &pixels[p * 4..p * 4 + 4]),
            wrong.map(|p| &expected[p * 4..p * 4 + 4]),
        );
        self.gpu.fade_replays() > replays
    }
}

/// White text, half clear, on a dark panel: `before`, then `fading`.
fn plain(before: &'static str, fading: &'static str) -> impl Fn() -> View {
    move || {
        column()
            .size(W, H)
            .p(12.)
            .bg(rgb(0x101820))
            .text_size(22.)
            .child(
                rich_text()
                    .child(text_span(before).text_color(Color(255, 255, 255, 200)))
                    .child(text_span(fading).text_color(Color(255, 220, 160, 255))),
            )
    }
}

/// Times through a fade from `start`, `ms` long: its start, odd points
/// along it (not on whole milliseconds), its end exactly, and past it.
fn times(start: f64, ms: f64) -> Vec<f64> {
    let mut times: Vec<f64> = (0..=12).map(|k| start + ms * k as f64 / 12.).collect();
    times.extend([0.137, 0.3333, 0.618, 0.9999].map(|t| start + ms * t + 0.000123));
    times.sort_by(f64::total_cmp);
    times.extend([start + ms, start + ms + 0.5, start + ms + 40.]);
    times
}

#[test]
fn a_run_fading_draws_as_it_recoloured_at_each_time() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        let mut pair = Pair::new(direct, plain("Shown already. ", "Fading in now."));
        let fading = fade(1_000., 300.);
        pair.frame(&[], 900., "before it fades");
        let mut replays = 0;
        for (k, now) in times(1_000., 300.).into_iter().enumerate() {
            replays +=
                usize::from(pair.frame(&[("Fading in now.", fading)], now, &format!("t{k}")));
        }
        assert!(
            replays > 10,
            "direct {direct}: frames of the fade alone draw again: {replays}"
        );
    }
}

#[test]
fn a_fade_ends_as_its_text_drawn_plain() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        let mut pair = Pair::new(direct, plain("Shown already. ", "Fading in now."));
        let fading = fade(1_000., 300.);
        pair.frame(&[("Fading in now.", fading)], 1_150., "mid-way");
        pair.frame(&[("Fading in now.", fading)], 1_300., "at its end exactly");
        pair.frame(&[("Fading in now.", fading)], 1_301., "after its end");
        // Done: no fades; the text as it is, as a paragraph never faded.
        pair.frame(&[], 1_320., "settled");
        let mut fresh = GpuRenderer::new(W as u32, H as u32).unwrap();
        fresh.set_direct_metal(direct);
        fresh
            .render(&pair.ui.scene.borrow(), &[Rect::new(0., 0., W, H)])
            .unwrap();
        assert!(
            fresh.readback().unwrap() == pair.gpu.readback().unwrap(),
            "direct {direct}: settled, it draws as plain text drawn afresh"
        );
        assert!(!pair.ui.scene.borrow().has_text_fades());
    }
}

/// A reply's paragraph growing as its words fade in: a chunk appended mid-
/// fade (`step` 1), and the fading span cut again into pieces of its own
/// (`step` 2), each fading on as it was, the chunk from when it came.
fn growing(step: Signal<u32>) -> View {
    let span = |words: &'static str, color: Color| text_span(words).text_color(color);
    column()
        .size(W, H)
        .p(12.)
        .bg(rgb(0x101820))
        .text_size(22.)
        .child(rich_text_signal(move || {
            let shown = span("Shown already. ", Color(255, 255, 255, 200));
            let warm = Color(255, 220, 160, 255);
            match step.get() {
                0 => vec![shown, span("Fading in", warm)],
                1 => vec![
                    shown,
                    span("Fading in", warm),
                    span(" and more.", Color(200, 230, 255, 230)),
                ],
                _ => vec![
                    shown,
                    span("Fading", warm),
                    span(" in", Color(255, 221, 160, 255)),
                    span(" and more.", Color(200, 230, 255, 230)),
                ],
            }
        }))
}

#[test]
fn a_chunk_mid_fade_draws_as_its_runs_recoloured() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        let mut pair = Pair::stepped(direct, growing);
        let (first, second) = (fade(1_000., 300.), fade(1_100., 300.));
        for now in [1_000., 1_030., 1_060.] {
            pair.frame(&[("Fading in", first)], now, "the first words");
        }
        assert!(pair.frame(&[("Fading in", first)], 1_090., "the first words, alone"));
        // A chunk: new text, drawn as usual, not again from the last.
        pair.step(1);
        let both = [("Fading in", first), (" and more.", second)];
        assert!(
            !pair.frame(&both, 1_100., "a chunk"),
            "direct {direct}: a chunk's frame draws its new text"
        );
        for now in [1_120.5, 1_180., 1_299.99, 1_300.] {
            pair.frame(&both, now, "both fading");
        }
        // Cut again: the first fade's run in two pieces, each fading as it did.
        pair.step(2);
        let cut = [("Fading", first), (" in", first), (" and more.", second)];
        assert!(!pair.frame(&cut, 1_310., "cut again"));
        let mut replays = 0;
        for now in [1_320., 1_350.25, 1_399., 1_400., 1_401.] {
            replays += usize::from(pair.frame(&cut, now, "cut, fading"));
        }
        assert!(replays > 0, "direct {direct}: its fades alone draw again");
    }
}

/// `plain`'s paragraph, its colours and panel the theme's.
fn themed() -> View {
    column()
        .size(W, H)
        .p(12.)
        .bg(tok::SURFACE_RAISED)
        .text_size(22.)
        .child(
            rich_text()
                .child(text_span("Shown already. ").text_color(tok::TEXT_SECONDARY))
                .child(text_span("Fading in now.").text_color(tok::TEXT_PRIMARY)),
        )
}

#[test]
fn a_theme_changed_mid_fade_fades_on_in_its_colours() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        let mut pair = Pair::new(direct, themed);
        let fading = fade(1_000., 300.);
        for now in [1_000., 1_040., 1_080.] {
            pair.frame(&[("Fading in now.", fading)], now, "dark");
        }
        assert!(pair.frame(&[("Fading in now.", fading)], 1_100., "dark, alone"));
        pair.ui.theme().set(LIGHT).unwrap();
        pair.reference.theme().set(LIGHT).unwrap();
        assert!(
            !pair.frame(&[("Fading in now.", fading)], 1_120., "the theme changed"),
            "direct {direct}: a theme's frame draws its colours"
        );
        for now in [1_150., 1_200.5, 1_300., 1_330.] {
            pair.frame(&[("Fading in now.", fading)], now, "light");
        }
    }
}

#[test]
fn with_reduced_motion_a_run_shows_at_once() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        let mut pair = Pair::new(direct, plain("Shown already. ", "Fading in now."));
        // No fade at all, or one taking no time: its colour, as plain text.
        pair.frame(&[], 1_000., "no fade");
        pair.frame(
            &[("Fading in now.", fade(1_000., 0.))],
            1_000.,
            "a fade of 0 ms",
        );
        pair.frame(
            &[("Fading in now.", fade(1_000., 0.))],
            1_016.,
            "a fade of 0 ms, on",
        );
        let mut plain_gpu = GpuRenderer::new(W as u32, H as u32).unwrap();
        plain_gpu.set_direct_metal(direct);
        plain_gpu
            .render(&pair.reference.scene.borrow(), &[Rect::new(0., 0., W, H)])
            .unwrap();
        assert!(plain_gpu.readback().unwrap() == pair.gpu.readback().unwrap());
    }
}

#[test]
fn runs_fading_from_their_own_starts_each_draw_at_theirs() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        let mut pair = Pair::new(direct, || {
            column()
                .size(W, H)
                .p(12.)
                .bg(rgb(0x101820))
                .text_size(22.)
                .child(
                    rich_text()
                        .child(text_span("One, ").text_color(Color(255, 255, 255, 255)))
                        .child(text_span("two, ").text_color(Color(255, 200, 120, 255)))
                        .child(text_span("three, ").text_color(Color(160, 220, 255, 180)))
                        .child(text_span("four.").text_color(Color(255, 255, 255, 255))),
                )
        });
        let fades = [
            ("two, ", fade(1_000., 300.)),
            ("three, ", fade(1_060., 250.)),
            ("four.", fade(1_120., 400.)),
        ];
        let mut replays = 0;
        for now in times(1_000., 520.) {
            replays += usize::from(pair.frame(&fades, now, "staggered"));
        }
        assert!(replays > 10, "direct {direct}: {replays} drawn again");
    }
}

/// `plain`'s paragraph in a box fading its edges (`fade_edges`) where the
/// text reaches its bottom band, over a card it fades with (`card`: the
/// band then draws as one, bands.rs) or straight on the panel.
fn banded(card: bool) -> impl Fn() -> View {
    move || {
        let words = rich_text()
            .child(text_span("Shown already, and on. ").text_color(Color(255, 255, 255, 200)))
            .child(text_span("Fading in under the band.").text_color(Color(255, 220, 160, 255)));
        let words = match card {
            true => div().bg(rgb(0x384858)).rounded(6.).p(4.).child(words),
            false => div().child(words),
        };
        column()
            .size(W, H)
            .p(12.)
            .bg(rgb(0x101820))
            .text_size(22.)
            .child(
                column()
                    .w(280.)
                    .h(70.)
                    .overflow_hidden()
                    .fade_edges(0., 40.)
                    .child(words),
            )
    }
}

#[test]
fn text_under_an_edge_fade_draws_as_it_recoloured() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        for card in [false, true] {
            let mut pair = Pair::new(direct, banded(card));
            let fading = fade(1_000., 300.);
            let mut replays = 0;
            for now in times(1_000., 300.) {
                replays += usize::from(pair.frame(
                    &[("Fading in under the band.", fading)],
                    now,
                    &format!("card {card}"),
                ));
            }
            // In its fade's region it draws again too; through a band drawn
            // as one, as usual.
            if !card {
                assert!(replays > 10, "direct {direct}: {replays} drawn again");
            }
        }
    }
}

/// The frames of `plain`'s fade up to 1100 ms, the last drawn again.
fn fading_alone(pair: &mut Pair, words: &str) -> TextFade {
    let fading = fade(1_000., 300.);
    for now in [1_000., 1_040., 1_080.] {
        pair.frame(&[(words, fading)], now, "fading");
    }
    assert!(
        pair.frame(&[(words, fading)], 1_100., "fading, alone"),
        "direct {}: a frame of its fade alone draws again",
        pair.direct
    );
    fading
}

#[test]
fn a_frame_drawing_damage_elsewhere_draws_as_usual() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        let mut pair = Pair::stepped(direct, |step| {
            column()
                .size(W, H)
                .p(12.)
                .bg(rgb(0x101820))
                .text_size(22.)
                .child(
                    rich_text()
                        .child(text_span("Shown already. ").text_color(Color(255, 255, 255, 200)))
                        .child(text_span("Fading in now.").text_color(Color(255, 220, 160, 255))),
                )
                .child(
                    text_signal(move || format!("{} done", step.get()))
                        .text_size(12.)
                        .text_color(rgb(0x80c0ff)),
                )
        });
        let fading = fading_alone(&mut pair, "Fading in now.");
        pair.step(1);
        assert!(
            !pair.frame(&[("Fading in now.", fading)], 1_120., "a box below changed"),
            "direct {direct}: damage past the fade's draws as usual"
        );
        assert!(pair.frame(&[("Fading in now.", fading)], 1_140., "fading again"));
    }
}

#[test]
fn a_frame_scrolled_draws_as_usual() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        let mut pair = Pair::scrolled(direct, |_, offset| {
            column().size(W, H).bg(rgb(0x101820)).text_size(22.).child(
                scroll(offset)
                    .h(H)
                    .w(W)
                    .child(div().h(40.).w(W).bg(rgb(0x203040)))
                    .child(
                        rich_text()
                            .child(
                                text_span("Shown already. ").text_color(Color(255, 255, 255, 200)),
                            )
                            .child(
                                text_span("Fading in now.").text_color(Color(255, 220, 160, 255)),
                            ),
                    )
                    .child(div().h(400.).w(W).bg(rgb(0x182028))),
            )
        });
        pair.gpu.debug_allow_scroll_copies();
        pair.today.debug_allow_scroll_copies();
        let fading = fading_alone(&mut pair, "Fading in now.");
        pair.scroll(12.);
        assert!(
            !pair.frame(&[("Fading in now.", fading)], 1_120., "scrolled"),
            "direct {direct}: a scroll's frame draws as usual"
        );
        // (A frame its copy drew isn't kept: the next is drawn, then kept.)
        pair.frame(&[("Fading in now.", fading)], 1_140., "fading after");
        assert!(pair.frame(&[("Fading in now.", fading)], 1_160., "fading again"));
    }
}

#[test]
fn a_frame_of_another_size_or_scale_draws_as_usual() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        let mut pair = Pair::new(direct, plain("Shown already. ", "Fading in now."));
        let fading = fading_alone(&mut pair, "Fading in now.");
        pair.resize(W, H + 20., 1.);
        assert!(
            !pair.frame(&[("Fading in now.", fading)], 1_120., "resized"),
            "direct {direct}: a resized target draws as usual"
        );
        assert!(pair.frame(&[("Fading in now.", fading)], 1_140., "fading again"));
        pair.resize(W, H + 20., 2.);
        assert!(
            !pair.frame(&[("Fading in now.", fading)], 1_160., "at 2x"),
            "direct {direct}: a scale's change draws as usual"
        );
        assert!(pair.frame(&[("Fading in now.", fading)], 1_180., "fading again at 2x"));
    }
}

#[test]
fn a_frame_changing_its_background_draws_as_usual() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        let mut pair = Pair::new(direct, plain("Shown already. ", "Fading in now."));
        let fading = fading_alone(&mut pair, "Fading in now.");
        // The window's own background (a theme's), the scene as it was.
        pair.gpu.set_background(Color(40, 10, 10, 255));
        pair.today.set_background(Color(40, 10, 10, 255));
        assert!(
            !pair.frame(&[("Fading in now.", fading)], 1_120., "another background"),
            "direct {direct}: another background draws as usual"
        );
    }
}

#[test]
fn text_under_a_backdrop_blur_draws_as_usual() {
    let _gpu_fixture = gpu_fixture();
    for direct in [true, false] {
        let mut pair = Pair::new(direct, || {
            column().size(W, H).bg(rgb(0x101820)).text_size(22.).child(
                overlay()
                    .size(W, 80.)
                    .child(
                        rich_text()
                            .child(
                                text_span("Shown already. ").text_color(Color(255, 255, 255, 200)),
                            )
                            .child(
                                text_span("Fading in now.").text_color(Color(255, 220, 160, 255)),
                            ),
                    )
                    .child(
                        row().child(div().size(100., 60.)).child(
                            div()
                                .size(120., 60.)
                                .bg(Color(255, 255, 255, 40))
                                .backdrop_blur(8.),
                        ),
                    ),
            )
        });
        let fading = fade(1_000., 300.);
        let mut replays = 0;
        for now in times(1_000., 300.) {
            replays += usize::from(pair.frame(&[("Fading in now.", fading)], now, "under glass"));
        }
        assert_eq!(replays, 0, "direct {direct}: a blur over it blurs it again");
    }
}
