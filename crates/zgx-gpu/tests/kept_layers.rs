//! A window holding layers keeps its frame too: when only what's inside a
//! layer moves (pictures on springs under a layer shader, a page still over
//! them), the layer's texture is painted again and its one composite made
//! again, and the page's items are drawn as they were kept. Each such frame
//! is pixel for pixel what drawing the scene afresh draws.
use zgx::decoration::{FillShader, ShaderUniforms};
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

const SIZE: (f32, f32) = (240., 160.);
const GRAIN: &str = "
fn layer(input: LayerInput) -> vec4<f32> {
    let q = input.p / input.size - 0.5;
    return vec4(input.color.rgb * (1.0 - dot(q, q)) + input.params[0].x * 0.1, input.color.a);
}";

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
/// A wallpaper of `layered` shapes under a layer shader, and a page of rows
/// over it. The shapes, and the wallpaper.
fn scene() -> (Scene, Vec<NodeId>, NodeId) {
    let mut scene = Scene::new(SIZE.0, SIZE.1);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    let wallpaper = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        fixed(SIZE.0, SIZE.1),
    );
    let mut shapes = Vec::new();
    for (index, color) in [Color(200, 80, 60, 255), Color(60, 140, 220, 255)]
        .into_iter()
        .enumerate()
    {
        let shape = scene.append(wallpaper, NodeKind::Rect(color), fixed(120., 90.));
        // A layer of its own, as a picture with an opacity is.
        scene.set_effects(
            shape,
            Effects {
                opacity: 0.9,
                ..Default::default()
            },
        );
        scene.set_transform(
            shape,
            Transform {
                x: 20. + 60. * index as f32,
                y: 20.,
            },
        );
        shapes.push(shape);
    }
    scene.set_layer_shader(wallpaper, Some(FillShader::new(GRAIN, None)));
    let page = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        fixed(SIZE.0, SIZE.1),
    );
    for row in 0..16 {
        let card = scene.append(
            page,
            NodeKind::Quad(QuadStyle {
                fill: Color(20, 20, 26, 200),
                radius: 3.,
                ..Default::default()
            }),
            fixed(180., 7.),
        );
        scene.set_transform(
            card,
            Transform {
                x: 30.,
                y: 8. + row as f32 * 9.,
            },
        );
    }
    (scene, shapes, wallpaper)
}
fn move_shapes(scene: &mut Scene, shapes: &[NodeId], wallpaper: NodeId, frame: usize) {
    for (index, shape) in shapes.iter().enumerate() {
        let t = frame as f32 * 0.3 + index as f32;
        scene.set_transform(
            *shape,
            Transform {
                x: 20. + 60. * index as f32 + 12. * t.sin(),
                y: 20. + 9. * t.cos(),
            },
        );
    }
    scene.set_layer_uniforms(
        wallpaper,
        ShaderUniforms {
            time: 0.,
            params: [[frame as f32 % 3., 0., 0., 0.], [0.; 4], [0.; 4], [0.; 4]],
        },
    );
}
fn afresh(scene: &Scene) -> Vec<u8> {
    let mut gpu = GpuRenderer::new(SIZE.0 as u32, SIZE.1 as u32).unwrap();
    gpu.render(scene, &[Rect::new(0., 0., SIZE.0, SIZE.1)])
        .unwrap();
    gpu.readback().unwrap()
}

#[test]
fn layers_moving_under_a_still_page_patch_the_kept_frame_and_draw_as_afresh() {
    let _gpu = gpu_fixture();
    let (mut scene, shapes, wallpaper) = scene();
    let mut gpu = GpuRenderer::new(SIZE.0 as u32, SIZE.1 as u32).unwrap();
    let report = scene.flush();
    gpu.render(&scene, &report.damage).unwrap();
    let mut patched = 0;
    for frame in 1..12 {
        move_shapes(&mut scene, &shapes, wallpaper, frame);
        let report = scene.flush();
        let stats = gpu.render(&scene, &report.damage).unwrap();
        patched += stats.patched_items;
        // The wallpaper's texture painted again; its pictures' kept.
        assert_eq!(stats.layer_repaints, 1, "frame {frame}");
        if frame % 4 == 0 {
            assert_eq!(gpu.readback().unwrap(), afresh(&scene), "frame {frame}");
        }
    }
    assert!(
        patched >= 10,
        "frames patched the kept frame: {patched} items"
    );
}

#[test]
fn a_blend_reading_whats_behind_a_layer_keeps_no_frame() {
    let _gpu = gpu_fixture();
    let (mut scene, shapes, wallpaper) = scene();
    scene.set_blend(shapes[1], BlendMode::Multiply);
    let mut gpu = GpuRenderer::new(SIZE.0 as u32, SIZE.1 as u32).unwrap();
    let report = scene.flush();
    gpu.render(&scene, &report.damage).unwrap();
    // The blend is inside the wallpaper, drawn into its texture: the window's
    // frame is still kept.
    move_shapes(&mut scene, &shapes, wallpaper, 1);
    let report = scene.flush();
    gpu.render(&scene, &report.damage).unwrap();
    assert_eq!(gpu.readback().unwrap(), afresh(&scene));
    // A blend in the window's own pass, reading what's behind it: no frame
    // kept, drawn as usual.
    scene.set_blend(wallpaper, BlendMode::Screen);
    for frame in 2..5 {
        move_shapes(&mut scene, &shapes, wallpaper, frame);
        let report = scene.flush();
        let stats = gpu.render(&scene, &report.damage).unwrap();
        assert_eq!(stats.patched_items, 0, "frame {frame}");
    }
    assert_eq!(gpu.readback().unwrap(), afresh(&scene));
}
