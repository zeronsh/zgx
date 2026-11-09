//! A colour glyph (COLR) whose layer covers a pixel in an opaque colour
//! draws that pixel opaque, in that colour: its layers composited over one
//! another divide by 255 (`zgx-swash`'s `blit`). Divided by 256, a fully
//! covered pixel came out at alpha 253, so no colour glyph (Segoe UI Emoji's,
//! every emoji on Windows) had an opaque pixel. The font is one red square
//! (`fonts/make_colr_square.py`).
use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, fontdb,
};

#[test]
fn a_colour_layer_covering_a_pixel_draws_it_opaque() {
    let mut db = fontdb::Database::new();
    db.load_font_data(include_bytes!("fonts/ZgxColrSquare.ttf").to_vec());
    let mut fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
    let mut buffer = Buffer::new(&mut fonts, Metrics::new(64., 80.));
    buffer.set_size(Some(200.), Some(100.));
    buffer.set_text(
        "\u{E000}",
        &Attrs::new().family(Family::Name("ZgxColrSquare")),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(&mut fonts, false);
    let run = buffer.layout_runs().next().expect("a line");
    let glyph = run
        .glyphs
        .first()
        .expect("the square")
        .physical((0., 0.), 1.);
    let image = SwashCache::new()
        .get_image_uncached(&mut fonts, glyph.cache_key)
        .expect("an image");
    assert_eq!(
        image.content,
        SwashContent::Color,
        "drawn from its colour layers"
    );
    let (w, h) = (
        image.placement.width as usize,
        image.placement.height as usize,
    );
    // Its inside, a pixel in from each edge: covered whole.
    let mut inside = 0;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let pixel = &image.data[(y * w + x) * 4..][..4];
            assert_eq!(pixel, [255, 0, 0, 255], "at ({x}, {y}) of {w} by {h}");
            inside += 1;
        }
    }
    assert!(inside > 1000, "{inside} pixels inside");
}
