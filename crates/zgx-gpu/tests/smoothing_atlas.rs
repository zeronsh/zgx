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

const LATIN: &str = "The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs! \
0123456789 (){}[]<>=+-*/%&|^~#@$; fn main() -> Result<(), Error> { let x = 42; }";
const CJK: &str = "你好世界我们的生活中有很多美好的东西学习工作朋友家人时间快乐幸福健康天气今天明天昨天\
東京大阪日本語の文章を書くことは難しいですが楽しいです漢字平仮名片仮名";

/// A page at 2x: a light band and a dark one, each four texts of Latin and
/// CJK at 12 to 24 points. The dark band's texts, for its ink to animate.
fn page(scene: &mut Scene, width: f32, height: f32) -> Vec<NodeId> {
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Column));
    let mut dark = Vec::new();
    for (band, (ground, ink)) in [
        (Color(255, 255, 255, 255), Color(20, 20, 20, 255)),
        (Color(24, 24, 28, 255), Color(235, 235, 235, 255)),
    ]
    .into_iter()
    .enumerate()
    {
        let parent = scene.append(
            scene.root(),
            NodeKind::Rect(ground),
            Style {
                width: Some(width),
                height: Some(height / 2.),
                ..Default::default()
            },
        );
        scene.set_kind(parent, NodeKind::Container(Layout::Column));
        for (row, size) in [12f32, 14., 18., 24.].into_iter().enumerate() {
            let first = if (band + row) % 2 == 0 { LATIN } else { CJK };
            let node = scene.append(
                parent,
                NodeKind::Text {
                    text: format!("{first} {LATIN} {CJK}").into(),
                    font_size: size,
                    color: ink,
                },
                Style {
                    width: Some(width),
                    ..Default::default()
                },
            );
            if band == 1 {
                dark.push(node);
            }
        }
    }
    dark
}

#[test]
fn a_colour_animating_through_smoothing_levels_does_not_grow_the_atlas() {
    text::set_font_smoothing(Some(true));
    let (width, height) = (1200., 800.);
    let all = [Rect::new(0., 0., width, height)];
    let mut gpu = GpuRenderer::new(2400, 1600).expect("a GPU adapter");
    gpu.set_scale_factor(2.);
    let mut scene = Scene::new(width, height);
    gpu.install_text(&mut scene);
    let dark = page(&mut scene, width, height);
    // Grays a smoothing level apart (linear-light luminance in quarters).
    let grays = [0, 137, 188, 225, 255];
    assert_eq!(
        grays.map(|g| text::smoothing(Color(g, g, g, 255))),
        [1, 2, 3, 4, 5],
        "a level apart"
    );
    scene.flush();
    gpu.render(&scene, &all).unwrap();
    // The page's first frame grows the atlas to hold it: from there, the
    // page's light band and the dark band's current level are what it holds.
    let first = gpu.debug_cache_stats().atlas_bytes;
    let mut previous = gpu.debug_cache_stats().atlas_entries;
    let mut fills = 0;
    for gray in (0..4).flat_map(|_| grays.into_iter().chain(grays.into_iter().rev())) {
        for &node in &dark {
            if let NodeKind::Text {
                text, font_size, ..
            } = scene.kind(node).clone()
            {
                let color = Color(gray, gray, gray, 255);
                scene.set_kind(
                    node,
                    NodeKind::Text {
                        text,
                        font_size,
                        color,
                    },
                );
            }
        }
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
