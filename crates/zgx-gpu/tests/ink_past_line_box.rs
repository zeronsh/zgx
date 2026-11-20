//! Ink may pass its line box: a text whose box is shorter than its glyphs
//! (a 20 px font in a 14 px box) still draws the part below the box.
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

#[test]
fn text_ink_below_its_box_is_drawn() {
    let mut scene = Scene::new(80., 60.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    scene.append(
        scene.root(),
        NodeKind::Text {
            text: "HH".into(),
            font_size: 20.,
            color: Color(255, 255, 255, 255),
        },
        Style {
            width: Some(60.),
            height: Some(14.),
            ..Default::default()
        },
    );
    let mut gpu = GpuRenderer::new(80, 60).unwrap();
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    let pixels = gpu.readback().unwrap();
    let lit = |y: usize| {
        (0..80).any(|x| pixels[(y * 80 + x) * 4 + 3] > 0 && pixels[(y * 80 + x) * 4] > 100)
    };
    assert!(lit(8), "ink inside the box");
    assert!(lit(15), "ink below the 14 px box is not clipped");
}
