//! macOS draws its text unhinted at 2x: Core Text's glyphs and the
//! renderer's, keyed as the renderer keys them at that scale, are the same
//! to a few hundredths of a pixel's coverage. Hinted (swash's hinting,
//! skrifa's autohinter on faces with no instructions such as SF), they were
//! 0.13 apart on average. At 1x CoreGraphics autohints whole pixel sizes
//! from 7 to 18 its own way: the renderer keeps swash's hinting there.
#![cfg(target_os = "macos")]
use cosmic_text::{CacheKeyFlags, SwashImage};
use std::sync::Arc;
use zgx::text_layout::FontStyle;
use zgx_gpu::GpuRenderer;

const LINES: [&str; 2] = [
    "The quick brown fox jumps over the lazy dog. 0123456789 (){}[]<>=+-*/%&|^~#@$",
    "你好世界我们的生活中有很多美好的东西学习工作朋友家人时间快乐幸福健康",
];

/// Coverage, 0 to 1.
fn ink(image: &SwashImage) -> f32 {
    image.data.iter().map(|&a| f32::from(a) / 255.).sum()
}

/// The mean coverage difference between `a` and `b` over the pixels either
/// covers, placed by their placements.
fn mean_diff(a: &SwashImage, b: &SwashImage) -> f32 {
    let (pa, pb) = (a.placement, b.placement);
    let left = pa.left.min(pb.left);
    let top = pa.top.max(pb.top);
    let right = (pa.left + pa.width as i32).max(pb.left + pb.width as i32);
    let bottom = (pa.top - pa.height as i32).min(pb.top - pb.height as i32);
    let (w, h) = ((right - left) as usize, (top - bottom) as usize);
    let canvas = |image: &SwashImage| {
        let p = image.placement;
        let mut c = vec![0f32; w * h];
        for row in 0..p.height as usize {
            for col in 0..p.width as usize {
                c[((top - p.top) as usize + row) * w + (p.left - left) as usize + col] =
                    f32::from(image.data[row * p.width as usize + col]) / 255.;
            }
        }
        c
    };
    let (ca, cb) = (canvas(a), canvas(b));
    let (sum, n) = ca
        .iter()
        .zip(&cb)
        .filter(|(x, y)| **x > 0. || **y > 0.)
        .fold((0f32, 0usize), |(s, n), (x, y)| (s + (x - y).abs(), n + 1));
    if n == 0 { 0. } else { sum / n as f32 }
}

#[test]
fn at_2x_glyphs_are_unhinted_and_match_core_text() {
    let gpu = GpuRenderer::new(16, 16).expect("a GPU adapter");
    let (fonts, cache) = (gpu.text_system(), gpu.text_cache());
    let (mut fonts, mut cache) = (fonts.borrow_mut(), cache.borrow_mut());
    let mut swash = cosmic_text::SwashCache::new();
    let (mut diffs, mut ours, mut theirs) = (Vec::new(), 0f32, 0f32);
    for size in [13f32, 16.] {
        for line in LINES {
            let text: Arc<str> = line.into();
            let (places, ..) = zgx_gpu::plain_glyph_places(
                &mut fonts,
                &mut cache,
                &text,
                size,
                &FontStyle::default(),
                10_000.,
                100.,
                2.,
            )
            .expect("plain text");
            for (key, ..) in places {
                assert!(
                    key.flags.contains(CacheKeyFlags::DISABLE_HINTING),
                    "keyed unhinted at 2x"
                );
                // Smoothing off: the rasterizers and their hinting alone.
                let key = zgx_gpu::text::smoothed(key, 0);
                let (Some(a), Some(b)) = (
                    zgx_gpu::glyph_image(&mut swash, &mut fonts, key),
                    zgx_gpu::core_text_glyph(&mut fonts, key),
                ) else {
                    continue;
                };
                if a.content != cosmic_text::SwashContent::Mask || ink(&b) < 0.5 {
                    continue;
                }
                ours += ink(&a);
                theirs += ink(&b);
                diffs.push(mean_diff(&a, &b));
            }
        }
    }
    assert!(diffs.len() > 100, "{} glyphs compared", diffs.len());
    let mean = diffs.iter().sum::<f32>() / diffs.len() as f32;
    let ink = (ours - theirs) / theirs;
    // 0.020 measured (hinted: 0.126), ink within 0.1%.
    assert!(
        mean < 0.025,
        "mean coverage difference {mean:.4} over {} glyphs",
        diffs.len()
    );
    assert!(ink.abs() < 0.01, "ink {:+.2}%", ink * 100.);
}

#[test]
fn at_1x_glyphs_keep_their_hinting() {
    let gpu = GpuRenderer::new(16, 16).expect("a GPU adapter");
    let (fonts, cache) = (gpu.text_system(), gpu.text_cache());
    let (mut fonts, mut cache) = (fonts.borrow_mut(), cache.borrow_mut());
    let text: Arc<str> = LINES[0].into();
    let (places, ..) = zgx_gpu::plain_glyph_places(
        &mut fonts,
        &mut cache,
        &text,
        13.,
        &FontStyle::default(),
        10_000.,
        100.,
        1.,
    )
    .expect("plain text");
    assert!(!places.is_empty());
    assert!(
        places
            .iter()
            .all(|(key, ..)| !key.flags.contains(CacheKeyFlags::DISABLE_HINTING))
    );
}
