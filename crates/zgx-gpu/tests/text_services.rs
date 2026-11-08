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
