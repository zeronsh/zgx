//! A hard spread shadow on a superellipse corner is the box offset by the
//! spread, a ring of even width: the same at the diagonal as at the sides.
use std::sync::Arc;
use zgx::decoration::Decoration;
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

#[test]
fn a_wide_spread_ring_keeps_its_width_round_a_superellipse_corner() {
    zgx::style::set_corner_shape(2.);
    let mut scene = Scene::new(140., 140.);
    scene.append(
        scene.root(),
        NodeKind::Quad(QuadStyle {
            fill: Color(255, 255, 255, 255),
            radius: 30.,
            decoration: Some(Arc::new(Decoration {
                shadows: Some(Arc::from([BoxShadow {
                    color: Color(255, 0, 0, 255),
                    offset: Transform { x: 0., y: 0. },
                    blur_radius: 0.,
                    spread: 8.,
                }])),
                ..Default::default()
            })),
            ..Default::default()
        }),
        Style {
            width: Some(100.),
            height: Some(100.),
            margin: Insets {
                left: 20.,
                top: 20.,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let mut gpu = GpuRenderer::new(140, 140).unwrap();
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    let pixels = gpu.readback().unwrap();
    let red = |x: f32, y: f32| {
        let (x, y) = (x.floor() as usize, y.floor() as usize);
        let i = (y * 140 + x) * 4;
        f32::from(pixels[i]) / 255. * f32::from(pixels[i + 3]) / 255.
            * (1. - f32::from(pixels[i + 1]) / 255.)
    };
    // Points on the corner's surface (centre (50, 50), radius 30, exponent 4)
    // and along its normal: 7.5 points out is in the ring, 8.5 out is clear.
    let n = 4.0_f32;
    let mut worst_in = 1.0_f32;
    let mut worst_out = 0.0_f32;
    for step in 1..9 {
        let t = std::f32::consts::FRAC_PI_2 * step as f32 / 9.;
        let (u, v) = (t.cos().powf(2. / n), t.sin().powf(2. / n));
        let g = (u.powf(n - 1.), v.powf(n - 1.));
        let len = (g.0 * g.0 + g.1 * g.1).sqrt();
        let normal = (g.0 / len, g.1 / len);
        let surface = (50. - 30. * u, 50. - 30. * v);
        let at = |d: f32| (surface.0 - normal.0 * d, surface.1 - normal.1 * d);
        let inside = at(7.5);
        let outside = at(8.5);
        worst_in = worst_in.min(red(inside.0, inside.1));
        worst_out = worst_out.max(red(outside.0, outside.1));
    }
    assert!(worst_in > 0.4, "the ring is there all round: {worst_in}");
    assert!(worst_out < 0.4, "and no wider: {worst_out}");
}
