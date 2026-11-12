//! Where text sits as a browser draws it (zgx-cosmic-text's patch): a line's baseline on a whole pixel, and
//! a font's `trak` table applied at the size it is drawn.
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, fontdb};

const GEIST: &[u8] = include_bytes!("fonts/Geist-500-subset.ttf");

fn fonts(bytes: Vec<u8>) -> FontSystem {
    let mut db = fontdb::Database::new();
    db.load_font_data(bytes);
    FontSystem::new_with_locale_and_db("en-US".into(), db)
}

fn be16(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

fn be32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn units_per_em(font: &[u8]) -> u16 {
    let tables = be16(font, 4) as usize;
    (0..tables)
        .map(|i| 12 + 16 * i)
        .find(|&entry| &font[entry..entry + 4] == b"head")
        .map(|entry| be16(font, be32(font, entry + 8) as usize + 18))
        .expect("a head table")
}

/// `font` with an `opsz` axis and a `trak` table that tracks 8 points by nothing and 24 by `units` font units.
fn tracked(font: &[u8], units: i16) -> Vec<u8> {
    let mut trak = Vec::new();
    trak.extend(0x0001_0000u32.to_be_bytes());
    trak.extend([0, 0, 0, 12, 0, 0, 0, 0]);
    // One track, two sizes (8 and 24 points); offsets count from the table's start.
    trak.extend([0, 1, 0, 2]);
    trak.extend(28u32.to_be_bytes());
    trak.extend(0u32.to_be_bytes());
    trak.extend([1, 0]);
    trak.extend(36u16.to_be_bytes());
    trak.extend(0x0008_0000u32.to_be_bytes());
    trak.extend(0x0018_0000u32.to_be_bytes());
    trak.extend(0i16.to_be_bytes());
    trak.extend(units.to_be_bytes());
    // Version 1.0, axes at 16, one 20-byte axis, no named instances.
    let mut fvar: Vec<u8> = [1u16, 0, 16, 2, 1, 20, 0, 8]
        .iter()
        .flat_map(|field| field.to_be_bytes())
        .collect();
    fvar.extend(b"opsz");
    for value in [8., 16., 96.] {
        fvar.extend(((value * 65536.) as i32).to_be_bytes());
    }
    fvar.extend([0, 0, 1, 0]);
    let mut tables: Vec<([u8; 4], &[u8])> = (0..usize::from(be16(font, 4)))
        .map(|index| {
            let record = 12 + 16 * index;
            let (offset, len) = (
                be32(font, record + 8) as usize,
                be32(font, record + 12) as usize,
            );
            (
                font[record..record + 4].try_into().unwrap(),
                &font[offset..offset + len],
            )
        })
        .collect();
    tables.retain(|(tag, _)| tag != b"fvar");
    tables.push((*b"fvar", &fvar));
    tables.push((*b"trak", &trak));
    tables.sort_by_key(|(tag, _)| *tag);
    let count = tables.len();
    let selector = count.ilog2() as usize;
    let mut out = font[..4].to_vec();
    for field in [
        count,
        16 << selector,
        selector,
        (count - (1 << selector)) * 16,
    ] {
        out.extend((field as u16).to_be_bytes());
    }
    let mut data = Vec::new();
    for (tag, table) in &tables {
        out.extend(tag);
        out.extend([0; 4]); // Checksums go unchecked.
        out.extend(((12 + 16 * count + data.len()) as u32).to_be_bytes());
        out.extend((table.len() as u32).to_be_bytes());
        data.extend(*table);
        data.resize(data.len().next_multiple_of(4), 0);
    }
    out.extend(data);
    out
}

fn width(fonts: &mut FontSystem, size: f32, optical: u16, text: &str) -> f32 {
    let mut buffer = Buffer::new(fonts, Metrics::new(size, size * 1.5));
    let attrs = Attrs::new().family(Family::Name("Geist")).optical(optical);
    buffer.set_text(text, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(fonts, false);
    buffer
        .layout_runs()
        .map(|run| run.line_w)
        .fold(0., f32::max)
}

#[test]
fn a_fonts_tracking_applies_at_the_size_it_is_drawn() {
    let font = tracked(GEIST, -100);
    let upem = f32::from(units_per_em(&font));
    let (size, text) = (16., "iiiiiiiiii");
    let mut plain_fonts = fonts(GEIST.to_vec());
    let mut fonts = fonts(font);
    let untracked = width(&mut plain_fonts, size, size as u16, text);
    let sized = width(&mut fonts, size, size as u16, text);
    // 16 points is halfway from 8 to 24: half of -100 units a glyph.
    let expected = 10. * -50. * size / upem;
    assert!(
        (sized - untracked - expected).abs() < 0.05,
        "{untracked} became {sized}, expected {expected} more"
    );
    // Without an optical size the table is read at 12 points, as Core Text does.
    let unsized_ = width(&mut fonts, size, 0, text);
    let expected = 10. * -25. * size / upem;
    assert!((unsized_ - untracked - expected).abs() < 0.05);
}

#[test]
fn a_font_without_tracking_shapes_the_same_at_any_optical_size() {
    let mut fonts = fonts(GEIST.to_vec());
    let plain = width(&mut fonts, 16., 0, "Requests 48,213");
    let sized = width(&mut fonts, 16., 16, "Requests 48,213");
    assert_eq!(plain, sized);
}

#[test]
fn a_lines_baseline_is_on_a_whole_pixel_with_the_half_leading_floored() {
    let mut fonts = fonts(GEIST.to_vec());
    for size in [11., 12.5, 13., 14.5, 15., 17.3] {
        for line_height in [18., 19., 20., 21., 23.] {
            let mut buffer = Buffer::new(&mut fonts, Metrics::new(size, line_height));
            let attrs = Attrs::new().family(Family::Name("Geist"));
            buffer.set_text("one\ntwo\nthree", &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(&mut fonts, false);
            let ys: Vec<f32> = buffer.layout_runs().map(|run| run.line_y).collect();
            assert_eq!(ys.len(), 3);
            for y in &ys {
                assert_eq!(
                    y.fract(),
                    0.,
                    "size {size}, line height {line_height}: {ys:?}"
                );
            }
            for pair in ys.windows(2) {
                assert_eq!(pair[1] - pair[0], line_height, "size {size}: {ys:?}");
            }
        }
    }
}
