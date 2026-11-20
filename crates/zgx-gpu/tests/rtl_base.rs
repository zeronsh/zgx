//! A paragraph forced right to left (`FontStyle::paragraph_rtl`, CSS
//! `direction: rtl`) shapes at the right to left base level even when it
//! starts with Latin letters: its closing full stop lands at the left, as in
//! Chrome's right to left paragraph; a left to right one keeps it at the
//! right.
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};

fn stop_is_at_left(rtl: bool) -> bool {
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_font_data(include_bytes!("fonts/GeistVariable-subset.ttf").to_vec());
    let mut fonts = FontSystem::new_with_locale_and_db("en".into(), db);
    let mut buffer = Buffer::new(&mut fonts, Metrics::new(16., 20.));
    let attrs = Attrs::new()
        .family(Family::Name("Geist"))
        .paragraph_rtl(rtl);
    buffer.set_text("Hello world.", &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(&mut fonts, false);
    let run = buffer.layout_runs().next().expect("a line");
    let text = run.text;
    let stop = run
        .glyphs
        .iter()
        .find(|g| &text[g.start..g.end] == ".")
        .expect("the full stop");
    let h = run
        .glyphs
        .iter()
        .find(|g| &text[g.start..g.end] == "H")
        .expect("the H");
    stop.x < h.x
}

#[test]
fn latin_text_in_a_right_to_left_paragraph_places_its_full_stop_at_the_left() {
    assert!(!stop_is_at_left(false), "left to right: Hello world.");
    assert!(stop_is_at_left(true), "right to left: .Hello world");
}
