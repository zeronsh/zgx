//! Images' own textures past their budget (64 MiB): those not drawn this
//! frame go, least recently drawn first, and upload again when drawn. A long
//! page of photos scrolled through drew until its 54th, then every frame
//! failed.
use std::sync::Arc;
use zgx::{image::ImageData, scene::*};
use zgx_gpu::GpuRenderer;

fn photo(i: u32) -> NodeKind {
    let pixels: Vec<u8> = (0..640 * 480)
        .flat_map(|p| [(p % 251) as u8, (i * 2) as u8, 90, 255])
        .collect();
    NodeKind::Image(Arc::new(ImageData::new(640, 480, pixels).unwrap()))
}
fn photo_style() -> Style {
    Style {
        width: Some(640.),
        height: Some(480.),
        ..Default::default()
    }
}
fn page(height: f32, photos: u32) -> (Scene, NodeId) {
    let mut scene = Scene::new(800., height);
    let column = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Column),
        Style::default(),
    );
    for i in 0..photos {
        scene.append(column, photo(i), photo_style());
    }
    (scene, column)
}
fn scroll(scene: &mut Scene, column: NodeId, to: f32) -> Vec<Rect> {
    scene.set_style(
        column,
        Style {
            margin: Insets {
                top: -to,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    scene.flush().damage
}

#[test]
fn a_long_page_of_photos_scrolls_down_and_back() {
    let (mut scene, column) = page(600., 100);
    let mut gpu = GpuRenderer::new(800, 600).unwrap();
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    // 100 photos of 1.2 MB: the budget holds about 54.
    for step in 1..100 {
        let damage = scroll(&mut scene, column, step as f32 * 480.);
        gpu.render(&scene, &damage)
            .unwrap_or_else(|e| panic!("photo {step}: {e}"));
    }
    // Back to the top: the photos given back upload again as they show.
    let mut uploads = 0;
    for step in (0..99).rev() {
        let damage = scroll(&mut scene, column, step as f32 * 480.);
        uploads += gpu.render(&scene, &damage).unwrap().image_uploads;
    }
    assert!(
        uploads > 0 && uploads < 99,
        "{uploads} uploads scrolling back"
    );
}

/// More than the budget drawn in one frame: nothing drawn there can go, so
/// the last are drawn at a lower resolution, and none is left out, nor an
/// issue raised. (Stacked, half transparent: each one shows.)
#[test]
fn more_photos_than_the_budget_in_one_frame_draw_what_fits() {
    let mut scene = Scene::new(800., 600.);
    let stack = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Overlay),
        Style::default(),
    );
    for i in 0..60u32 {
        let pixels: Vec<u8> = (0..640 * 480)
            .flat_map(|p| [(p % 251) as u8, (i * 2) as u8, 90, 128])
            .collect();
        let image = Arc::new(ImageData::new(640, 480, pixels).unwrap());
        scene.append(stack, NodeKind::Image(image), photo_style());
    }
    let mut gpu = GpuRenderer::new(800, 600).unwrap();
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    assert_eq!(gpu.take_issues(), Vec::<String>::new());
    let stats = gpu.debug_cache_stats();
    assert_eq!(stats.image_textures, 60, "every one drawn");
    // (With what the last, each cut to 64 pixels, may go over it.)
    assert!(
        stats.image_bytes <= 64 * 1024 * 1024 + 60 * 64 * 48 * 4,
        "{}",
        stats.image_bytes
    );
}

/// A script's picture keeps its file: nothing is decoded until it is
/// drawn, the first frame goes without it (and says it waited), and once
/// decoded off the main thread it draws at the size it is shown, its
/// decoded copy ready to go.
#[test]
fn a_kept_file_draws_once_decoded_at_the_size_it_shows() {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(400, 300, image::Rgba([230, 20, 20, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let picture = zgx_gpu::assets::decode_image_kept(Arc::from(png)).unwrap();
    assert!(picture.keeps_file());
    assert_eq!((picture.width(), picture.height()), (400, 300));
    let mut scene = Scene::new(200., 200.);
    scene.append(
        scene.root(),
        NodeKind::Image(picture.clone()),
        Style {
            width: Some(100.),
            height: Some(75.),
            ..Default::default()
        },
    );
    let mut gpu = GpuRenderer::new(200, 200).unwrap();
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    // (`images_decoded` takes that the first frame waited for it.)
    assert_eq!(gpu.debug_cache_stats().image_textures, 0);
    let mut drawn = false;
    for _ in 0..500 {
        std::thread::sleep(std::time::Duration::from_millis(4));
        // As the host does once told they are decoded: not otherwise, so a
        // frame that patched only what changed would be caught.
        if gpu.images_decoded() {
            gpu.render(&scene, &[Rect::new(0., 0., 200., 200.)])
                .unwrap();
        }
        if gpu.debug_cache_stats().image_textures == 1 {
            drawn = true;
            break;
        }
    }
    assert!(drawn, "drawn once decoded");
    assert_eq!(
        gpu.debug_cache_stats().image_bytes,
        100 * 75 * 4,
        "at the size it shows"
    );
    assert!(gpu.take_images_uploaded(), "its decoded copy may go");
    assert!(gpu.readback().unwrap()[(30 * 200 + 30) * 4] > 200);
}

/// A picture's row scrolled out of a list and back: its node is new, its image
/// the same, its decoded pixels given up long since. Its texture waited, so it
/// draws in the frame it returns, not blank until it decodes again; once the
/// window has gone idle it has gone, and memory with it.
#[test]
fn a_kept_file_returning_to_the_scene_draws_at_once() {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(400, 300, image::Rgba([230, 20, 20, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let picture = zgx_gpu::assets::decode_image_kept(Arc::from(png)).unwrap();
    let style = || Style {
        width: Some(100.),
        height: Some(75.),
        ..Default::default()
    };
    let whole = [Rect::new(0., 0., 200., 200.)];
    let mut scene = Scene::new(200., 200.);
    let mut gpu = GpuRenderer::new(200, 200).unwrap();
    let node = scene.append(scene.root(), NodeKind::Image(picture.clone()), style());
    scene.flush();
    gpu.render(&scene, &whole).unwrap();
    for _ in 0..500 {
        std::thread::sleep(std::time::Duration::from_millis(4));
        if gpu.images_decoded() {
            gpu.render(&scene, &whole).unwrap();
        }
        if gpu.debug_cache_stats().image_textures == 1 {
            break;
        }
    }
    assert_eq!(gpu.debug_cache_stats().image_textures, 1, "decoded");
    assert!(gpu.take_images_uploaded());
    // Its pixels given up, as the host does after uploads.
    zgx::image::release_decoded();

    let red = |gpu: &GpuRenderer| gpu.readback().unwrap()[(30 * 200 + 30) * 4];
    scene.remove(node);
    scene.flush();
    gpu.render(&scene, &whole).unwrap();
    assert!(red(&gpu) < 100, "gone with its row");
    let node = scene.append(scene.root(), NodeKind::Image(picture.clone()), style());
    scene.flush();
    gpu.render(&scene, &whole).unwrap();
    assert!(!gpu.images_decoded(), "nothing to wait for");
    assert!(red(&gpu) > 200, "drawn in the frame it returns");

    // Gone again, and the window idle: its texture goes too.
    scene.remove(node);
    scene.flush();
    gpu.render(&scene, &whole).unwrap();
    assert_eq!(gpu.debug_cache_stats().image_textures, 1, "kept");
    gpu.trim();
    assert_eq!(gpu.debug_cache_stats().image_textures, 0, "given back");
}

/// What textures of pictures no node shows may hold is bounded, the least
/// recently drawn going first.
#[test]
fn textures_of_images_gone_are_bounded() {
    let whole = [Rect::new(0., 0., 800., 600.)];
    let mut scene = Scene::new(800., 600.);
    let mut gpu = GpuRenderer::new(800, 600).unwrap();
    // 40 photos of 1.2 MB, each in the scene a frame: 48 MB seen, 32 MB kept.
    for i in 0..40 {
        let node = scene.append(scene.root(), photo(i), photo_style());
        scene.flush();
        gpu.render(&scene, &whole).unwrap();
        scene.remove(node);
        scene.flush();
        gpu.render(&scene, &whole).unwrap();
    }
    let stats = gpu.debug_cache_stats();
    assert!(
        stats.image_bytes <= 32 * 1024 * 1024,
        "{}",
        stats.image_bytes
    );
    assert!(
        stats.image_bytes >= 24 * 1024 * 1024,
        "{}",
        stats.image_bytes
    );
    gpu.trim();
    assert_eq!(gpu.debug_cache_stats().image_bytes, 0);
}

/// A 2 by 2 checker, black and white, drawn at 100 by 100: smooth, its
/// middle blends them; pixelated (`Scene::set_image_rendering`), each point
/// is the nearest of the four, black or white, as an image and as a
/// background image alike.
#[test]
fn pixelated_images_take_the_nearest_pixel() {
    use zgx::decoration::{Background, Decoration};
    use zgx::image::ImageRendering;
    let checker = Arc::new(
        ImageData::new(
            2,
            2,
            vec![
                0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255,
            ],
        )
        .unwrap(),
    );
    // Grey values (neither black nor white) along a row through the middle.
    let greys = |background: bool, rendering: ImageRendering| {
        let mut scene = Scene::new(100., 100.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let kind = match background {
            false => NodeKind::Image(checker.clone()),
            true => NodeKind::Quad(QuadStyle {
                decoration: Some(Arc::new(Decoration {
                    background: Some(Background::Image {
                        image: checker.clone(),
                        fit: Default::default(),
                    }),
                    ..Default::default()
                })),
                ..Default::default()
            }),
        };
        let node = scene.append(
            scene.root(),
            kind,
            Style {
                width: Some(100.),
                height: Some(100.),
                ..Default::default()
            },
        );
        scene.set_image_rendering(node, rendering);
        let mut gpu = GpuRenderer::new(100, 100).unwrap();
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let pixels = gpu.readback().unwrap();
        (0..100)
            .map(|x| pixels[(25 * 100 + x) * 4])
            .filter(|v| (8..248).contains(v))
            .count()
    };
    for background in [false, true] {
        let smooth = greys(background, ImageRendering::Auto);
        assert!(
            smooth > 20,
            "background {background}: smooth blends ({smooth} greys)"
        );
        let pixelated = greys(background, ImageRendering::Pixelated);
        assert_eq!(
            pixelated, 0,
            "background {background}: pixelated is black or white"
        );
    }
}

/// A script's picture (a kept file, decoded as drawn) drawn pixelated takes
/// the nearest pixel too, once its pixels are decoded: a 4 by 4 checker at
/// 64 by 64 is 16 hard squares (Zeron's lightbox at 300%). Said of the box
/// around it, as an image view's style is (its picture a node inside):
/// inherited, as CSS's `image-rendering` is.
#[test]
fn a_kept_file_drawn_pixelated_takes_the_nearest_pixel() {
    use zgx::image::ImageRendering;
    let mut png = Vec::new();
    image::RgbaImage::from_fn(4, 4, |x, y| match (x + y) % 2 {
        0 => image::Rgba([0, 0, 0, 255]),
        _ => image::Rgba([255, 255, 255, 255]),
    })
    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
    .unwrap();
    let greys = |rendering: ImageRendering| {
        let picture = zgx_gpu::assets::decode_image_kept(Arc::from(png.clone())).unwrap();
        let mut scene = Scene::new(64., 64.);
        let sized = Style {
            width: Some(64.),
            height: Some(64.),
            ..Default::default()
        };
        let around = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            sized.clone(),
        );
        scene.append(around, NodeKind::Image(picture), sized);
        scene.set_image_rendering(around, rendering);
        let mut gpu = GpuRenderer::new(64, 64).unwrap();
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        for _ in 0..500 {
            if gpu.debug_cache_stats().image_textures == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(4));
            if gpu.images_decoded() {
                gpu.render(&scene, &[Rect::new(0., 0., 64., 64.)]).unwrap();
            }
        }
        let pixels = gpu.readback().unwrap();
        (0..64)
            .map(|x| pixels[(8 * 64 + x) * 4])
            .filter(|v| (8..248).contains(v))
            .count()
    };
    let smooth = greys(ImageRendering::Auto);
    assert!(smooth > 8, "smooth blends ({smooth} greys)");
    let pixelated = greys(ImageRendering::Pixelated);
    assert_eq!(pixelated, 0, "pixelated is black or white");
}

/// A page of images, every other one pixelated, draws in as many draws as
/// the same page all smooth: each image samples its own texture in a draw
/// of its own either way, the pixelated ones with a sampler of their own.
#[test]
fn pixelated_images_draw_in_as_many_draws() {
    use zgx::image::ImageRendering;
    let images: Vec<_> = (0..20u8)
        .map(|i| Arc::new(ImageData::new(4, 4, [i * 12, 90, 200 - i * 9, 255].repeat(16)).unwrap()))
        .collect();
    let draws = |mixed: bool| {
        let mut scene = Scene::new(500., 200.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        for (i, image) in images.iter().enumerate() {
            let node = scene.append(
                scene.root(),
                NodeKind::Image(image.clone()),
                Style {
                    width: Some(40.),
                    height: Some(40.),
                    ..Default::default()
                },
            );
            scene.set_transform(
                node,
                Transform {
                    x: (i % 10) as f32 * 48.,
                    y: (i / 10) as f32 * 48.,
                },
            );
            if mixed && i % 2 == 0 {
                scene.set_image_rendering(node, ImageRendering::Pixelated);
            }
        }
        let mut gpu = GpuRenderer::new(500, 200).unwrap();
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap().draw_calls
    };
    let (smooth, mixed) = (draws(false), draws(true));
    assert_eq!(mixed, smooth, "draws: {smooth} smooth, {mixed} mixed");
}
