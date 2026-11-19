//! A colour animating through font smoothing's levels draws its text's
//! glyphs again at each level it passes, and leaves the last level's
//! behind: the glyph atlas gives those back as it fills rather than
//! growing, as it would for new text. Grown, it would stay grown, and a
//! grown and shrunk Metal texture leaves its pages in the footprint (some
//! 19 MB after a page's colour tween). A binary of its own: the smoothing
//! setting is the process's.
#![cfg(target_os = "macos")]
use zgx::scene::*;
use zgx_gpu::{GpuRenderer, text};

#[test]
fn a_colour_animating_through_smoothing_levels_does_not_grow_the_atlas() {
    text::set_font_smoothing(Some(true));
    let (width, height) = (2800., 130.);
    let all = [Rect::new(0., 0., width, height)];
    let mut gpu = GpuRenderer::new(width as u32, height as u32).expect("a GPU adapter");
    let mut scene = Scene::new(width, height);
    gpu.install_text(&mut scene);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    // Grays a smoothing level apart (linear-light luminance in quarters),
    // over black: a line whose glyphs take nearly half the atlas at each.
    let line = |gray: u8| NodeKind::Text {
        text: "The quick brown fox jumps over the lazy dog 0123456789 ABCDEFG".into(),
        font_size: 84.,
        color: Color(gray, gray, gray, 255),
    };
    let grays = [0, 137, 188, 225, 255];
    assert_eq!(
        grays.map(|g| text::smoothing(Color(g, g, g, 255))),
        [1, 2, 3, 4, 5],
        "a level apart"
    );
    let style = Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    };
    let node = scene.append(scene.root(), line(grays[0]), style);
    scene.flush();
    gpu.render(&scene, &all).unwrap();
    let first = gpu.debug_cache_stats().atlas_bytes;
    let mut previous = gpu.debug_cache_stats().atlas_entries;
    let mut fills = 0;
    for gray in (0..4).flat_map(|_| grays.into_iter().chain(grays.into_iter().rev())) {
        scene.set_kind(node, line(gray));
        scene.flush();
        gpu.render(&scene, &all).unwrap();
        let entries = gpu.debug_cache_stats().atlas_entries;
        fills += usize::from(entries < previous);
        previous = entries;
    }
    text::set_font_smoothing(None);
    assert!(fills > 2, "the atlas filled again and again: {fills}");
    assert_eq!(gpu.debug_cache_stats().atlas_bytes, first, "grew");
}
