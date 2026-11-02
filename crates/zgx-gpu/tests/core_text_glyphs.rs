//! A glyph drawn through Core Text (`core_text_glyph`, which draws the faces
//! swash can't: macOS 26's PingFang, `hvgl` only) matches swash's of the
//! same unhinted glyph where both can draw it, at subpixel offsets text
//! uses: from 18 px, the same coverage within antialiasing's differences,
//! placed alike within a tenth of a pixel; at 13 px, where Core Text draws
//! its own way (as macOS's own text does), placed within half a pixel and
//! as heavy within 12%.
#![cfg(target_os = "macos")]
use cosmic_text::{CacheKey, CacheKeyFlags, FontSystem, SwashCache, SwashImage, fontdb};

const DEJAVU: &[u8] = include_bytes!("../../../comparisons/fonts/DejaVuSans.ttf");

/// `image`'s coverage on a canvas whose top-left is (`left`, `top`) from
/// the pen (y up), `width` by `height`.
fn on_canvas(image: &SwashImage, left: i32, top: i32, width: usize, height: usize) -> Vec<f32> {
    let mut canvas = vec![0f32; width * height];
    let p = image.placement;
    for row in 0..p.height as usize {
        for column in 0..p.width as usize {
            let x = (p.left - left) as usize + column;
            let y = (top - p.top) as usize + row;
            canvas[y * width + x] = f32::from(image.data[row * p.width as usize + column]) / 255.;
        }
    }
    canvas
}

#[test]
fn core_texts_glyphs_match_swashs() {
    let mut db = fontdb::Database::new();
    db.load_font_data(DEJAVU.to_vec());
    let mut fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
    let id = fonts.db().faces().next().expect("DejaVu Sans").id;
    let font = fonts.get_font(id, fontdb::Weight::NORMAL).expect("a font");
    let charmap = font.as_swash().charmap();
    let mut swash = SwashCache::new();
    for size in [13., 18., 24., 48.] {
        for x in [0., 0.25, 0.5] {
            for c in ['a', 'g', 'W', '8', '@'] {
                let (key, _, _) = CacheKey::new(
                    id,
                    charmap.map(c),
                    size,
                    (x, 0.),
                    fontdb::Weight::NORMAL,
                    0,
                    // Unhinted: Core Text doesn't hint, and swash's hints
                    // move TrueType outlines a pixel at small sizes.
                    CacheKeyFlags::DISABLE_HINTING,
                );
                let what = format!("{c:?} at {size} px, x + {x}");
                let theirs = swash
                    .get_image_uncached(&mut fonts, key)
                    .expect("swash draws it");
                let ours = zgx_gpu::core_text_glyph(&mut fonts, key).expect("Core Text draws it");
                let (a, b) = (theirs.placement, ours.placement);
                // Placed alike: the inked part where swash has it, give or
                // take the pixel of room each leaves round its box.
                let left = a.left.min(b.left);
                let top = a.top.max(b.top);
                let right = (a.left + a.width as i32).max(b.left + b.width as i32);
                let bottom = (a.top - a.height as i32).min(b.top - b.height as i32);
                let (width, height) = ((right - left) as usize, (top - bottom) as usize);
                let theirs = on_canvas(&theirs, left, top, width, height);
                let ours = on_canvas(&ours, left, top, width, height);
                let ink: f32 = theirs.iter().sum();
                let difference: f32 = theirs.iter().zip(&ours).map(|(a, b)| (a - b).abs()).sum();
                let ours_ink: f32 = ours.iter().sum();
                assert!(ink > 1., "{what}: swash drew nothing");
                let centroid = |mask: &[f32]| {
                    let (mut x, mut y, mut total) = (0., 0., 0.);
                    for (at, coverage) in mask.iter().enumerate() {
                        x += coverage * (at % width) as f32;
                        y += coverage * (at / width) as f32;
                        total += coverage;
                    }
                    (x / total, y / total)
                };
                let (a, b) = (centroid(&theirs), centroid(&ours));
                let shift = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
                let (spread, weight) = (difference / ink, (ours_ink - ink).abs() / ink);
                if size >= 18. {
                    // Coverage within 15% of the glyph's ink, ink within
                    // 3%, placed within a tenth of a pixel.
                    assert!(
                        spread < 0.15,
                        "{what}: differs by {:.1}% of its ink",
                        100. * spread
                    );
                    assert!(weight < 0.03, "{what}: ink {ours_ink:.1} against {ink:.1}");
                    assert!(shift < 0.1, "{what}: {shift:.2} px from swash's");
                } else {
                    // Small, Core Text draws its own way (a little heavier,
                    // up to half a pixel off, as macOS's own text): placed
                    // within half a pixel, ink within 12%.
                    assert!(weight < 0.12, "{what}: ink {ours_ink:.1} against {ink:.1}");
                    assert!(shift < 0.5, "{what}: {shift:.2} px from swash's");
                }
            }
        }
    }
}
