//! On macOS a full-width CJK glyph advances as Chrome lays system-ui out
//! (PingFang UI, optically sized): about 0.959 em for an ideograph at 14 px.
#![cfg(target_os = "macos")]
use cosmic_text::{Attrs, AttrsList, BufferLine, Family, FontSystem, LineEnding, Shaping, Weight};

fn width(fonts: &mut FontSystem, text: &str, size: u16) -> f32 {
    let attrs = Attrs::new()
        .family(Family::SansSerif)
        .weight(Weight(450))
        .optical(size);
    let mut line = BufferLine::new(
        text,
        LineEnding::None,
        AttrsList::new(&attrs),
        Shaping::Advanced,
    );
    line.shape(fonts, 8)
        .spans
        .iter()
        .flat_map(|span| &span.words)
        .flat_map(|word| &word.glyphs)
        .map(|glyph| glyph.x_advance * f32::from(size))
        .sum()
}

#[test]
fn cjk_advances_match_chrome() {
    let mut fonts = FontSystem::new();
    // 学習と記憶 at 14 px in Chrome: 13.422 + 13.422 + 13.75 + 13.422 + 13.422.
    let w = width(&mut fonts, "学習と記憶", 14);
    assert!((w - 67.438).abs() < 0.3, "{w}");
    let w = width(&mut fonts, "学習と記憶", 28);
    assert!((w - 133.125).abs() < 0.8, "{w}");
}
