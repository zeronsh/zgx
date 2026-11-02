//! A cached layer (a node with a layer shader, rasterized once and kept
//! while nothing in it changes) draws again whenever what it shows changes:
//! nodes coming into it or leaving it, a picture in it that finishes
//! decoding, a layer made anew where one was. Each frame is checked against
//! a renderer drawing the scene for the first time (Zeron's wallpaper under
//! an effect came up blank as its picture changed).
use std::sync::Arc;
use zgx::decoration::FillShader;
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
const SIZE: u32 = 120;
/// The layer as painted.
const AS_PAINTED: &str = "fn layer(input: LayerInput) -> vec4<f32> { return input.color; }";
const RED: Color = Color(255, 0, 0, 255);
const BLUE: Color = Color(0, 0, 255, 255);
fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
fn scene() -> Scene {
    let mut scene = Scene::new(SIZE as f32, SIZE as f32);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    scene
}
/// A 100-point layer at the window's corner, a red box in it.
fn layer(scene: &mut Scene, shader: &Arc<FillShader>) -> NodeId {
    let id = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Overlay),
        fixed(100., 100.),
    );
    scene.append(id, NodeKind::Rect(RED), fixed(100., 100.));
    scene.set_layer_shader(id, Some(shader.clone()));
    id
}
/// `gpu`'s frame of `scene`, which must be what a renderer new to it draws.
fn frame(gpu: &mut GpuRenderer, scene: &mut Scene, what: &str) -> Vec<u8> {
    let damage = scene.flush().damage;
    gpu.render(scene, &damage).unwrap();
    let pixels = gpu.readback().unwrap();
    let mut fresh = GpuRenderer::new(SIZE, SIZE).unwrap();
    fresh
        .render(scene, &[Rect::new(0., 0., SIZE as f32, SIZE as f32)])
        .unwrap();
    let wanted = fresh.readback().unwrap();
    let differ = pixels
        .chunks(4)
        .zip(wanted.chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        differ,
        0,
        "{what}: {differ} pixels differ from a fresh frame: {:?} vs {:?} in the middle",
        at(&pixels, 50, 50),
        at(&wanted, 50, 50)
    );
    pixels
}
fn at(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * SIZE + x) * 4) as usize;
    pixels[at..at + 4].try_into().unwrap()
}

#[test]
fn a_layer_draws_again_as_nodes_come_into_it_and_go() {
    let _gpu = gpu_fixture();
    let shader = FillShader::new(AS_PAINTED, None);
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let id = layer(&mut scene, &shader);
    frame(&mut gpu, &mut scene, "first");
    let blue = scene.append(id, NodeKind::Rect(BLUE), fixed(100., 100.));
    let p = frame(&mut gpu, &mut scene, "a node came in");
    assert_eq!(at(&p, 50, 50), [0, 0, 255, 255]);
    scene.remove(blue);
    let p = frame(&mut gpu, &mut scene, "it went");
    assert_eq!(at(&p, 50, 50), [255, 0, 0, 255]);
}

#[test]
fn a_layer_made_where_one_was_draws_its_own() {
    let _gpu = gpu_fixture();
    let shader = FillShader::new(AS_PAINTED, None);
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let id = layer(&mut scene, &shader);
    frame(&mut gpu, &mut scene, "first");
    scene.remove(id);
    frame(&mut gpu, &mut scene, "gone");
    // In the slots just freed, its revision new: blue now.
    let again = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Overlay),
        fixed(100., 100.),
    );
    scene.append(again, NodeKind::Rect(BLUE), fixed(100., 100.));
    scene.set_layer_shader(again, Some(shader.clone()));
    let p = frame(&mut gpu, &mut scene, "made again");
    assert_eq!(at(&p, 50, 50), [0, 0, 255, 255]);
}

#[test]
fn a_layer_draws_again_as_a_picture_in_it_decodes() {
    let _gpu = gpu_fixture();
    let shader = FillShader::new(AS_PAINTED, None);
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(400, 400, image::Rgba([0, 0, 255, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let picture = zgx_gpu::assets::decode_image_kept(Arc::from(png)).unwrap();
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = scene();
    let id = layer(&mut scene, &shader);
    frame(&mut gpu, &mut scene, "first");
    scene.append(id, NodeKind::Image(picture), fixed(100., 100.));
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    // As the host does once told its pixels are decoded: it draws again,
    // until a frame no longer goes without them.
    let mut rounds = 0;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(20));
        if !gpu.images_decoded() {
            break;
        }
        gpu.render(&scene, &[]).unwrap();
        rounds += 1;
        assert!(rounds < 50, "drawn without its picture each time");
    }
    eprintln!("rounds {rounds}");
    let p = frame(&mut gpu, &mut scene, "its picture decoded");
    assert_eq!(at(&p, 50, 50), [0, 0, 255, 255]);
}

/// So through views: a switch inside a view with a layer shader, its
/// branch swapped, as Zeron's wallpaper swaps its picture.
#[test]
fn a_layer_view_draws_again_as_what_it_shows_is_swapped() {
    use zgx::compose::prelude::*;
    use zgx::widgets::Ui;
    let _gpu = gpu_fixture();
    let shader = FillShader::new(AS_PAINTED, None);
    let mut ui = Ui::new(SIZE as f32, SIZE as f32);
    let which = ui.signal(false);
    let shown = which.clone();
    ui.mount(div().size(100., 100.).layer_shader(shader).child(switch(
        move || shown.get(),
        |blue, _| match blue {
            true => div().size(100., 100.).bg(rgb(0x0000ff)),
            false => div().size(100., 100.).bg(rgb(0xff0000)),
        },
    )));
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut draw = |ui: &Ui, what: &str| {
        ui.try_prepare_frame().unwrap();
        frame(&mut gpu, &mut ui.scene.borrow_mut(), what)
    };
    draw(&ui, "first");
    for (i, blue) in [true, false, true].into_iter().enumerate() {
        which.set(blue);
        let p = draw(&ui, &format!("swap {i}"));
        let wanted = if blue {
            [0, 0, 255, 255]
        } else {
            [255, 0, 0, 255]
        };
        assert_eq!(at(&p, 50, 50), wanted, "swap {i}");
    }
}
