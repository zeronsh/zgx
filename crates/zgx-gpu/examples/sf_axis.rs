use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight};
fn main() {
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_system_fonts();
    let mut fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
    let family = fonts.db().faces().find(|f| f.post_script_name.starts_with(".SFNS-")).and_then(|f| f.families.first().map(|(n, _)| n.clone())).unwrap();
    let t = "revenue-by-region.png Hamburgefonstiv";
    for w in (100..=1000).step_by(25) {
        let mut b = Buffer::new(&mut fonts, Metrics::new(100., 120.));
        b.set_text(t, &Attrs::new().family(Family::Name(&family)).weight(Weight(w)), Shaping::Advanced, None);
        b.shape_until_scroll(&mut fonts, false);
        println!("{w}: {:.3}", b.layout_runs().next().unwrap().line_w);
    }
}
