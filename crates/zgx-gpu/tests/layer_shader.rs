//! Layer shaders (`Scene::set_layer_shader`): a node's subtree, cached as a
//! layer, composites through a WGSL `filter` that sees each of its pixels
//! at the node's own position, can read the layer elsewhere, and takes
//! per-node uniforms that composite again without repainting. Under a local
//! transform it still works in the node's space. On the wgpu path, which
//! frames with Metal directly fall back to.
use zgx::decoration::{FillShader, ShaderUniforms};
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
const SIZE: u32 = 200;
fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * SIZE + x) * 4) as usize;
    pixels[at..at + 4].try_into().unwrap()
}
fn near(actual: [u8; 4], expected: [u8; 4]) -> bool {
    actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 2)
}
/// Its left half tinted by `params[0]`; its right half shows the layer's
/// left half (`zgx_sample`), unfiltered.
const TINT_AND_COPY: &str = "
fn layer(input: LayerInput) -> vec4<f32> {
    if input.p.x < input.size.x * 0.5 {
        return vec4(input.color.rgb * input.params[0].rgb, input.color.a);
    }
    return zgx_sample(input.p - vec2(input.size.x * 0.5, 0.0));
}";
const RED: Color = Color(255, 0, 0, 255);
const GREEN: Color = Color(0, 255, 0, 255);
fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
/// A 100 × 100 container at (50, 50): its left half red, its right green.
fn card(scene: &mut Scene) -> NodeId {
    let root = scene.root();
    let id = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        fixed(100., 100.),
    );
    scene.set_transform(id, Transform { x: 50., y: 50. });
    scene.append(id, NodeKind::Rect(RED), fixed(50., 100.));
    let right = scene.append(id, NodeKind::Rect(GREEN), fixed(50., 100.));
    scene.set_transform(right, Transform { x: 50., y: 0. });
    id
}
fn tint(r: f32) -> ShaderUniforms {
    ShaderUniforms {
        time: 0.,
        params: [[r, 1., 1., 1.], [0.; 4], [0.; 4], [0.; 4]],
    }
}
fn scene() -> Scene {
    let mut scene = Scene::new(SIZE as f32, SIZE as f32);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    scene
}
/// Render the flushed frame, then check it against a full repaint.
fn render(gpu: &mut GpuRenderer, scene: &mut Scene) -> (FrameReport, Vec<u8>) {
    let frame = scene.flush();
    gpu.render(scene, &frame.damage).unwrap();
    let pixels = gpu.readback().unwrap();
    gpu.render(scene, &[Rect::new(0., 0., SIZE as f32, SIZE as f32)])
        .unwrap();
    let full = gpu.readback().unwrap();
    let differs =
        (0..SIZE * SIZE).find(|i| pixels[*i as usize * 4..][..4] != full[*i as usize * 4..][..4]);
    assert!(
        differs.is_none(),
        "incremental frame differs from full at {:?}: {:?} vs {:?}",
        differs.map(|i| (i % SIZE, i / SIZE)),
        differs.map(|i| pixel(&pixels, i % SIZE, i / SIZE)),
        differs.map(|i| pixel(&full, i % SIZE, i / SIZE)),
    );
    (frame, pixels)
}
fn renderers() -> impl Iterator<Item = (bool, GpuRenderer)> {
    [true, false].into_iter().map(|direct| {
        let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
        gpu.set_direct_metal(direct);
        (direct, gpu)
    })
}

#[test]
fn a_layer_composites_through_its_shader_in_its_own_space() {
    let _gpu_fixture = gpu_fixture();
    let shader = FillShader::new(TINT_AND_COPY, None);
    for (direct, mut gpu) in renderers() {
        let mut scene = scene();
        let id = card(&mut scene);
        scene.set_layer_shader(id, Some(shader.clone()));
        scene.set_layer_uniforms(id, tint(0.5));
        assert!(scene.is_layer(id));
        let (_, p) = render(&mut gpu, &mut scene);
        // Left: red at half; right: the layer's left half, red as painted.
        assert!(
            near(pixel(&p, 70, 100), [128, 0, 0, 255]),
            "direct {direct}: {:?}",
            pixel(&p, 70, 100)
        );
        assert!(
            near(pixel(&p, 130, 100), [255, 0, 0, 255]),
            "{:?}",
            pixel(&p, 130, 100)
        );
        assert_eq!(pixel(&p, 20, 20), [0; 4]);

        // New uniforms composite again: no layout, the layer kept.
        scene.set_layer_uniforms(id, tint(1.));
        let (frame, p) = render(&mut gpu, &mut scene);
        assert_eq!(frame.layout_nodes, 0);
        assert!(
            near(pixel(&p, 70, 100), [255, 0, 0, 255]),
            "{:?}",
            pixel(&p, 70, 100)
        );

        // Under a local transform, `p` is still the node's own: halved about
        // its centre, its left quarter of the window is tinted.
        scene.set_layer_uniforms(id, tint(0.));
        scene.set_local_transform(
            id,
            LocalTransform {
                scale: (0.5, 0.5),
                ..LocalTransform::IDENTITY
            },
        );
        let (_, p) = render(&mut gpu, &mut scene);
        assert!(
            near(pixel(&p, 85, 100), [0, 0, 0, 255]),
            "{:?}",
            pixel(&p, 85, 100)
        );
        assert!(
            near(pixel(&p, 115, 100), [255, 0, 0, 255]),
            "{:?}",
            pixel(&p, 115, 100)
        );
        assert_eq!(pixel(&p, 60, 100), [0; 4]);

        // Without its shader it composites as painted.
        scene.set_layer_shader(id, None);
        scene.set_local_transform(id, LocalTransform::IDENTITY);
        let (_, p) = render(&mut gpu, &mut scene);
        assert_eq!(pixel(&p, 70, 100), [255, 0, 0, 255]);
        assert_eq!(pixel(&p, 130, 100), [0, 255, 0, 255]);
    }
}

#[test]
fn a_layer_shader_that_does_not_compile_composites_the_layer_as_painted() {
    let _gpu_fixture = gpu_fixture();
    assert!(
        zgx_gpu::check_layer_shader(
            "fn layer(input: LayerInput) -> vec4<f32> { return input.nope; }"
        )
        .is_err()
    );
    assert_eq!(zgx_gpu::check_layer_shader(TINT_AND_COPY), Ok(()));
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let id = card(&mut scene);
    scene.set_layer_shader(id, Some(FillShader::new("fn layer(", None)));
    let (_, p) = render(&mut gpu, &mut scene);
    assert_eq!(pixel(&p, 70, 100), [255, 0, 0, 255]);
    assert_eq!(pixel(&p, 130, 100), [0, 255, 0, 255]);
}

/// `hue_rotate`, `sepia` and `invert`, against the numbers CSS's matrices give
/// for these colours, applied alone and one after another.
#[test]
fn color_filters_give_the_numbers_css_does() {
    use zgx::compose::prelude::*;
    use zgx::widgets::Ui;
    let _gpu = gpu_fixture();
    type Case = (&'static str, fn(View) -> View, Color, [u8; 3]);
    let cases: [Case; 9] = [
        ("invert", |v| v.invert(1.), RED, [0, 255, 255]),
        ("half invert", |v| v.invert(0.5), RED, [128, 128, 128]),
        ("no invert", |v| v.invert(0.), RED, [255, 0, 0]),
        ("sepia", |v| v.sepia(1.), RED, [100, 89, 69]),
        ("no hue", |v| v.hue_rotate(0.), RED, [255, 0, 0]),
        ("hue 90", |v| v.hue_rotate(90.), RED, [0, 91, 0]),
        ("hue 180", |v| v.hue_rotate(180.), RED, [0, 109, 109]),
        ("hue 360", |v| v.hue_rotate(360.), GREEN, [0, 255, 0]),
        // In order: inverted to cyan, then back.
        ("twice", |v| v.invert(1.).invert(1.), RED, [255, 0, 0]),
    ];
    for (name, filter, color, expected) in cases {
        let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
        let mut ui = Ui::new(SIZE as f32, SIZE as f32);
        ui.mount(filter(div().size(100., 100.).bg(color)));
        ui.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        let pixels = gpu.readback().unwrap();
        let got = pixel(&pixels, 50, 50);
        let wanted = [expected[0], expected[1], expected[2], 255];
        assert!(near(got, wanted), "{name}: {got:?}, not {wanted:?}");
    }
}

/// Its pixels what lies behind it, moved along by `params[0].x` points (a
/// shift of the backdrop), or as it is where that's 0.
const SHIFTED_BACKDROP: &str = "
fn layer(input: LayerInput) -> vec4<f32> {
    let behind = zgx_backdrop(input.p + vec2(input.params[0].x, 0.0));
    return vec4(mix(behind.rgb, input.color.rgb, input.color.a), max(behind.a, input.color.a));
}";
fn shift(by: f32) -> ShaderUniforms {
    ShaderUniforms {
        time: 0.,
        params: [[by, 0., 0., 0.], [0.; 4], [0.; 4], [0.; 4]],
    }
}
/// A left half red and right half green on `parent`, the seam at x = 100.
fn behind(scene: &mut Scene, parent: NodeId) -> (NodeId, NodeId) {
    let left = scene.append(parent, NodeKind::Rect(RED), fixed(100., SIZE as f32));
    let right = scene.append(parent, NodeKind::Rect(GREEN), fixed(100., SIZE as f32));
    scene.set_transform(right, Transform { x: 100., y: 0. });
    (left, right)
}
/// An empty box over them, x 60 to 140, reading the backdrop.
fn reader(scene: &mut Scene, parent: NodeId) -> NodeId {
    let id = scene.append(
        parent,
        NodeKind::Container(Layout::Overlay),
        fixed(80., 80.),
    );
    scene.set_transform(id, Transform { x: 60., y: 60. });
    scene.set_layer_shader(id, Some(FillShader::new(SHIFTED_BACKDROP, None)));
    id
}

#[test]
fn a_layer_shader_reads_what_lies_behind_it() {
    let _gpu_fixture = gpu_fixture();
    assert_eq!(zgx_gpu::check_layer_shader(SHIFTED_BACKDROP), Ok(()));
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let root = scene.root();
    behind(&mut scene, root);
    let id = reader(&mut scene, root);
    scene.set_layer_extent(id, 0., 30.);
    let (_, p) = render(&mut gpu, &mut scene);
    // Unmoved, it shows what's behind it exactly.
    assert_eq!(pixel(&p, 90, 100), [255, 0, 0, 255]);
    assert_eq!(pixel(&p, 110, 100), [0, 255, 0, 255]);
    // Moved along by 20 points the seam shows at 80.
    scene.set_layer_uniforms(id, shift(20.));
    let (frame, p) = render(&mut gpu, &mut scene);
    assert_eq!(frame.layout_nodes, 0);
    assert_eq!(pixel(&p, 70, 100), [255, 0, 0, 255]);
    assert_eq!(pixel(&p, 90, 100), [0, 255, 0, 255]);
    // Past the box as painted.
    assert_eq!(pixel(&p, 50, 100), [255, 0, 0, 255]);
    assert_eq!(pixel(&p, 150, 100), [0, 255, 0, 255]);
    // At the box's right edge it reads 20 points past it, within its reach.
    assert_eq!(pixel(&p, 135, 100), [0, 255, 0, 255]);
    // A read past the reach holds at its edge: the seam, 30 past the box, isn't crossed.
    scene.set_layer_uniforms(id, shift(-200.));
    let (_, p) = render(&mut gpu, &mut scene);
    assert_eq!(pixel(&p, 130, 100), [255, 0, 0, 255]);
}

#[test]
#[cfg(target_os = "macos")]
fn a_backdrop_is_never_compressed_on_apple_gpus() {
    // Draws sampling a compressed backdrop after a blit into part of it now
    // and then read 16-pixel blocks of it from before the copy: a reader
    // moving into what it had not read before showed them black, or bright
    // where dividing by their alpha blew up (zgx-video's
    // `repainting_what_changed_draws_what_painting_it_all_draws`).
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let root = scene.root();
    behind(&mut scene, root);
    let id = reader(&mut scene, root);
    scene.set_layer_extent(id, 0., 30.);
    scene.set_layer_uniforms(id, shift(20.));
    let (_, p) = render(&mut gpu, &mut scene);
    // What it reads is the copy's, from a backdrop that started cleared.
    assert_eq!(pixel(&p, 70, 100), [255, 0, 0, 255]);
    assert_eq!(pixel(&p, 90, 100), [0, 255, 0, 255]);
    assert_eq!(pixel(&p, 135, 100), [0, 255, 0, 255]);
    let held = gpu.debug_backdrops_compress();
    assert!(!held.is_empty(), "a backdrop is held");
    assert!(held.iter().all(|compress| !compress), "{held:?}");
}

#[test]
fn a_backdrop_reader_repaints_when_what_is_behind_it_changes() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let root = scene.root();
    behind(&mut scene, root);
    // A square behind it, far to its left.
    let square = scene.append(root, NodeKind::Rect(RED), fixed(10., 10.));
    scene.set_transform(square, Transform { x: 25., y: 100. });
    let id = reader(&mut scene, root);
    scene.set_layer_extent(id, 0., 60.);
    scene.set_layer_uniforms(id, shift(-40.));
    render(&mut gpu, &mut scene);
    // It changes, out of the reader's box but in what it reads, and shows
    // moved along by 40 in the reader. (`render` asserts this incremental
    // frame equals a full one.)
    scene.set_kind(square, NodeKind::Rect(Color(0, 0, 255, 255)));
    let (_, p) = render(&mut gpu, &mut scene);
    assert_eq!(pixel(&p, 30, 105), [0, 0, 255, 255]);
    assert_eq!(pixel(&p, 70, 105), [0, 0, 255, 255]);
    assert_eq!(pixel(&p, 80, 105), [255, 0, 0, 255]);
    // Nothing changed: nothing is drawn.
    let frame = scene.flush();
    let stats = gpu.render(&scene, &frame.damage).unwrap();
    assert_eq!(stats.draw_calls, 0);
    assert_eq!(stats.render_passes, 0);
}

#[test]
fn a_backdrop_read_in_a_layer_reads_that_layer() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let root = scene.root();
    behind(&mut scene, root);
    // An isolated group, its own content blue on its left half only, over
    // the window's red and green.
    let group = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Overlay),
        fixed(SIZE as f32, SIZE as f32),
    );
    scene.set_isolated(group, true);
    let blue = scene.append(
        group,
        NodeKind::Rect(Color(0, 0, 255, 255)),
        fixed(100., 100.),
    );
    let id = reader(&mut scene, group);
    let _ = blue;
    let (_, p) = render(&mut gpu, &mut scene);
    // Reads the group: blue where the group has it, clear below it (showing
    // the window's red and green through), never the window's pixels in it.
    assert_eq!(pixel(&p, 90, 90), [0, 0, 255, 255]);
    assert_eq!(pixel(&p, 90, 120), [255, 0, 0, 255]);
    let _ = id;
}

#[test]
fn a_backdrop_reader_under_a_transform_and_opacity_reads_in_its_own_space() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let root = scene.root();
    behind(&mut scene, root);
    let id = reader(&mut scene, root);
    scene.set_layer_extent(id, 0., 40.);
    // Halved about its centre (box 80..120 on screen), shifted 20 of its own
    // points, 10 on screen: the seam shows at 90 on screen.
    scene.set_local_transform(
        id,
        LocalTransform {
            scale: (0.5, 0.5),
            ..LocalTransform::IDENTITY
        },
    );
    scene.set_layer_uniforms(id, shift(20.));
    let (_, p) = render(&mut gpu, &mut scene);
    assert_eq!(pixel(&p, 85, 100), [255, 0, 0, 255]);
    assert_eq!(pixel(&p, 95, 100), [0, 255, 0, 255]);
    // At half opacity: half what it shows over what's under it, red over green.
    let mut effects = scene.effects(id);
    effects.opacity = 0.5;
    scene.set_effects(id, effects);
    let (_, p) = render(&mut gpu, &mut scene);
    // At 95, green read at half over the red under it; at 102, green over green.
    assert!(
        near(pixel(&p, 95, 100), [128, 128, 0, 255]),
        "{:?}",
        pixel(&p, 95, 100)
    );
    assert!(
        near(pixel(&p, 102, 100), [0, 255, 0, 255]),
        "{:?}",
        pixel(&p, 102, 100)
    );
    // Read from the other side: red at half over the green under it.
    scene.set_layer_uniforms(id, shift(-40.));
    let (_, p) = render(&mut gpu, &mut scene);
    assert!(
        near(pixel(&p, 110, 100), [128, 128, 0, 255]),
        "{:?}",
        pixel(&p, 110, 100)
    );
}

/// Clear in the box; past it, by distance from the box, solid blue for 10
/// points and then nothing.
const RING: &str = "
fn layer(input: LayerInput) -> vec4<f32> {
    let outside = max(max(-input.p, input.p - input.size), vec2(0.0));
    let d = length(outside);
    if d <= 0.0 { return input.color; }
    return vec4(0.0, 0.0, 1.0, select(0.0, 1.0, d < 10.0));
}";

#[test]
fn a_layer_shader_paints_past_its_box_by_its_bleed() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let id = card(&mut scene);
    scene.set_layer_shader(id, Some(FillShader::new(RING, None)));
    // No bleed: it stops at the box.
    let (_, p) = render(&mut gpu, &mut scene);
    assert_eq!(pixel(&p, 45, 100), [0; 4]);
    // A bleed: it paints past it, and the content stays as painted.
    scene.set_layer_extent(id, 20., 0.);
    let (_, p) = render(&mut gpu, &mut scene);
    assert_eq!(pixel(&p, 45, 100), [0, 0, 255, 255]);
    assert_eq!(pixel(&p, 155, 100), [0, 0, 255, 255]);
    // Painted where it was, to the pixel: red to its seam at 100, then green.
    assert_eq!(pixel(&p, 70, 100), [255, 0, 0, 255]);
    assert_eq!(pixel(&p, 98, 100), [255, 0, 0, 255]);
    assert_eq!(pixel(&p, 102, 100), [0, 255, 0, 255]);
    assert_eq!(pixel(&p, 148, 148), [0, 255, 0, 255]);
    // Past the ring, nothing.
    assert_eq!(pixel(&p, 35, 100), [0; 4]);
    // And the bleed gone again: the frame's damage covers where it was
    // (`render` compares it with a full repaint).
    scene.set_layer_extent(id, 0., 0.);
    let (_, p) = render(&mut gpu, &mut scene);
    assert_eq!(pixel(&p, 45, 100), [0; 4]);
    // Moved with its bleed: where it was and where it lands.
    scene.set_layer_extent(id, 20., 0.);
    render(&mut gpu, &mut scene);
    scene.set_transform(id, Transform { x: 60., y: 50. });
    let (_, p) = render(&mut gpu, &mut scene);
    assert_eq!(pixel(&p, 55, 100), [0, 0, 255, 255]);
    assert_eq!(pixel(&p, 45, 100), [0; 4]);
}

#[test]
fn a_bleed_is_cut_by_the_clip_around_the_node_not_its_own() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    // A clip around it: x 40 to 200.
    let clip = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Overlay),
        Style {
            clip: true,
            ..fixed(160., SIZE as f32)
        },
    );
    scene.set_transform(clip, Transform { x: 48., y: 0. });
    let id = scene.append(
        clip,
        NodeKind::Container(Layout::Overlay),
        Style {
            clip: true,
            ..fixed(100., 100.)
        },
    );
    scene.set_transform(id, Transform { x: 2., y: 50. });
    scene.set_layer_shader(id, Some(FillShader::new(RING, None)));
    scene.set_layer_extent(id, 20., 0.);
    let (_, p) = render(&mut gpu, &mut scene);
    // Its own clip doesn't cut it; the one around it does.
    assert_eq!(pixel(&p, 49, 100), [0, 0, 255, 255]);
    assert_eq!(pixel(&p, 47, 100), [0; 4]);
    assert_eq!(pixel(&p, 155, 100), [0, 0, 255, 255]);
}

#[test]
fn a_layer_shader_sees_its_boxs_corners() {
    let _gpu_fixture = gpu_fixture();
    let shader = FillShader::new(
        "fn layer(input: LayerInput) -> vec4<f32> {
            return vec4(input.radii.x / 20.0, input.radii.y / 20.0, 0.0, 1.0);
        }",
        None,
    );
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let id = scene.append(
        scene.root(),
        NodeKind::Quad(QuadStyle {
            fill: Color(255, 255, 255, 255),
            radius: 10.,
            ..Default::default()
        }),
        fixed(100., 100.),
    );
    scene.set_layer_shader(id, Some(shader));
    let (_, p) = render(&mut gpu, &mut scene);
    assert!(
        near(pixel(&p, 50, 50), [128, 128, 0, 255]),
        "{:?}",
        pixel(&p, 50, 50)
    );
}

#[test]
fn backdrop_readers_apart_share_one_copy_and_pass() {
    let _gpu_fixture = gpu_fixture();
    let shader = FillShader::new(SHIFTED_BACKDROP, None);
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let root = scene.root();
    behind(&mut scene, root);
    let reader = |scene: &mut Scene, x: f32, y: f32| {
        let id = scene.append(root, NodeKind::Container(Layout::Overlay), fixed(40., 40.));
        scene.set_transform(id, Transform { x, y });
        scene.set_layer_shader(id, Some(shader.clone()));
        scene.set_layer_extent(id, 0., 8.);
        scene.set_layer_uniforms(id, shift(5.));
        id
    };
    reader(&mut scene, 20., 20.);
    reader(&mut scene, 120., 20.);
    let frame = scene.flush();
    let stats = gpu.render(&scene, &frame.damage).unwrap();
    assert_eq!(stats.backdrop_copies, 1, "apart, one copy");
    // Over each other, the second reads what the first drew: copies of its own.
    let third = reader(&mut scene, 30., 30.);
    let frame = scene.flush();
    let stats = gpu.render(&scene, &frame.damage).unwrap();
    assert_eq!(stats.backdrop_copies, 2, "the third reads the first");
    // Pixels as a full frame's (`render`), the first's shift showing at its seam.
    scene.set_layer_uniforms(third, shift(0.));
    render(&mut gpu, &mut scene);
}

#[test]
fn a_backdrop_reader_reads_what_its_own_backdrop_blur_blurred() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let root = scene.root();
    behind(&mut scene, root);
    let id = reader(&mut scene, root);
    let mut effects = scene.effects(id);
    effects.blur_radius = 12.;
    scene.set_effects(id, effects);
    let (_, p) = render(&mut gpu, &mut scene);
    // At the seam, red and green mixed: the blur's, not its sharp edge.
    let seam = pixel(&p, 100, 100);
    assert!(seam[0] > 70 && seam[1] > 70, "{seam:?}");
    // Far from it, as they were.
    assert_eq!(pixel(&p, 40, 100), [255, 0, 0, 255]);
}

#[test]
fn a_backdrop_reader_in_a_rounded_clip_is_cut_by_its_corners() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let root = scene.root();
    behind(&mut scene, root);
    // A rounded clip at (50, 50), a reader filling it, turning what's behind
    // it blue-free: its pixel with green and red swapped.
    let clip = scene.append(
        root,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad: QuadStyle {
                radius: 30.,
                ..Default::default()
            },
        },
        Style {
            clip: true,
            ..fixed(100., 100.)
        },
    );
    scene.set_transform(clip, Transform { x: 50., y: 50. });
    scene.set_corners_clip(clip, true);
    let id = scene.append(
        clip,
        NodeKind::Container(Layout::Overlay),
        fixed(100., 100.),
    );
    scene.set_layer_shader(
        id,
        Some(FillShader::new(
            "fn layer(input: LayerInput) -> vec4<f32> {
                let behind = zgx_backdrop(input.p);
                return vec4(behind.g, behind.r, behind.b, behind.a);
            }",
            None,
        )),
    );
    let (_, p) = render(&mut gpu, &mut scene);
    // Inside the curve it's swapped; at the corner it's cut, as behind.
    assert_eq!(pixel(&p, 90, 100), [0, 255, 0, 255]);
    assert_eq!(pixel(&p, 110, 100), [255, 0, 0, 255]);
    assert_eq!(pixel(&p, 52, 52), [255, 0, 0, 255]);
    assert_eq!(pixel(&p, 148, 148), [0, 255, 0, 255]);
}

/// `backdrop_blur` in a rounded clip: cut along its curve, as what's painted is.
#[test]
fn a_backdrop_blur_is_cut_by_the_rounded_clip_around_it() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let root = scene.root();
    // Fine stripes, so what's blurred reads otherwise than what's not.
    for i in 0..20 {
        let stripe = scene.append(
            root,
            NodeKind::Rect(if i % 2 == 0 { RED } else { GREEN }),
            fixed(SIZE as f32, 10.),
        );
        scene.set_transform(
            stripe,
            Transform {
                x: 0.,
                y: i as f32 * 10.,
            },
        );
    }
    let clip = scene.append(
        root,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad: QuadStyle {
                radius: 40.,
                ..Default::default()
            },
        },
        Style {
            clip: true,
            ..fixed(100., 100.)
        },
    );
    scene.set_transform(clip, Transform { x: 50., y: 50. });
    scene.set_corners_clip(clip, true);
    let pane = scene.append(
        clip,
        NodeKind::Container(Layout::Overlay),
        fixed(100., 100.),
    );
    let mut effects = scene.effects(pane);
    effects.blur_radius = 12.;
    scene.set_effects(pane, effects);
    let (_, p) = render(&mut gpu, &mut scene);
    // In the middle, blurred: red and green mixed.
    let middle = pixel(&p, 100, 105);
    assert!(middle[0] > 60 && middle[1] > 60, "{middle:?}");
    // Past the curve, in the box's corner: as painted.
    assert_eq!(pixel(&p, 53, 53), [0, 255, 0, 255]);
    assert_eq!(pixel(&p, 146, 146), [255, 0, 0, 255]);
}

/// Draws on either side of a reader sharing an earlier one's copy don't join
/// over it: a call spanning its quad would paint it with theirs, the glyph
/// atlas across its box.
#[test]
fn draws_around_a_reader_sharing_a_copy_do_not_draw_over_it() {
    let _gpu_fixture = gpu_fixture();
    let shader = FillShader::new(SHIFTED_BACKDROP, None);
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let fonts = gpu.text_system();
    let mut scene = scene();
    scene.set_text_measurer(move |text: &str, size: f32, width: Option<f32>| {
        zgx_gpu::measure_text(&mut fonts.borrow_mut(), text, size, width)
    });
    let root = scene.root();
    scene.append(root, NodeKind::Rect(RED), fixed(SIZE as f32, SIZE as f32));
    let reader = |scene: &mut Scene, y: f32| {
        let id = scene.append(root, NodeKind::Container(Layout::Overlay), fixed(60., 40.));
        scene.set_transform(id, Transform { x: 130., y });
        scene.set_layer_shader(id, Some(shader.clone()));
    };
    let block = |scene: &mut Scene, y: f32| {
        let quad = QuadStyle {
            fill: GREEN,
            radius: 3.,
            ..Default::default()
        };
        let id = scene.append(root, NodeKind::Quad(quad), fixed(20., 10.));
        scene.set_transform(id, Transform { x: 4., y });
    };
    // Plenty of glyphs in the atlas, large, away from them.
    let glyphs = scene.append(
        root,
        NodeKind::Text {
            text: "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789".into(),
            color: Color(255, 255, 255, 255),
            font_size: 48.,
        },
        fixed(120., 60.),
    );
    scene.set_transform(glyphs, Transform { x: 4., y: 136. });
    reader(&mut scene, 10.);
    block(&mut scene, 70.);
    reader(&mut scene, 70.);
    block(&mut scene, 120.);
    let frame = scene.flush();
    let stats = gpu.render(&scene, &frame.damage).unwrap();
    assert_eq!(stats.backdrop_copies, 1, "they share a copy");
    let shown = gpu.readback().unwrap();
    for y in 70..110 {
        for x in 130..190 {
            assert_eq!(pixel(&shown, x, y), [255, 0, 0, 255], "at {x}, {y}");
        }
    }
}

#[test]
fn a_backdrop_reader_keeps_what_lies_under_its_shadow() {
    let _gpu_fixture = gpu_fixture();
    for (direct, mut gpu) in renderers() {
        let mut scene = scene();
        let root = scene.root();
        behind(&mut scene, root);
        let id = reader(&mut scene, root);
        // A shadow reaching far past the box and what it reads: its layer
        // composites over all of it.
        scene.set_kind(
            id,
            NodeKind::Quad(QuadStyle {
                shadow: Some(BoxShadow {
                    color: Color(0, 0, 0, 30),
                    offset: Transform { x: 0., y: 8. },
                    blur_radius: 20.,
                    spread: 0.,
                }),
                ..Default::default()
            }),
        );
        let (_, p) = render(&mut gpu, &mut scene);
        for (x, y, color) in [
            (4, 4, RED),
            (196, 4, GREEN),
            (4, 196, RED),
            (196, 196, GREEN),
        ] {
            let Color(r, g, b, a) = color;
            let got = pixel(&p, x, y);
            assert!(
                near(got, [r, g, b, a]),
                "direct {direct}, at {:?}: {got:?}",
                (x, y)
            );
        }
    }
}
