//! Prints the faces the system font database has for a family, with the
//! weight each is listed at and the axes of its font file, and the weight and
//! optical size the text engine draws a request of each weight at:
//! `cargo run -p zgx-gpu --example font_faces -- "Segoe UI Variable"`.
use cosmic_text::{Attrs, AttrsList, BufferLine, Family, FontSystem, LineEnding, Shaping, Weight};

fn main() {
    let wanted = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Segoe UI Variable".into());
    let mut fonts = FontSystem::new();
    for face in fonts.db().faces() {
        if !face.families.iter().any(|(name, _)| name.contains(&wanted)) {
            continue;
        }
        let axes = fonts
            .db()
            .with_face_data(face.id, |data, index| {
                ttf_parser::Face::parse(data, index).ok().map(|f| {
                    f.variation_axes()
                        .into_iter()
                        .map(|a| {
                            format!(
                                "{}:{}..{}",
                                String::from_utf8_lossy(&a.tag.to_bytes()),
                                a.min_value,
                                a.max_value
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
            })
            .flatten()
            .unwrap_or_default();
        println!(
            "face {:?} families {:?} weight {} index {} ps {} axes [{axes}]",
            face.id,
            face.families.iter().map(|f| &f.0).collect::<Vec<_>>(),
            face.weight.0,
            face.index,
            face.post_script_name,
        );
    }
    for weight in [400, 450, 500, 600, 700] {
        let attrs = Attrs::new()
            .family(Family::Name(&wanted))
            .weight(Weight(weight))
            .optical(14);
        let mut line = BufferLine::new(
            "Hamburgefonts",
            LineEnding::None,
            AttrsList::new(&attrs),
            Shaping::Advanced,
        );
        let width: f32 = line
            .shape(&mut fonts, 8)
            .spans
            .iter()
            .flat_map(|s| &s.words)
            .flat_map(|w| &w.glyphs)
            .map(|g| g.x_advance * 14.)
            .sum();
        println!("asked {weight}: width of \"Hamburgefonts\" at 14px {width:.3}");
    }
}
