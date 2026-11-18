//! Text services without a GPU lay text out exactly as a renderer's do.
use zgx::{compose::prelude::*, scene::Rect, widgets::Ui};
use zgx_gpu::{GpuRenderer, TextServices};

fn laid_out(install: impl FnOnce(&mut zgx::scene::Scene)) -> Vec<Rect> {
    let mut ui = Ui::new(320., 240.);
    install(&mut ui.scene.borrow_mut());
    let view = ui.mount(
        column().w(320.).text_size(15.).children([
            text("A wrapped paragraph long enough to take a second and a third line")
                .w(120.)
                .text_wrap(true)
                .id("paragraph"),
            text("short").id("short"),
            text("fjgqy WAVE ÅÄÖ").italic(true).id("italic"),
        ]),
    );
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    ["paragraph", "short", "italic"]
        .map(|id| scene.layout_bounds(view.find(id).expect(id)))
        .to_vec()
}

#[test]
fn text_services_measure_as_a_renderer_does() {
    let services = TextServices::new();
    let without = laid_out(|scene| services.install(scene));
    // Wrapped into lines, and measured: not the fallback's fixed advances.
    assert!(without[0].height > without[1].height * 2., "{without:?}");
    let Ok(gpu) = GpuRenderer::new(320, 240) else {
        eprintln!("no GPU adapter: compared with nothing");
        return;
    };
    assert_eq!(without, laid_out(|scene| gpu.install_text(scene)));
}

/// The widths of a word in a row, as a text and as a checkbox's label (each
/// as wide as it measures), in a scene `install` gave text services or not.
fn row_widths(install: impl FnOnce(&mut zgx::scene::Scene)) -> [f32; 2] {
    let mut ui = Ui::new(200., 60.);
    install(&mut ui.scene.borrow_mut());
    let on = ui.signal(true);
    let view = ui.mount(column().w(200.).text_size(12.).children([
        row().child(text("Lamps").id("text")),
        row().child(checkbox("Lamps", on).id("checkbox")),
    ]));
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    ["text", "checkbox"].map(|id| scene.layout_bounds(view.find(id).expect(id)).width)
}

/// A scene its host installed no text services on, drawn by a renderer, is
/// measured by the renderer's text: a row's word as wide as the glyphs it
/// draws, not zgx's estimate, which laid a checkbox's "Lamps" out narrower
/// than them and its last glyph was cut. Once the renderer is gone, a scene
/// is left with the estimate again.
#[test]
fn a_scene_drawn_without_text_services_is_measured_by_its_renderer() {
    let Ok(gpu) = GpuRenderer::new(200, 60) else {
        eprintln!("no GPU adapter: compared with nothing");
        return;
    };
    let installed = row_widths(|scene| gpu.install_text(scene));
    assert_eq!(row_widths(|_| {}), installed);
    drop(gpu);
    let estimated = row_widths(|_| {});
    assert_ne!(
        estimated, installed,
        "the estimate measures as the renderer does"
    );
}

/// Two renderers on contexts of their own, on one thread: a scene with no
/// text services takes the last one's and keeps them, drawn by both each
/// frame (a reference renderer's): never traded back and forth, its text
/// laid out again each time. Its host's own installing leaves it none taken.
#[test]
fn a_scene_keeps_the_text_services_it_took_whichever_renderer_draws_it() {
    let (Ok(mut drawing), Ok(mut last)) = (GpuRenderer::new(200, 60), GpuRenderer::new(200, 60))
    else {
        eprintln!("no GPU adapter: compared with nothing");
        return;
    };
    let same = |a: &std::rc::Rc<zgx::scene::DefaultTextServices>,
                b: &std::rc::Rc<zgx::scene::DefaultTextServices>| {
        std::ptr::addr_eq(std::rc::Rc::as_ptr(a), std::rc::Rc::as_ptr(b))
    };
    let mut ui = Ui::new(200., 60.);
    let _view = ui.mount(row().child(text("Lamps")));
    let frame = |ui: &mut Ui, gpu: &mut GpuRenderer| {
        ui.prepare_frame();
        let taken = ui.scene.borrow().default_text_services().cloned();
        let damage = ui.scene.borrow_mut().flush().damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        taken
    };
    // The first to draw it, with other text services, says so (an issue a
    // host hands on), once.
    let said = |gpu: &mut GpuRenderer| {
        gpu.take_issues()
            .iter()
            .filter(|issue| issue.contains("another renderer's text services"))
            .count()
    };
    let taken = frame(&mut ui, &mut drawing).expect("the last renderer's taken");
    assert_eq!(said(&mut drawing), 1);
    for each in 0..4 {
        let gpu = if each % 2 == 0 {
            &mut last
        } else {
            &mut drawing
        };
        let kept = frame(&mut ui, gpu).expect("kept");
        assert!(same(&taken, &kept));
    }
    assert_eq!((said(&mut drawing), said(&mut last)), (0, 0));
    drawing.install_text(&mut ui.scene.borrow_mut());
    assert!(frame(&mut ui, &mut drawing).is_none());
    assert!(frame(&mut ui, &mut drawing).is_none());
}

/// A scene whose host measures its text another way (a plain text measurer,
/// as `fx_bench` installs) keeps it: a renderer's text services are taken
/// only by a scene with none of any kind.
#[test]
fn a_scene_measured_its_hosts_way_takes_no_default_text_services() {
    let Ok(mut gpu) = GpuRenderer::new(200, 60) else {
        eprintln!("no GPU adapter: compared with nothing");
        return;
    };
    let mut scene = zgx::scene::Scene::new(200., 60.);
    scene.set_text_measurer(|text: &str, size: f32, _: Option<f32>| {
        (text.chars().count() as f32 * size, size)
    });
    let root = scene.root();
    let label = scene.append(
        root,
        zgx::scene::NodeKind::Text {
            text: "Lamps".into(),
            color: zgx::scene::Color(255, 255, 255, 255),
            font_size: 12.,
        },
        zgx::scene::Style::default(),
    );
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    assert!(scene.default_text_services().is_none());
    assert_eq!(scene.layout_bounds(label).width, 60.);
}

/// Without a renderer to trim the words it shapes as it draws, the text
/// cache sweeps them as they grow: 450,000 labels' values churned headless
/// leave cosmic-text's words bounded (untrimmed, they grow about 2.7 KB a
/// frame, without end); `release_unused` empties both caches at once, and
/// gives back the table the words filled.
#[test]
fn headless_words_stay_bounded_and_release_on_demand() {
    let services = TextServices::new();
    let mut ui = Ui::new(800., 900.);
    services.install(&mut ui.scene.borrow_mut());
    let counter = ui.signal(0_u64);
    let _view = ui.mount(column().children((0..30_u64).map(|i| {
        let counter = counter.clone();
        text_signal(move || format!("Label {i}: {} items", counter.get() * 7 + i))
    })));
    let held = || {
        let (shaped, compact) = services.text_cache().borrow().bytes();
        let words = services.text_system().borrow().shape_run_cache.len();
        (shaped + compact, words)
    };
    let frame = |n: u64| {
        counter.set(n);
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
    };
    // 15,000 frames of 30 labels: 450,000 values, about 105,000 new words.
    // Held, the words climb to where sweeps keep them and stay: the last
    // third of the churn holds no more than the one before (but the words a
    // few texts shape between two looks at the cache).
    let mut most = [0; 3];
    for n in 1..=15000 {
        frame(n);
        let third = (n as usize - 1) / 5000;
        most[third] = most[third].max(held().1);
    }
    assert!(
        most[2] <= most[1] + 256,
        "{most:?} words held at most in each third"
    );
    assert!(
        most[2] < 60_000,
        "{most:?} words held at most in each third"
    );
    services.release_unused();
    assert_eq!(held(), (0, 0));
    // The words' table too, not only the words: it stays the size the churn
    // grew it to otherwise.
    assert_eq!(services.text_system().borrow().shape_run_cache.bytes(), 0);
    // What is shown is shaped again as it is laid out.
    frame(15001);
    assert!(held().0 > 0);
}

/// On a grid of cells (`LetterSpacing::cells`, a terminal's), a run takes
/// exactly its characters' cells, whichever fonts draw them: braille (wider
/// than a cell in macOS's fallback), a letterlike symbol, Thai, a combining
/// accent (no cell of its own) and plain letters alike. Off the grid the
/// same text keeps its fonts' own advances.
#[test]
fn a_run_on_a_grid_takes_its_cells_whichever_font_draws_it() {
    use std::sync::Arc;
    use zgx::text_layout::{FontFamily, FontFeatures, FontStyle, LetterSpacing};
    let mut scene = zgx::scene::Scene::new(800., 600.);
    TextServices::new().install(&mut scene);
    let mut font = FontStyle {
        family: FontFamily::Monospace,
        features: FontFeatures::new([(*b"liga", 0), (*b"calt", 0)]),
        fallbacks: Arc::from([FontFamily::SansSerif]),
        ..FontStyle::default()
    };
    let size = 13.;
    let cell = scene
        .measure_text_with_font("0000000000", size, None, &font)
        .0
        / 10.;
    font.letter_spacing = LetterSpacing::cells(cell);
    for (text, cells) in [
        ("\u{280b}\u{2819}\u{2839}\u{2838}\u{283c} Loading", 13),
        ("21\u{2103} \u{20b9}5", 6),
        ("\u{e01}\u{e32}\u{e23} ok", 6),
        ("cafe\u{301} 0123", 9),
        ("plain text", 10),
    ] {
        let width = scene.measure_text_with_font(text, size, None, &font).0;
        assert!(
            (width - cells as f32 * cell).abs() < 0.05,
            "{text:?}: {width} for {cells} cells of {cell}"
        );
    }
}

/// The link's box in a paragraph at `w` (its first fragment, as the accessibility tree has it), laid out by the
/// real shaper (`shaped`) or headless (`FallbackTextLayout`): `words` in one span, then "Publish…" as a link span of
/// its own.
fn link_box(w: f32, words: &str, shaped: bool) -> Rect {
    use zgx::semantics::Role;
    let mut ui = Ui::new(1200., 200.);
    if shaped {
        TextServices::new().install(&mut ui.scene.borrow_mut());
    }
    ui.mount(
        rich_text()
            .w(w)
            .text_size(13.)
            .line_height(16.)
            .text_wrap(true)
            .child(text_span(words))
            .child(text_span("Publish…").on_click(|| {}))
            .into(),
    );
    ui.prepare_frame();
    let link = ui
        .semantics
        .borrow()
        .iter()
        .find(|(_, s)| matches!(s.role, Role::Link | Role::Button))
        .map(|(id, _)| id)
        .expect("the link");
    ui.scene.borrow().bounds(link)
}

/// A line that must break where one span ends and the next begins (the first span's trailing space) breaks there,
/// as it would inside a span, and as Chrome does: the next span's word moves whole to the next line, not split at
/// the line's end. Shaped or headless alike; a break inside the first span is the control.
#[test]
fn a_line_breaks_at_a_spans_boundary_as_between_words() {
    for shaped in [true, false] {
        for words in [
            "Share it in the plugin store ",
            "1.0.0 in the plugin store · 1.1.0 not approved ",
        ] {
            // Wide enough for both on one line: where the link starts and how wide it is.
            let one = link_box(1100., words, shaped);
            assert_eq!(one.y, 0., "{words:?}, shaped {shaped}: one line: {one:?}");
            // The words fit, the link not after them: the break is the boundary's.
            let w = one.x + one.width / 2.;
            let at = link_box(w, words, shaped);
            assert!(
                (at.width - one.width).abs() < 0.5 && at.y >= 16. && at.x < 1.,
                "{words:?} at {w}, shaped {shaped}: the link whole at line two's start, not split: {at:?} ({one:?})"
            );
            // The control: narrower still, the break inside the words.
            let inside = link_box(one.x - 20., words, shaped);
            assert!(
                (inside.width - one.width).abs() < 0.5 && inside.y >= 16.,
                "{words:?}, shaped {shaped}: a break inside the words keeps the link whole: {inside:?}"
            );
        }
    }
}

/// Geist Mono: every character 0.6 em wide, as the fallback layout's estimate is, so the two lay out the same widths
/// and only their breaks can differ.
const GEIST_MONO: &[u8] = include_bytes!("../../zgx-desktop/examples/support/fonts/GeistMono.ttf");
/// A character's width in Geist Mono at 10 points.
const CELL: f32 = 6.;

/// Where each of `layout`'s rows starts in `text`: each grapheme whose caret is on a row below the last's.
fn row_starts(layout: &dyn zgx::text_layout::TextLayout, text: &str) -> Vec<usize> {
    use zgx::text_layout::{TextAffinity, TextPosition};
    let at = |byte_offset| {
        layout
            .caret_position(TextPosition {
                byte_offset,
                affinity: TextAffinity::After,
            })
            .y
    };
    let mut starts = vec![0];
    let mut last = at(0);
    for range in zgx::text_layout::grapheme_ranges(text).skip(1) {
        let y = at(range.start);
        if y > last + 0.5 {
            starts.push(range.start);
            last = y;
        }
    }
    starts
}

/// Headless (`FallbackTextLayout`, what a scene with no shaper lays out with) and shaped, text breaks into the same
/// lines: at hard breaks (line feeds, CRLF, a record separator), by word where a line is full (UAX #14: spaces
/// hanging past the edge, a hyphen, zero-width spaces, no break at a no-break space), a word wider than the line by
/// grapheme, and where one span ends and the next begins as anywhere else. The widths of their lines may differ
/// (estimated advances); where they break mustn't, or a headless test sees lines the app doesn't.
#[test]
fn headless_text_breaks_where_shaped_text_does() {
    use zgx::rich_text::{RichText, TextRun};
    use zgx::text_layout::{FallbackTextLayout, FontFamily, FontStyle};
    let services = TextServices::new();
    zgx_gpu::text::FontData::new(GEIST_MONO.to_vec())
        .unwrap()
        .install(&mut services.text_system().borrow_mut());
    let mut scene = zgx::scene::Scene::new(800., 600.);
    services.install(&mut scene);
    let font = FontStyle {
        family: FontFamily::Named("Geist Mono".into()),
        ..FontStyle::default()
    };
    let mut wrong = Vec::new();
    // Each text and its width, in characters.
    for (text, cells) in [
        ("the quick brown fox jumps over the lazy dog", 16),
        ("aaa bbb", 3),
        ("aaa   bbb ccc", 4),
        ("supercalifragilistic and more", 8),
        ("a hard\nbreak and more words here", 12),
        ("line one\r\nline two is longer than that", 12),
        ("ends with a break\n", 10),
        ("let x = 1;\u{1e}let y = 2;\u{1e}console.log(x);", 40),
        ("well-known hyphen-ated words", 9),
        ("verylongword\u{200b}withzero\u{200b}widthbreaks", 14),
        ("no\u{a0}break\u{a0}here at all", 10),
        ("dash\u{2014}between words", 7),
        ("Share it in the plugin store Publish\u{2026}", 34),
        ("tab\tseparated\twords and more of them", 12),
        ("a line\u{2028}separator and more words", 12),
        ("a paragraph\u{2029}separator and more", 12),
        ("a lone\rcarriage return here", 12),
        ("a next\u{85}line here and more", 12),
    ] {
        let width = Some(cells as f32 * CELL + CELL / 2.);
        let shaped = scene.shape_text_with_font(text, 10., width, &font);
        let headless = FallbackTextLayout::with_font(text, 10., width, &font);
        let (shaped, headless) = (row_starts(&*shaped, text), row_starts(&headless, text));
        if shaped != headless {
            wrong.push(format!(
                "{text:?} in {cells}: shaped {shaped:?}, headless {headless:?}"
            ));
        }
    }
    // Rich text: a line full where one span ends and the next begins.
    for (spans, cells) in [
        (
            &["Share it in the plugin store ", "Publish\u{2026}"][..],
            34,
        ),
        (
            &["Share it in the plugin store", " Publish\u{2026}"][..],
            34,
        ),
        (
            &["Share it in the plugin store", " ", "Publish\u{2026}"][..],
            34,
        ),
        (&["one ", "two ", "three ", "four"][..], 9),
        (&["glued", "together", "words"][..], 12),
    ] {
        let mut text = String::new();
        let mut runs = Vec::new();
        for span in spans {
            let start = text.len();
            text.push_str(span);
            runs.push(TextRun {
                range: start..text.len(),
                font: font.clone(),
                font_size: 10.,
                ..TextRun::default()
            });
        }
        let rich = RichText::new(text.as_str(), runs).unwrap();
        let width = Some(cells as f32 * CELL + CELL / 2.);
        let shaped = scene.shape_rich_text(&rich, width);
        let headless = FallbackTextLayout::with_rich(&rich, width);
        let (shaped, headless) = (row_starts(&*shaped, &text), row_starts(&headless, &text));
        if shaped != headless {
            wrong.push(format!(
                "{spans:?} in {cells}: shaped {shaped:?}, headless {headless:?}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
