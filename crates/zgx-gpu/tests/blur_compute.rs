//! A backdrop blur's stages as one compute pass (blur.wgsl's `cs_*`) draw
//! the same bytes as the render passes they replace, which remain where an
//! adapter can't run them (`blur_compute`): frosted panels at 1×, 2× and
//! 3×, narrow, wide and fading blurs, drawn through wgpu.
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

fn fixed(w: f32, h: f32) -> Style {
    Style {
        width: Some(w),
        height: Some(h),
        ..Default::default()
    }
}

/// Stripes and text under a rounded frosted panel off the pixel grid.
fn scene(sigma: f32, fade: f32) -> Scene {
    let mut s = Scene::new(160., 100.);
    s.set_kind(s.root(), NodeKind::Container(Layout::Overlay));
    for i in 0..16 {
        let stripe = s.append(
            s.root(),
            NodeKind::Rect(Color((i * 37 % 255) as u8, (i * 91 % 255) as u8, 200, 255)),
            fixed(6., 100.),
        );
        s.set_transform(
            stripe,
            Transform {
                x: i as f32 * 10.,
                y: 0.,
            },
        );
    }
    let text = s.append(
        s.root(),
        NodeKind::Text {
            text: "Under the glass".into(),
            color: Color(255, 255, 255, 255),
            font_size: 14.,
        },
        fixed(150., 20.),
    );
    s.set_transform(text, Transform { x: 6., y: 55. });
    let glass = s.append(
        s.root(),
        NodeKind::Quad(QuadStyle {
            radius: 12.,
            ..Default::default()
        }),
        fixed(120., 60.),
    );
    s.set_transform(glass, Transform { x: 18.5, y: 20.25 });
    s.set_effects(
        glass,
        Effects {
            blur_radius: sigma,
            edge_fade: fade,
            ..Default::default()
        },
    );
    s
}

/// A frame through wgpu, its stages as passes or not, and its blur passes.
fn frame(passes: bool, scale: f32, sigma: f32, fade: f32) -> (Vec<u8>, usize, bool) {
    zgx_gpu::debug_blur_passes(passes);
    let mut gpu = GpuRenderer::new((160. * scale) as u32, (100. * scale) as u32).unwrap();
    gpu.set_direct_metal(false);
    gpu.set_scale_factor(scale);
    let compute = gpu.debug_blur_compute();
    let mut s = scene(sigma, fade);
    let damage = s.flush().damage;
    let stats = gpu.render(&s, &damage).unwrap();
    assert_eq!(gpu.debug_direct_frames(), 0, "drawn through wgpu");
    (gpu.readback().unwrap(), stats.blur_passes, compute)
}

#[test]
fn compute_stages_draw_the_render_passes_bytes() {
    for scale in [1., 2., 3.] {
        for (sigma, fade) in [(2., 0.), (9., 0.), (24., 0.), (10., 24.)] {
            let (stages, stage_passes, compute) = frame(false, scale, sigma, fade);
            let (passes, render_passes, fallback) = frame(true, scale, sigma, fade);
            assert!(!fallback, "the switch takes the render passes");
            // Metal has them: what's compared there is both paths.
            assert!(
                compute || !cfg!(target_os = "macos"),
                "compute stages on Metal"
            );
            assert_eq!(render_passes, 3, "reduce, across and down");
            // Where the adapter has no compute stages, both are passes.
            assert_eq!(stage_passes, if compute { 1 } else { 3 });
            assert!(
                stages == passes,
                "{scale}x, sigma {sigma}, fade {fade}: {} bytes differ",
                stages.iter().zip(&passes).filter(|(a, b)| a != b).count()
            );
        }
    }
    zgx_gpu::debug_blur_passes(false);
}
