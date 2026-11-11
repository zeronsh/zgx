//! Direct3D 11's glyph atlas (`ZGX_D3D11=1`, the `d3d11` feature) shrinks
//! at idle as Direct3D 12's does (`GpuRenderer::trim_atlas`): its glyphs are
//! copied into the smaller atlas, so the next frame draws the same picture
//! without rasterizing them again. `ZGX_D3D11` is read once a process, so
//! this test is one of its own, setting it first.
#[cfg(windows)]
use zgx::scene::*;
#[cfg(windows)]
use zgx_gpu::GpuRenderer;

#[cfg(windows)]
fn fixed(w: f32, h: f32) -> Style {
    Style {
        width: Some(w),
        height: Some(h),
        ..Default::default()
    }
}

/// Five lines of large letters, enough to grow the atlas past its first
/// size (as `pixels.rs`'s `large_letters`).
#[cfg(windows)]
fn large_letters(scene: &mut Scene) -> Vec<NodeId> {
    (0..5)
        .map(|line| {
            let text: String = (0..14)
                .map(|i| (b'A' + (line * 5 + i) % 26) as char)
                .collect();
            scene.append(
                scene.root(),
                NodeKind::Text {
                    text: text.as_str().into(),
                    font_size: 120. + f32::from(line) * 7.,
                    color: Color(255, 255, 255, 255),
                },
                fixed(1200., 170.),
            )
        })
        .collect()
}

#[cfg(windows)]
fn main() {
    // SAFETY: Set before anything else runs, the test's only thread.
    unsafe { std::env::set_var("ZGX_D3D11", "1") };
    let mut scene = Scene::new(1200., 900.);
    let large = large_letters(&mut scene);
    let mut gpu = GpuRenderer::new(1200, 900).unwrap();
    // Drawn by Direct3D 11: wgpu runs on its no-op backend then.
    assert_eq!(gpu.adapter_info().backend, zgx_gpu::wgpu::Backend::Noop);
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    let first_size = 512 * 512 * 4;
    let grown = gpu.debug_cache_stats().atlas_bytes;
    assert!(grown > first_size, "the large letters grew the atlas");
    for id in large {
        scene.remove(id);
    }
    scene.append(
        scene.root(),
        NodeKind::Text {
            text: "The quick brown fox jumps over the lazy dog".into(),
            font_size: 16.,
            color: Color(255, 255, 255, 255),
        },
        fixed(1200., 40.),
    );
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    let before = gpu.readback().unwrap();
    assert!(
        before.as_chunks::<4>().0.iter().any(|pixel| pixel[0] > 128),
        "the small line drew"
    );
    gpu.trim();
    gpu.trim_atlas();
    assert_eq!(gpu.debug_cache_stats().atlas_bytes, first_size);
    let whole = [Rect::new(0., 0., 1200., 900.)];
    let stats = gpu.render(&scene, &whole).unwrap();
    assert_eq!(stats.glyph_uploads, 0, "drawn from the glyphs copied");
    assert!(gpu.readback().unwrap() == before, "the same picture");
    println!("d3d11_atlas: the atlas shrank and drew the same picture");
}

#[cfg(not(windows))]
fn main() {}
