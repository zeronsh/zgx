use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use zgx_gpu::GpuRenderer;
fn main() {
    let gpu = GpuRenderer::new(64, 64).unwrap();
    let fonts_rc = gpu.text_system();
    let mut fonts = fonts_rc.borrow_mut();
    fonts.db_mut().load_font_file(std::env::args().nth(1).unwrap()).unwrap();
    let fam = fonts.db().faces().find(|f| f.post_script_name.starts_with("Inter")).map(|f| f.families[0].0.clone()).unwrap();
    println!("family {fam}");
    for (text, size, w) in [
        ("Quarterly revenue report, PDF", 13.0f32, 400u16),
        ("Quarterly revenue report, PDF", 14.0, 400),
        ("Quarterly revenue report, PDF", 13.0, 500),
        ("PDF,", 13.0, 400),
        ("A long title about the economics of renewable energy transitions, 2025", 14.0, 400),
        ("AV To Ty We r. y, F. P,", 13.0, 400),
    ] {
        for (label, kern) in [("default", None), ("kern0", Some(0)), ("kern1", Some(1))] {
            let mut ff = cosmic_text::FontFeatures::new();
            if let Some(v) = kern { ff.set(cosmic_text::FeatureTag::new(b"kern"), v); }
            let mut b = Buffer::new(&mut fonts, Metrics::new(size, size * 1.5));
            b.set_text(text, &Attrs::new().family(Family::Name(&fam)).weight(Weight(w)).font_features(ff), Shaping::Advanced, None);
            b.shape_until_scroll(&mut fonts, false);
            println!("{size}px {w} {label}: {:.3}  {text:?}", b.layout_runs().next().unwrap().line_w);
        }
    }
}
