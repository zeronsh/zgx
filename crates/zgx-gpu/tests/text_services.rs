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
