//! A rounded corner's edge is shaded as Core Animation shades a CALayer's:
//! within one level of a capture of one (`fixtures/appkit-corner-*.png`).
//! Native feel is the bar on the desktop, so zgx's corner keeps CA's
//! antialiasing (`saturate((1 - |tc|) / fwidth(|tc|) + 0.5)`, its band the
//! slopes' sum, √2 wider at the diagonal than Chrome's, where Skia's is a
//! pixel at every angle). Measured Nov 16 against Chrome and a real window:
//! zgx and the CALayer differed by 0.02 levels over the corner (max 1), Chrome
//! by 0.12 (max 4). A change toward Chrome's edge fails here.
//!
//! The captures: a borderless window at 2x whose layer-backed content view
//! (the page colour) holds a CALayer 80x80 at (10, 10) points, `cornerRadius`
//! 15, `cornerCurve` `.circular`, `borderWidth` 1, `masksToBounds`, read back
//! with ScreenCaptureKit at 200x200. Dark: page #141414, fill #1f1f1f, border
//! rgb(247, 234, 212) at 0.059. Light: page #f5f5f5, fill white, border black
//! at 0.1. The same scene is drawn here.
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

/// Within a level of the CALayer's on macOS, where both are drawn by Metal
/// and Core Animation; elsewhere (llvmpipe, WARP) within three, their
/// antialiasing rasterising a little otherwise, so that an edge drawn
/// another way (Chrome's, 3 to 8 levels off) still fails.
const TOLERANCE: u8 = if cfg!(target_os = "macos") { 1 } else { 3 };
const DARK: &[u8] = include_bytes!("fixtures/appkit-corner-dark.png");
const LIGHT: &[u8] = include_bytes!("fixtures/appkit-corner-light.png");

fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}

/// The reference scene, drawn at 2x: RGBA, 200x200.
fn draw(direct: bool, (page, fill, edge): (Color, Color, Color)) -> Vec<u8> {
    let mut scene = Scene::new(100., 100.);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    scene.append(
        root,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad: QuadStyle {
                fill: page,
                ..Default::default()
            },
        },
        fixed(100., 100.),
    );
    let id = scene.append(
        root,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad: QuadStyle {
                fill,
                radius: 15.,
                border_color: edge,
                border_width: 1.,
                ..Default::default()
            },
        },
        fixed(80., 80.),
    );
    scene.set_transform(id, Transform { x: 10., y: 10. });
    scene.flush();
    let mut gpu = GpuRenderer::new(200, 200).unwrap();
    gpu.set_scale_factor(2.);
    gpu.set_direct_metal(direct);
    gpu.render(&scene, &[Rect::new(0., 0., 100., 100.)])
        .unwrap();
    gpu.readback().unwrap()
}

#[test]
fn a_rounded_corners_edge_is_shaded_as_a_calayers() {
    let themes = [
        (
            "dark",
            DARK,
            (
                Color(0x14, 0x14, 0x14, 255),
                Color(0x1f, 0x1f, 0x1f, 255),
                Color(247, 234, 212, 15),
            ),
        ),
        (
            "light",
            LIGHT,
            (
                Color(0xf5, 0xf5, 0xf5, 255),
                Color(255, 255, 255, 255),
                Color(0, 0, 0, 26),
            ),
        ),
    ];
    for (theme, capture, colours) in themes {
        let appkit = image::load_from_memory(capture).unwrap().to_rgba8();
        assert_eq!(
            appkit.dimensions(),
            (200, 200),
            "{theme}: the capture's size"
        );
        for direct in [true, false] {
            let ours = draw(direct, colours);
            // Every corner (the four quarter-circles and the edges beside
            // them), each channel within a level of the CALayer's.
            let mut worst = (0, 0, 0);
            for (x0, y0) in [(16, 16), (148, 16), (16, 148), (148, 148)] {
                for y in y0..y0 + 36 {
                    for x in x0..x0 + 36 {
                        let at = (y * 200 + x) as usize * 4;
                        let theirs = appkit.get_pixel(x, y).0;
                        for channel in 0..3 {
                            let off = ours[at + channel].abs_diff(theirs[channel]);
                            if off > worst.0 {
                                worst = (off, x, y);
                            }
                        }
                    }
                }
            }
            let (off, x, y) = worst;
            assert!(
                off <= TOLERANCE,
                "{theme}, direct {direct}: ({x}, {y}) is {off} levels off the CALayer's"
            );
        }
    }
}
