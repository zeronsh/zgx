//! On macOS an Apple Color Emoji glyph advances as CoreText gives it at that
//! size (1.25 em small, easing to 1 em), as the same text lays out in Chrome.
#![cfg(target_os = "macos")]
use cosmic_text::{Attrs, AttrsList, BufferLine, Family, FontSystem, LineEnding, Shaping, Weight};

fn emoji_advance(fonts: &mut FontSystem, size: u16) -> Option<f32> {
    let attrs = Attrs::new()
        .family(Family::SansSerif)
        .weight(Weight(450))
        .optical(size);
    let mut line = BufferLine::new(
        "🧬",
        LineEnding::None,
        AttrsList::new(&attrs),
        Shaping::Advanced,
    );
    let glyph = line
        .shape(fonts, 8)
        .spans
        .iter()
        .flat_map(|span| &span.words)
        .flat_map(|word| &word.glyphs)
        .next()?
        .clone();
    let color = fonts.db().face(glyph.font_id)?.post_script_name == "AppleColorEmoji";
    color.then(|| glyph.x_advance * f32::from(size))
}

#[test]
fn an_emoji_advances_as_chrome_lays_it_out() {
    let mut fonts = FontSystem::new();
    // (size, width Chrome gives 🧬 at it)
    for (size, chrome) in [
        (10, 13.),
        (13, 16.),
        (14, 18.),
        (16, 20.),
        (18, 21.),
        (20, 22.),
        (24, 24.),
        (32, 32.),
    ] {
        let Some(width) = emoji_advance(&mut fonts, size) else {
            return;
        };
        assert_eq!(width, chrome, "at {size}px");
    }
}
