//! The software raster's bundled font, what plain text falls back to without
//! its scene's text services, is parsed the first time text draws with it,
//! once for the process: rasters that draw none never hold it (parsed, every
//! glyph's outline, about 17 MB). A binary of its own: the font is the
//! process's.
use zgx::scene::{Color, NodeKind, Rect, Scene, Style};
use zgx_desktop::raster::{Raster, bundled_font_parsed};

#[test]
fn the_bundled_font_is_parsed_when_text_first_needs_it() {
    let mut scene = Scene::new(120., 40.);
    scene.append(
        scene.root(),
        NodeKind::Rect(Color(255, 0, 0, 255)),
        Style {
            width: Some(20.),
            height: Some(20.),
            ..Default::default()
        },
    );
    let full = [Rect::new(0., 0., 120., 40.)];
    let _ = scene.flush();
    let mut first = Raster::new(120, 40);
    first.render(&scene, &full);
    let mut second = Raster::new(120, 40);
    second.render(&scene, &full);
    assert!(!bundled_font_parsed(), "no text drawn, no font parsed");
    scene.append(
        scene.root(),
        NodeKind::Text {
            text: "MMMM".into(),
            color: Color(255, 255, 255, 255),
            font_size: 20.,
        },
        Style {
            width: Some(100.),
            height: Some(20.),
            ..Default::default()
        },
    );
    let _ = scene.flush();
    first.render(&scene, &full);
    assert!(bundled_font_parsed(), "parsed for the first text drawn");
    let white = |raster: &Raster| {
        raster
            .pixels
            .iter()
            .any(|p| p & 0xff > 200 && (p >> 8) & 0xff > 200 && (p >> 16) & 0xff > 200)
    };
    assert!(white(&first), "its glyphs drawn");
    // The other raster draws them from the same font.
    second.render(&scene, &full);
    assert!(white(&second));
}
