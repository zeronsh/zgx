use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use zgx_gpu::GpuRenderer;
fn main() {
    let gpu = GpuRenderer::new(64, 64).unwrap();
    let fonts_rc = gpu.text_system();
    let mut fonts = fonts_rc.borrow_mut();
    let sf = fonts.db().faces().find(|f| f.post_script_name.starts_with(".SFNS-")).and_then(|f| f.families.first().map(|(n, _)| n.clone())).unwrap();
    let t = std::env::args().nth(1).unwrap_or("The quick brown fox jumps over the lazy dog, complex revenue-by-region.png".into());
    let size: f32 = std::env::args().nth(2).map_or(14., |s| s.parse().unwrap());
    for w in [400u16, 450, 500, 550, 600, 700] {
        let mut out = vec![];
        for (label, fam) in [("generic", Family::SansSerif), ("explicit", Family::Name(&sf)), ("system-ui", Family::Name("system-ui")), ("Geist", Family::Name("Geist"))] {
            let mut b = Buffer::new(&mut fonts, Metrics::new(size, size * 1.4));
            b.set_text(&t, &Attrs::new().family(fam).weight(Weight(w)).optical(size.round() as u16), Shaping::Advanced, None);
            b.shape_until_scroll(&mut fonts, false);
            out.push(format!("{label} {:.3}", b.layout_runs().next().unwrap().line_w));
        }
        println!("{w}: {}", out.join("  "));
    }
}
