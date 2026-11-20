//! `dir = "auto"`: text runs the way its first strong character does.
use zgx::{compose::prelude::*, scene::Direction, scene::NodeId, widgets::Ui};

fn text_node(ui: &Ui, handle: &zgx::compose::ViewHandle, id: &str) -> NodeId {
    let scene = ui.scene.borrow();
    let mut node = handle.find(id).unwrap();
    while !matches!(
        scene.kind(node),
        zgx::scene::NodeKind::Text { .. } | zgx::scene::NodeKind::RichText { .. }
    ) {
        node = scene.children(node)[0];
    }
    node
}

#[test]
fn the_first_strong_character_says_which_way_text_runs() {
    use zgx::text_layout::first_strong_direction as dir;
    assert_eq!(dir("مرحبا بالعالم"), Some(Direction::Rtl));
    assert_eq!(dir("שלום עולם"), Some(Direction::Rtl));
    assert_eq!(dir("Hello مرحبا"), Some(Direction::Ltr));
    assert_eq!(
        dir("123 - مرحبا"),
        Some(Direction::Rtl),
        "digits and marks are not strong"
    );
    assert_eq!(
        dir("\u{2067}مرحبا\u{2069} abc"),
        Some(Direction::Ltr),
        "an isolate's text is skipped"
    );
    assert_eq!(dir("12 + 3 = 15"), None);
    assert_eq!(dir(""), None);
}

#[test]
fn auto_text_runs_right_to_left_for_arabic_and_starts_at_the_right() {
    let mut ui = Ui::new(600., 400.);
    let handle = ui.mount(column().w(400.).children(vec![
        text("مرحبا بالعالم").dir_auto().w(300.).id("ar"),
        text("שלום עולם").dir_auto().w(300.).id("he"),
        text("Hello world").dir_auto().w(300.).id("en"),
        text("2026").dir_auto().w(300.).id("digits"),
    ]));
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    use zgx::text_layout::TextAlign;
    drop(scene);
    for (id, direction, align) in [
        ("ar", Direction::Rtl, TextAlign::Right),
        ("he", Direction::Rtl, TextAlign::Right),
        ("en", Direction::Ltr, TextAlign::Start),
        ("digits", Direction::Ltr, TextAlign::Start),
    ] {
        let node = text_node(&ui, &handle, id);
        let scene = ui.scene.borrow();
        assert_eq!(scene.direction(node), direction, "{id}");
        assert_eq!(scene.font(node).align, align, "{id}");
    }
}

#[test]
fn text_with_no_strong_character_takes_its_parents_way() {
    let mut ui = Ui::new(600., 400.);
    let handle = ui.mount(
        column()
            .rtl()
            .w(400.)
            .child(text("2026").dir_auto().w(300.).id("digits")),
    );
    ui.prepare_frame();
    let node = text_node(&ui, &handle, "digits");
    assert_eq!(ui.scene.borrow().direction(node), Direction::Rtl);
}

#[test]
fn auto_direction_follows_the_text_as_it_changes() {
    let mut ui = Ui::new(600., 400.);
    let words = ui.signal(String::from("Hello"));
    let shown = words.clone();
    let handle = ui.mount(
        column()
            .w(400.)
            .child(text_signal(move || shown.get()).dir_auto().w(300.).id("t")),
    );
    ui.prepare_frame();
    let node = text_node(&ui, &handle, "t");
    assert_eq!(ui.scene.borrow().direction(node), Direction::Ltr);
    words.set("مرحبا".into());
    ui.prepare_frame();
    assert_eq!(ui.scene.borrow().direction(node), Direction::Rtl);
    words.set("Hello again".into());
    ui.prepare_frame();
    assert_eq!(ui.scene.borrow().direction(node), Direction::Ltr);
}

/// A container takes the way of the text it holds: a list item's marker
/// and text swap sides as they do in an Arabic answer.
#[test]
fn an_auto_container_mirrors_its_row_by_its_text() {
    let mut ui = Ui::new(600., 400.);
    let handle = ui.mount(column().w(400.).children(vec![
        row().dir_auto().w(400.).id("ar").children(vec![
            div().w(10.).h(10.).id("ar_marker"),
            text("مرحبا").id("ar_text"),
        ]),
        row().dir_auto().w(400.).id("en").children(vec![
            div().w(10.).h(10.).id("en_marker"),
            text("Hello").id("en_text"),
        ]),
    ]));
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    let at = |id: &str| scene.bounds(handle.find(id).unwrap());
    assert!(
        at("ar_marker").x > at("ar_text").x,
        "the marker at the right in Arabic"
    );
    assert!(
        at("en_marker").x < at("en_text").x,
        "and at the left in English"
    );
}

/// Text in a right to left node shapes at that base level, whatever it
/// starts with (`FontStyle::paragraph_rtl`): plain text through its font,
/// rich text through `Scene::fit_rich`; text in a left to right node doesn't.
#[test]
fn text_in_a_right_to_left_node_takes_the_right_to_left_base_level() {
    use std::sync::Arc;
    let mut ui = Ui::new(600., 400.);
    let handle = ui.mount(column().w(400.).children(vec![
        column().rtl().w(300.).id("rtl").child(text("Hello world.").id("plain")),
        column().w(300.).id("ltr").child(text("Hello world.").id("plain_ltr")),
    ]));
    ui.prepare_frame();
    let plain = text_node(&ui, &handle, "plain");
    let plain_ltr = text_node(&ui, &handle, "plain_ltr");
    let scene = ui.scene.borrow();
    assert!(scene.font(plain).paragraph_rtl);
    assert!(!scene.font(plain_ltr).paragraph_rtl);
    let rich = Arc::new(
        zgx::rich_text::RichText::new(
            "Hello world.",
            vec![zgx::rich_text::TextRun {
                range: 0..12,
                ..Default::default()
            }],
        )
        .unwrap(),
    );
    assert!(
        scene.fit_rich(plain, 300., &rich).runs()[0]
            .font
            .paragraph_rtl
    );
    assert!(
        !scene.fit_rich(plain_ltr, 300., &rich).runs()[0]
            .font
            .paragraph_rtl
    );
}
