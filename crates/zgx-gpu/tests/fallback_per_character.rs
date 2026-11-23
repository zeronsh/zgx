//! A character a word's face lacks is drawn from a fallback face alone:
//! the rest of the word stays in the face asked for, or, where its family
//! isn't installed, in the face the words beside it fall back to. And on
//! macOS a sans face's fallback is the system's sans, as AppKit's is, not
//! Chrome's Times for Latin text.
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use zgx_gpu::GpuRenderer;
use zgx_gpu::text::FontData;

/// Geist without `…` (U+2026), as the subset is.
const GEIST: &[u8] = include_bytes!("fonts/GeistVariable-subset.ttf");
const LINES: [&str; 3] = ["Ask anything, @ to mention…", "Loading…", "Thinking…"];

/// zgx's font system, with the Geist subset installed.
fn renderer() -> GpuRenderer {
    let gpu = GpuRenderer::new(16, 16).expect("a GPU adapter");
    FontData::new(GEIST.to_vec())
        .unwrap()
        .install(&mut gpu.text_system().borrow_mut());
    gpu
}

/// Each glyph of `text` asked for in `family`: its text, the PostScript name
/// of the face it is drawn from, and its glyph.
fn drawn_in(fonts: &mut FontSystem, family: Family, text: &str) -> Vec<(String, String, u16)> {
    let mut buffer = Buffer::new(fonts, Metrics::new(20., 24.));
    buffer.set_text(text, &Attrs::new().family(family), Shaping::Advanced, None);
    buffer.shape_until_scroll(fonts, false);
    let mut found = Vec::new();
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let face = fonts.db().face(glyph.font_id).expect("a face");
            found.push((
                text[glyph.start..glyph.end].to_string(),
                face.post_script_name.clone(),
                glyph.glyph_id,
            ));
        }
    }
    found
}

#[test]
fn a_word_keeps_its_face_for_all_but_the_character_it_lacks() {
    let gpu = renderer();
    let fonts = gpu.text_system();
    let mut fonts = fonts.borrow_mut();
    for line in LINES {
        let glyphs = drawn_in(&mut fonts, Family::Name("Geist"), line);
        for (text, face, glyph) in &glyphs {
            if text == "…" {
                assert_ne!(face, "Geist-Regular", "{line}: … from a face that has it");
                assert_ne!(*glyph, 0, "{line}: … drawn");
            } else {
                assert_eq!(
                    face, "Geist-Regular",
                    "{line}: {text} kept in Geist: {glyphs:?}"
                );
            }
        }
    }
}

/// A family that isn't installed: each word of the line in the face its
/// search falls back to, the words with `…` as the rest, and only the `…`
/// where that face lacks it from another. The platform's face for the `…`
/// took the whole word: Times, Chrome's standard font, which a Chrome
/// font system still asks first.
#[cfg(target_os = "macos")]
#[test]
fn a_missing_family_falls_back_per_character() {
    for chrome in [false, true] {
        let gpu = renderer();
        let fonts = gpu.text_system();
        let mut fonts = fonts.borrow_mut();
        if chrome {
            zgx_gpu::install_chrome_core_text_fallback(&mut fonts);
        }
        for family in [Family::Name("NoSuchFamily"), Family::SansSerif] {
            for line in LINES {
                let glyphs = drawn_in(&mut fonts, family, line);
                let first = &glyphs[0].1;
                let in_line = |(text, face, glyph): &(String, String, u16)| {
                    *glyph != 0 && (face == first || chrome && text == "…")
                };
                assert!(
                    glyphs.iter().all(in_line),
                    "chrome {chrome}, {family:?}: {glyphs:?}"
                );
            }
        }
    }
}

/// A sans face's missing character comes from the face Core Text gives the
/// system font for it (SF), at the text's weight.
#[cfg(target_os = "macos")]
#[test]
fn a_sans_faces_missing_character_falls_back_to_the_system_sans() {
    let gpu = renderer();
    let fonts = gpu.text_system();
    let mut fonts = fonts.borrow_mut();
    let (system, _) = zgx_gpu::core_text_advance('…', 400, 20.).expect("Core Text's face");
    for line in LINES {
        let glyphs = drawn_in(&mut fonts, Family::Name("Geist"), line);
        let (_, face, _) = glyphs.iter().find(|(text, ..)| text == "…").expect("a …");
        assert_eq!(face, &system, "{line}: {glyphs:?}");
    }
}
