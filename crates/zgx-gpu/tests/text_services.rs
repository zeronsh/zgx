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

/// A line mark's label in tabular figures (`LineMark::tabular`, OpenType's
/// `tnum`: an ordered list's numbers, lined up as the web's): each digit of
/// "1." and "11." a figure's width, as "8."'s is. Without it, Geist's "1"
/// is narrower than its other figures, and a mark's label is as it was.
#[test]
fn a_tabular_mark_labels_digits_each_take_a_figures_width() {
    use std::rc::Rc;
    use zgx::scene::{Color, NodeKind};
    use zgx::widgets::LineMark;
    let services = TextServices::new();
    zgx_gpu::text::FontData::new(include_bytes!("fonts/GeistVariable-subset.ttf").to_vec())
        .unwrap()
        .install(&mut services.text_system().borrow_mut());
    let mut ui = Ui::new(400., 200.);
    services.install(&mut ui.scene.borrow_mut());
    let value = ui.signal("one\ntwo\nthree".to_owned());
    let view = ui.mount(
        text_area("Note", value)
            .w(300.)
            .h(150.)
            .font_family("Geist")
            .text_size(13.)
            .line_height(20.),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_line_marks(Some(Rc::new(|i| {
        let (label, tabular) = [("1.", true), ("11.", true), ("1.", false)][i.min(2)];
        vec![LineMark {
            width: 28.,
            height: 20.,
            label: Some((label.into(), Color(0, 0, 0, 255), 13.)),
            tabular,
            ..LineMark::default()
        }]
    })));
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    let (mut tabular, mut plain) = (Vec::new(), Vec::new());
    let mut pending = vec![editor.node];
    while let Some(id) = pending.pop() {
        if let NodeKind::Text { text, .. } = scene.kind(id)
            && text.ends_with('.')
        {
            let font = scene.font(id).clone();
            let tnum = font.features.settings().contains(&(*b"tnum", 1));
            if tnum { &mut tabular } else { &mut plain }.push((text.to_string(), font));
        }
        pending.extend_from_slice(scene.children(id));
    }
    assert_eq!(tabular.len(), 2, "{tabular:?}");
    assert_eq!(plain.len(), 1, "a mark without it as it was: {plain:?}");
    let width = |text: &str, font| scene.measure_text_with_font(text, 13., None, font).0;
    let font = &tabular[0].1;
    let figure = width("8.", font) - width(".", font);
    assert!(
        (width("1.", font) - width(".", font) - figure).abs() < 0.01,
        "\"1\" a figure's width"
    );
    assert!(
        (width("11.", font) - width(".", font) - 2. * figure).abs() < 0.01,
        "\"11\" two figures' widths"
    );
    let font = &plain[0].1;
    assert!(
        width("1.", font) < width("8.", font) - 0.5,
        "Geist's proportional \"1\" is narrower"
    );
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
