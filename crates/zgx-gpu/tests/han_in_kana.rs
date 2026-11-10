//! A simplified Chinese character among Japanese kana takes its run's
//! script, Japanese, whose fonts lack it: it is drawn from a Chinese font
//! next, and no other face is loaded to look for it. A phone had none of
//! the common fallbacks, and loaded every face on it for each such word.
use cosmic_text::{
    Attrs, Buffer, Fallback, Family, FontSystem, Metrics, Shaping,
    fontdb::{Database, Source},
};
use std::sync::Arc;
use unicode_script::Script;

/// Japanese kana in ZgxKana, Han in ZgxHan; no common fallbacks.
struct Lists;
impl Fallback for Lists {
    fn common_fallback(&self) -> &'static [&'static str] {
        &[]
    }
    fn forbidden_fallback(&self) -> &'static [&'static str] {
        &[]
    }
    fn script_fallback(&self, script: Script, _locale: &str) -> &'static [&'static str] {
        match script {
            Script::Hiragana | Script::Katakana => &["ZgxKana"],
            Script::Han => &["ZgxHan"],
            _ => &[],
        }
    }
}

const DECOYS: usize = 40;

#[test]
fn han_among_kana_comes_from_a_chinese_font_without_a_walk() {
    let fonts = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts");
    let mut db = Database::new();
    let font = |name: &str| {
        let data: Arc<dyn AsRef<[u8]> + Send + Sync> =
            Arc::new(std::fs::read(format!("{fonts}/{name}")).unwrap());
        Source::Binary(data)
    };
    let kana = db.load_font_source(font("ZgxKana.ttf"))[0];
    // Faces with neither, before Han in the database: a walk loads each
    // before finding Han.
    let decoy = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../zgx-desktop/assets/DejaVuSans.ttf"
    ))
    .unwrap();
    for _ in 0..DECOYS {
        db.load_font_data(decoy.clone());
    }
    let han = db.load_font_source(font("ZgxHan.ttf"))[0];
    let mut system = FontSystem::new_with_locale_and_db_and_fallback("ja-JP".into(), db, Lists);
    let mut buffer = Buffer::new(&mut system, Metrics::new(16., 20.));
    {
        let mut view = buffer.borrow_with(&mut system);
        view.set_text(
            "あいう车见",
            &Attrs::new().family(Family::Name("ZgxKana")),
            Shaping::Advanced,
            None,
        );
        view.shape_until_scroll(false);
    }
    let faces: Vec<_> = buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.font_id))
        .collect();
    assert_eq!(faces, [kana, kana, kana, han, han], "kana, then Chinese");
    assert!(
        system.loaded_fonts() <= 2,
        "{} fonts loaded, of {} faces: a walk over them",
        system.loaded_fonts(),
        DECOYS + 2
    );
}
