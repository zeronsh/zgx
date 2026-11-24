//! On macOS a character the run's face lacks draws in the face Core Text
//! gives it as Chrome asks (system-ui's substitute, matched again at Blink's
//! weight: `.PingFang UI SC`'s Display face for Han and CJK punctuation,
//! `.CJK Symbols Fallback SC`'s Medium for kana at 450), with Core Text's
//! advances, tracking included: measured against Core Text here, at test
//! time, so a macOS that moves them moves both.
#![cfg(target_os = "macos")]
use cosmic_text::{Attrs, AttrsList, BufferLine, Family, FontSystem, LineEnding, Shaping, Weight};

/// Each glyph of `text` shaped as zgx shapes text (sans-serif, at the
/// size's optical size): its character, advance in pixels and face.
fn shaped(fonts: &mut FontSystem, text: &str, weight: u16, size: u16) -> Vec<(char, f32, String)> {
    let attrs = Attrs::new()
        .family(Family::SansSerif)
        .weight(Weight(weight))
        .optical(size);
    let mut line = BufferLine::new(
        text,
        LineEnding::None,
        AttrsList::new(&attrs),
        Shaping::Advanced,
    );
    let glyphs: Vec<_> = (line.shape(fonts, 8).spans.iter())
        .flat_map(|span| &span.words)
        .flat_map(|word| &word.glyphs)
        .map(|glyph| {
            (
                glyph.start,
                glyph.x_advance * f32::from(size),
                glyph.font_id,
            )
        })
        .collect();
    glyphs
        .into_iter()
        .map(|(start, advance, id)| {
            let face = fonts
                .db()
                .face(id)
                .map(|face| face.post_script_name.clone());
            (
                text[start..].chars().next().unwrap(),
                advance,
                face.unwrap_or_default(),
            )
        })
        .collect()
}

/// A face's name up to its style: Core Text names a variable face's
/// instance (`.CJKSymbolsFallbackSC-Medium`), the font file its face
/// (`.CJKSymbolsFallbackSC-Regular`).
fn stem(name: &str) -> &str {
    name.rsplit_once('-').map_or(name, |(stem, _)| stem)
}

#[test]
fn cjk_text_falls_back_and_advances_as_core_text_does() {
    let gpu = zgx_gpu::GpuRenderer::new(16, 16).unwrap();
    let fonts = gpu.text_system();
    let mut fonts = fonts.borrow_mut();
    // Han, CJK punctuation, hiragana and katakana.
    let classes = ['学', '録', '。', '、', '「', 'こ', 'ア'];
    let mut wrong = Vec::new();
    for size in [12u16, 14, 20, 28] {
        for weight in [400u16, 450, 700] {
            for c in classes {
                let Some((face, advance)) = zgx_gpu::core_text_advance(c, weight, f32::from(size))
                else {
                    panic!("Core Text has no face for {c}");
                };
                let glyphs = shaped(&mut fonts, &c.to_string(), weight, size);
                let [(_, shaped_advance, shaped_face)] = glyphs.as_slice() else {
                    panic!("{c} shaped as {glyphs:?}");
                };
                if stem(shaped_face) != stem(&face)
                    || (f64::from(*shaped_advance) - advance).abs() >= 0.01
                {
                    wrong.push(format!(
                        "{c} {size}px w{weight}: {shaped_face} {shaped_advance:.3}, Core Text {face} {advance:.3}"
                    ));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "as Core Text falls back:\n{}",
        wrong.join("\n")
    );
}

/// Chrome's own numbers at 14 px, weight 450 (DevTools, macOS 26): 学 and 。
/// in `.PingFangUIDisplaySC-Default` 13.42, こ in
/// `.CJKSymbolsFallbackSC-Medium` 13.75; a Japanese line as wide.
#[test]
fn a_japanese_line_is_as_wide_as_chrome_lays_it() {
    let gpu = zgx_gpu::GpuRenderer::new(16, 16).unwrap();
    let fonts = gpu.text_system();
    let mut fonts = fonts.borrow_mut();
    let glyphs = shaped(&mut fonts, "学こ。", 450, 14);
    let widths: Vec<f32> = glyphs
        .iter()
        .map(|glyph| (glyph.1 * 100.).round() / 100.)
        .collect();
    assert_eq!(widths, [13.42, 13.75, 13.42], "{glyphs:?}");
}

/// Anara's repro: "about 学習記録" at 14 px, weight 450, fits a 96 px box
/// on one line, as in Chrome (Han at 14.29 px, PingFang SC's, wrapped 録).
#[test]
fn mixed_latin_and_han_fits_where_chrome_fits_it() {
    let gpu = zgx_gpu::GpuRenderer::new(16, 16).unwrap();
    let fonts = gpu.text_system();
    let mut fonts = fonts.borrow_mut();
    let mut buffer = cosmic_text::Buffer::new(&mut fonts, cosmic_text::Metrics::new(14., 20.));
    buffer.set_size(Some(96.), None);
    let attrs = Attrs::new()
        .family(Family::SansSerif)
        .weight(Weight(450))
        .optical(14);
    buffer.set_text("about 学習記録", &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(&mut fonts, false);
    let lines: Vec<f32> = buffer.layout_runs().map(|run| run.line_w).collect();
    assert_eq!(lines.len(), 1, "wrapped: {lines:?}");
}

/// A font system of `locale` with the system's fonts and Core Text's
/// fallback, as zgx's own has it.
fn fonts_of(locale: &str) -> FontSystem {
    let (_, db) = FontSystem::new().into_locale_and_db();
    let mut fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
    zgx_gpu::install_core_text_fallback(&mut fonts);
    fonts.set_language(Some(locale));
    fonts
}

/// Chrome's own measure (157, DPR 2, `400 13px system-ui`) of a Japanese
/// line in each of two pages: one with no language, whose kana fall to Core
/// Text's `.CJK Symbols Fallback SC` and Han to `.PingFang UI Text SC`
/// (153.50), and one with `lang="ja"`, whose script's standard font,
/// Hiragino Kaku Gothic ProN, takes them all before Core Text. zgx's
/// language (`FontSystem::set_language`, an app's i18n locale) stands for
/// the page's. Chrome's Japanese line is 154.84: its kana pairs kern across
/// the line's break opportunities (ォ before ル), which zgx shapes word by
/// word here, so this line is Hiragino's full width, 13 a glyph.
#[test]
fn a_japanese_line_falls_back_as_chrome_with_and_without_a_language() {
    let line = "フォルダにファイルを追加";
    for (locale, kana, han, width) in [
        (
            "en-US",
            ".CJKSymbolsFallbackSC",
            ".PingFangUITextSC",
            153.50,
        ),
        ("ja-JP", "HiraKakuProN-W3", "HiraKakuProN-W3", 12. * 13.),
    ] {
        let mut fonts = fonts_of(locale);
        let glyphs = shaped(&mut fonts, line, 400, 13);
        let total: f32 = glyphs.iter().map(|glyph| glyph.1).sum();
        for (c, _, face) in &glyphs {
            let want = if *c == '追' || *c == '加' {
                han
            } else {
                kana
            };
            assert_eq!(
                stem(face),
                stem(want),
                "{locale}: {c} in {face}, Chrome's {want}"
            );
        }
        assert!(
            (total - width).abs() < 0.1,
            "{locale}: {total}, want {width}"
        );
    }
    // At 450 the standard font matches its family's weight as CSS does:
    // W3 (300), as nothing lies between 450 and 500 (Chrome: 14.00 at 14 px).
    let mut fonts = fonts_of("ja-JP");
    let glyphs = shaped(&mut fonts, "こ", 450, 14);
    assert_eq!(glyphs[0].2, "HiraKakuProN-W3", "{glyphs:?}");
    assert!((glyphs[0].1 - 14.00).abs() < 0.01, "{glyphs:?}");
}
