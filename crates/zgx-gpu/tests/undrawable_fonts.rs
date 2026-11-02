//! A face with no outlines swash can draw (macOS 26's PingFangUI keeps its
//! glyphs' shapes only in Apple's `hvgl` table) is no font: text asking for
//! it, or that only it and another face cover, takes the next face that
//! covers it, which draws. Without that, the face's glyphs shaped and drew
//! as blank space: every Chinese character on macOS 26.
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, fontdb};

const DEJAVU: &[u8] = include_bytes!("../../../comparisons/fonts/DejaVuSans.ttf");
const GEIST: &[u8] = include_bytes!("fonts/GeistVariable-subset.ttf");

/// `font` with its `glyf` table's tag renamed (to `glyz`, which sorts where
/// `glyf` did, keeping the table directory in order): its glyphs map and
/// measure as before, but nothing draws them, as with an `hvgl`-only face.
fn without_outlines(font: &[u8]) -> Vec<u8> {
    let mut out = font.to_vec();
    let tables = usize::from(u16::from_be_bytes([font[4], font[5]]));
    let record = (0..tables)
        .map(|i| 12 + 16 * i)
        .find(|at| &font[*at..*at + 4] == b"glyf")
        .expect("a TrueType font");
    out[record..record + 4].copy_from_slice(b"glyz");
    out
}

/// A font system of `fonts` alone.
fn system(fonts: &[Vec<u8>]) -> FontSystem {
    let mut db = fontdb::Database::new();
    for font in fonts {
        db.load_font_data(font.clone());
    }
    FontSystem::new_with_locale_and_db("en-US".into(), db)
}

/// The family of the face each glyph of `text`, asked for in `family`, is
/// drawn from.
fn drawn_in(fonts: &mut FontSystem, family: &str, text: &str) -> Vec<String> {
    let mut buffer = Buffer::new(fonts, Metrics::new(20., 24.));
    let attrs = Attrs::new().family(Family::Name(family));
    buffer.set_text(text, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(fonts, false);
    let mut found = Vec::new();
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let face = fonts.db().face(glyph.font_id).expect("a face");
            found.push(face.families[0].0.clone());
        }
    }
    found
}

#[test]
fn a_face_without_outlines_to_draw_is_passed_by_for_one_that_draws() {
    let stripped = without_outlines(DEJAVU);
    let mut fonts = system(&[stripped, GEIST.to_vec()]);
    let id = (fonts.db().faces())
        .find(|face| face.families[0].0 == "DejaVu Sans")
        .expect("loaded")
        .id;
    // It loads into the database (its names and characters are there), but
    // gives no font to shape or draw with.
    assert!(fonts.get_font(id, fontdb::Weight::NORMAL).is_none());
    let found = drawn_in(&mut fonts, "DejaVu Sans", "Hello");
    assert!(
        !found.is_empty() && found.iter().all(|family| family == "Geist"),
        "the next face that covers it: {found:?}"
    );
    // Control: with its outlines, the face asked for draws its text.
    let mut fonts = system(&[DEJAVU.to_vec(), GEIST.to_vec()]);
    let found = drawn_in(&mut fonts, "DejaVu Sans", "Hello");
    assert!(
        !found.is_empty() && found.iter().all(|family| family == "DejaVu Sans"),
        "{found:?}"
    );
}
