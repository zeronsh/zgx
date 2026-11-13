//! Glyph masks from swash or Core Text composited on the CPU into the
//! raster-floor probe (`raster_cpu swash|cg out.png [gamma]`), to compare
//! with Chrome's frame of the same lines.
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent};

const GEIST: &[u8] = include_bytes!("../tests/fonts/GeistVariable-subset.ttf");
const TEXT: &str = "Hello hamburgefonstiv. The quick brown fox";

fn main() {
    let mode = std::env::args().nth(1).unwrap();
    let out = std::env::args().nth(2).unwrap();
    let gamma: f32 = std::env::args().nth(3).map_or(1., |g| g.parse().unwrap());
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_font_data(GEIST.to_vec());
    let sf_mode = std::env::var("SF").is_ok();
    if sf_mode {
        db.load_system_fonts();
    }
    let mut fonts = FontSystem::new_with_locale_and_db("en".into(), db);
    let family_name = if sf_mode {
        fonts.db().faces().find(|f| f.post_script_name.starts_with(".SFNS-")).and_then(|f| f.families.first().map(|(n, _)| n.clone())).unwrap()
    } else {
        "Geist".to_string()
    };
    let weight: u16 = std::env::var("WEIGHT").map_or(400, |w| w.parse().unwrap());
    let text: String = std::env::var("TEXT").unwrap_or_else(|_| TEXT.to_string());
    let mut swash = SwashCache::new();
    let (w, h) = (800usize, 280usize);
    let mut frame = vec![255u8; w * h];
    for (x_css, top, white) in [(10., 10., false), (10.25, 40., false), (10.5, 70., false)] {
        let _ = white;
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(14., 20.));
        buffer.set_text(&text, &Attrs::new().family(Family::Name(&family_name)).weight(cosmic_text::Weight(weight)), Shaping::Advanced, None);
        buffer.set_size(Some(380.), None);
        buffer.shape_until_scroll(&mut fonts, false);
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let p = glyph.physical((x_css * 2., 0.), 2.);
                let image = match mode.as_str() {
                    "cg" => zgx_gpu::core_text_glyph(&mut fonts, p.cache_key),
                    m if m.starts_with("probe") => {
                        zgx_gpu::core_text_probe(&mut fonts, p.cache_key, m[5..].parse().unwrap())
                    }
                    _ => swash.get_image_uncached(&mut fonts, p.cache_key),
                };
                let Some(image) = image else { continue };
                if image.content != SwashContent::Mask {
                    continue;
                }
                let base_y = ((top + run.line_y) * 2.0f32).round() as i32 + p.y;
                for row in 0..image.placement.height as i32 {
                    for col in 0..image.placement.width as i32 {
                        let a = image.data[(row * image.placement.width as i32 + col) as usize];
                        let x = p.x + col + image.placement.left;
                        let y = base_y + row - image.placement.top;
                        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 || a == 0 {
                            continue;
                        }
                        let a = (a as f32 / 255.).powf(gamma);
                        let px = &mut frame[y as usize * w + x as usize];
                        *px = (*px as f32 * (1. - a)).round() as u8;
                    }
                }
            }
        }
    }
    image::save_buffer(&out, &frame, w as u32, h as u32, image::ColorType::L8).unwrap();
}
