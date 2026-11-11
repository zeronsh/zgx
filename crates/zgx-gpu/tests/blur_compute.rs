//! macOS: a backdrop blur's stages as one compute pass, where Metal is
//! drawn directly (blur.wgsl's `cs_*`, metal_frame.rs), draw the same bytes
//! as the render passes wgpu draws them with: frosted panels at 1×, 2× and
//! 3×, narrow, wide and fading blurs.
#![cfg(target_os = "macos")]
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

/// A frame drawn directly with Metal or through wgpu, and its blur passes.
fn frame(direct: bool, scale: f32, sigma: f32, fade: f32) -> (Vec<u8>, usize) {
    let mut gpu = GpuRenderer::new((160. * scale) as u32, (100. * scale) as u32).unwrap();
    gpu.set_direct_metal(direct);
    gpu.set_scale_factor(scale);
    let mut s = scene(sigma, fade);
    let damage = s.flush().damage;
    let stats = gpu.render(&s, &damage).unwrap();
    (gpu.readback().unwrap(), stats.blur_passes)
}

#[test]
fn compute_stages_draw_the_render_passes_bytes() {
    for scale in [1., 2., 3.] {
        for (sigma, fade) in [(2., 0.), (9., 0.), (24., 0.), (10., 24.)] {
            let (stages, stage_passes) = frame(true, scale, sigma, fade);
            let (passes, render_passes) = frame(false, scale, sigma, fade);
            assert_eq!(
                (stage_passes, render_passes),
                (1, 3),
                "one compute pass, three render passes"
            );
            assert!(
                stages == passes,
                "{scale}x, sigma {sigma}, fade {fade}: {} bytes differ",
                stages.iter().zip(&passes).filter(|(a, b)| a != b).count()
            );
        }
    }
}
