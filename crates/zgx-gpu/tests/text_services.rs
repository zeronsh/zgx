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
/// frame, without end); `release_unused` empties both caches at once.
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
    // What is shown is shaped again as it is laid out.
    frame(15001);
    assert!(held().0 > 0);
}

/// A node in a paragraph's text (`TextSpan::node`), shaped as a renderer
/// shapes it: its top `baseline` above the line's baseline, the text about
/// it on that baseline too, and its line grown to hold it.
#[test]
fn an_inline_node_sits_on_its_lines_baseline() {
    use zgx::scene::NodeKind;
    let services = TextServices::new();
    let mut ui = Ui::new(320., 240.);
    services.install(&mut ui.scene.borrow_mut());
    let node = InlineNode::new(column().w(30.).h(40.).id("formula"));
    let view = ui.mount(
        column().w(320.).text_size(15.).child(
            rich_text()
                .id("para")
                .child(text_span("where "))
                .child(text_span("$x$").node(&node, 30., 40., 30.))
                .child(text_span(" holds")),
        ),
    );
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    let paragraph = scene.children(view.find("para").unwrap())[0];
    let NodeKind::RichText { text } = scene.kind(paragraph) else {
        panic!("a paragraph");
    };
    let layout = scene.shape_rich_text(text, None);
    let baseline = layout.baseline(0).unwrap();
    let at = scene.bounds(paragraph);
    let room = scene.bounds(view.find("formula").unwrap());
    assert!(
        (room.y + 30. - (at.y + baseline)).abs() < 0.01,
        "{room:?} on {baseline} of {at:?}"
    );
    assert!(
        room.y >= at.y - 0.01 && room.y + 40. <= at.y + at.height + 0.01,
        "{room:?} in {at:?}"
    );
    // On one line, after its words.
    assert!(room.x > at.x && at.height < 60., "{room:?} in {at:?}");
}
