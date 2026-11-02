//! A text's language (`Styled::lang`, HTML's `lang`): inherited by what it
//! holds, apart from the features it rides with, and what assistive
//! technology reads it in.
use zgx::{compose::prelude::*, text_layout::FontFeatures, widgets::Ui};

/// Features as a font's own, without zgx's marks of the language its text
/// is in (`_han`, and whether it's inherited), which `han()` reads.
fn own(features: &FontFeatures) -> FontFeatures {
    FontFeatures::new(
        features
            .settings()
            .iter()
            .copied()
            .filter(|(tag, _)| tag[0] != b'_'),
    )
}

#[test]
fn a_language_is_inherited_apart_from_the_features_it_rides_with() {
    let mut ui = Ui::new(400., 300.);
    let handle = ui.mount(
        column()
            .lang("zh-TW")
            .child(text("inherited").id("inherited"))
            .child(
                text("own features")
                    .font_features(FontFeatures::new([(*b"tnum", 1)]))
                    .id("features"),
            )
            .child(
                column()
                    .lang("ja")
                    .child(text("nested").id("nested"))
                    .id("japanese"),
            )
            .id("root"),
    );
    let outside = ui.mount(text("outside"));
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    let node = |id: &str| handle.find(id).unwrap();
    let han = |id: &str| scene.font(node(id)).features.han();
    assert_eq!(han("inherited"), 5, "traditional Chinese");
    // Features of its own keep the inherited language.
    assert_eq!(han("features"), 5);
    let features = &scene.font(node("features")).features;
    assert_eq!(own(features), FontFeatures::new([(*b"tnum", 1)]));
    // Nearer says.
    assert_eq!(han("nested"), 1, "Japanese");
    // None set: the features say nothing, as before.
    assert_eq!(scene.font(outside.node()).features, FontFeatures::default());
    // Assistive technology's: the nearest that says.
    let language = |node| scene.language(node).map(|l| l.to_string());
    assert_eq!(language(node("inherited")).as_deref(), Some("zh-TW"));
    assert_eq!(language(node("nested")).as_deref(), Some("ja"));
    assert_eq!(language(outside.node()), None);
}

#[test]
fn a_span_says_its_own_language_over_its_paragraphs() {
    let mut ui = Ui::new(400., 300.);
    let handle = ui.mount(
        column().lang("ja").child(
            rich_text()
                .child(text_span("日本の"))
                .child(text_span("中國人").lang("zh-TW"))
                .id("paragraph"),
        ),
    );
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    let paragraph = scene.children(handle.find("paragraph").unwrap())[0];
    let zgx::scene::NodeKind::RichText { text } = scene.kind(paragraph) else {
        panic!("a paragraph");
    };
    let han: Vec<u32> = text
        .runs()
        .iter()
        .map(|run| run.font.features.han())
        .collect();
    assert_eq!(han, [1, 5]);
}

#[test]
fn a_language_names_which_chinese_japanese_or_korean_or_another() {
    let cases = [
        ("ja", 1),
        ("ja-JP", 1),
        ("ko-KR", 2),
        ("en", 3),
        ("fr-CA", 3),
        ("zh", 4),
        ("zh-CN", 4),
        ("zh-Hans-SG", 4),
        ("zh-Hant", 5),
        ("zh_TW", 5),
        ("zh-Hant-HK", 6),
        ("zh-MO", 6),
        ("yue", 6),
        ("ZH-tw", 5),
    ];
    for (language, han) in cases {
        assert_eq!(FontFeatures::han_of(language), han, "{language}");
    }
    let tnum = FontFeatures::new([(*b"tnum", 1)]);
    assert_eq!(tnum.with_han(5).with_han(0), tnum);
    assert_eq!(tnum.with_han(5).with_han(1).han(), 1);
}

#[test]
fn features_and_a_language_on_one_element_keep_each_other_in_either_order() {
    let mut ui = Ui::new(400., 300.);
    let tnum = FontFeatures::new([(*b"tnum", 1)]);
    let handle = ui.mount(
        column()
            .font_features(FontFeatures::new([(*b"smcp", 1)]))
            .child(text("a").font_features(tnum.clone()).lang("ko").id("first"))
            .child(
                text("b")
                    .lang("ko")
                    .font_features(tnum.clone())
                    .id("second"),
            )
            .child(text("c").lang("ko").id("inherits")),
    );
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    let features = |id: &str| scene.font(handle.find(id).unwrap()).features.clone();
    for id in ["first", "second"] {
        assert_eq!(
            (own(&features(id)), features(id).han()),
            (tnum.clone(), 2),
            "{id}"
        );
    }
    // A language alone keeps the inherited features.
    assert_eq!(
        (own(&features("inherits")), features("inherits").han()),
        (FontFeatures::new([(*b"smcp", 1)]), 2)
    );
}

/// A language changed as the app runs: the text it holds takes it again,
/// plain text with features of its own as well as without, a paragraph's
/// runs too, while a span's own language stands.
#[test]
fn a_language_changed_at_run_time_is_taken_again_by_what_it_holds() {
    let mut ui = Ui::new(400., 300.);
    let language = ui.signal(Some(String::from("ja")));
    let read = language.clone();
    let handle = ui.mount(
        column()
            .reactive_style(move || match read.get() {
                Some(language) => zgx::style::Styles::new().lang(language),
                None => zgx::style::Styles::new(),
            })
            .child(text("plain").id("plain"))
            .child(
                text("own features")
                    .font_features(FontFeatures::new([(*b"tnum", 1)]))
                    .id("features"),
            )
            .child(
                rich_text()
                    .child(text_span("段落"))
                    .child(text_span("中國人").lang("zh-TW"))
                    .id("paragraph"),
            ),
    );
    ui.prepare_frame();
    let hans = |ui: &Ui| {
        let scene = ui.scene.borrow();
        let han = |id: &str| scene.font(handle.find(id).unwrap()).features.han();
        let paragraph = scene.children(handle.find("paragraph").unwrap())[0];
        let zgx::scene::NodeKind::RichText { text } = scene.kind(paragraph) else {
            panic!("a paragraph");
        };
        let runs: Vec<u32> = text
            .runs()
            .iter()
            .map(|run| run.font.features.han())
            .collect();
        (han("plain"), han("features"), runs)
    };
    assert_eq!(hans(&ui), (1, 1, vec![1, 5]), "Japanese");
    language.set(Some("ko".into()));
    ui.prepare_frame();
    assert_eq!(
        hans(&ui),
        (2, 2, vec![2, 5]),
        "Korean: the span keeps its own"
    );
    language.set(Some("en".into()));
    ui.prepare_frame();
    let (plain, features, runs) = hans(&ui);
    assert_eq!((plain, features, runs[1]), (3, 3, 5), "another language");
    // None said: none taken, the features as they were.
    language.set(None);
    ui.prepare_frame();
    assert_eq!(hans(&ui), (0, 0, vec![0, 5]), "no language");
    let scene = ui.scene.borrow();
    let font = |id: &str| scene.font(handle.find(id).unwrap()).features.clone();
    assert_eq!(
        font("features"),
        FontFeatures::new([(*b"tnum", 1)]),
        "its features stay"
    );
    assert_eq!(font("plain"), FontFeatures::default());
}
