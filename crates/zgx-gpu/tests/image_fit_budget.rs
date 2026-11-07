//! Full-size pictures drawn small, however they come: a texture of the size
//! drawn, no issue raised, none left out, and no frame failing for them.
use std::sync::Arc;
use zgx::{image::ImageData, scene::*};
use zgx_gpu::GpuRenderer;

/// As a window's snapshot on a large display: about 52 MiB decoded.
const W: u32 = 5100;
const H: u32 = 2664;
const DRAWN: (f32, f32) = (300., 157.);
/// Two textures of the size drawn, with room for rounding.
const TWO_DRAWN: usize = 2 * 301 * 158 * 4;

fn raw(width: u32, height: u32, c: [u8; 4]) -> Arc<ImageData> {
    let pixels: Vec<u8> = c
        .iter()
        .copied()
        .cycle()
        .take((width * height * 4) as usize)
        .collect();
    Arc::new(ImageData::new(width, height, pixels).unwrap())
}
fn kept(width: u32, height: u32, c: [u8; 4]) -> Arc<ImageData> {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(width, height, image::Rgba(c))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let image = zgx_gpu::assets::decode_image_kept(Arc::from(png)).unwrap();
    assert!(image.keeps_file());
    image
}
fn sized((width, height): (f32, f32)) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
fn stack(scene: &mut Scene) -> NodeId {
    scene.append(
        scene.root(),
        NodeKind::Container(Layout::Overlay),
        Style::default(),
    )
}
/// Frames until nothing is decoding, each failing the test if it fails.
fn settle(gpu: &mut GpuRenderer, scene: &mut Scene, window: (u32, u32)) -> usize {
    let all = Rect::new(0., 0., window.0 as f32, window.1 as f32);
    let _ = scene.flush();
    let mut uploads = 0;
    for _ in 0..3000 {
        uploads += gpu.render(scene, &[all]).expect("a frame").image_uploads;
        if !gpu.images_decoded() {
            return uploads;
        }
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    panic!("still decoding");
}
fn pixel(gpu: &mut GpuRenderer, window: (u32, u32), (x, y): (usize, usize)) -> [u8; 3] {
    let pixels = gpu.readback().unwrap();
    let at = (y * window.0 as usize + x) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}
const WINDOWS: [(u32, u32); 2] = [(420, 320), (1600, 1000)];
type Make = fn(u32, u32, [u8; 4]) -> Arc<ImageData>;
const MAKERS: [(&str, Make); 2] = [("raw", raw), ("kept", kept)];

/// One fading out as the next fades in, in a popup's window and a large one.
#[test]
fn two_snapshots_crossfading_take_the_size_they_are_drawn() {
    for window in WINDOWS {
        for (name, make) in MAKERS {
            let (a, b) = (
                make(W, H, [230, 20, 20, 255]),
                make(W, H, [20, 20, 230, 255]),
            );
            let mut scene = Scene::new(window.0 as f32, window.1 as f32);
            let stack = stack(&mut scene);
            let a = scene.append(stack, NodeKind::Image(a), sized(DRAWN));
            let b = scene.append(stack, NodeKind::Image(b), sized(DRAWN));
            let mut gpu = GpuRenderer::new(window.0, window.1).unwrap();
            for (fade, red, blue) in [(0., 230, 0), (0.5, 60, 100), (1., 20, 230)] {
                let opacity = |opacity: f32| Effects {
                    opacity,
                    ..Default::default()
                };
                scene.set_effects(a, opacity(1. - fade));
                scene.set_effects(b, opacity(fade));
                settle(&mut gpu, &mut scene, window);
                let [r, _, bl] = pixel(&mut gpu, window, (50, 50));
                assert!(
                    r as i32 >= red - 15 && bl as i32 >= blue - 15,
                    "{name} {window:?} {fade}: {r} {bl}"
                );
            }
            let stats = gpu.debug_cache_stats();
            assert_eq!(stats.image_textures, 2, "{name} {window:?}");
            assert_eq!(stats.images_softer, 0, "{name} {window:?}");
            assert!(
                stats.image_bytes <= TWO_DRAWN,
                "{name} {window:?}: {}",
                stats.image_bytes
            );
            assert_eq!(gpu.take_issues(), Vec::<String>::new(), "{name} {window:?}");
        }
    }
}

/// An image masking something is a picture too: its texture takes the size it
/// masks at, and a kept file decoding fails no frame and shows nothing
/// unmasked while it does.
#[test]
fn a_mask_takes_the_size_it_is_drawn_and_fails_no_frame() {
    let window = WINDOWS[0];
    for (name, make) in MAKERS {
        let mut scene = Scene::new(window.0 as f32, window.1 as f32);
        let stack = stack(&mut scene);
        for _ in 0..2 {
            let holder = scene.append(stack, NodeKind::Container(Layout::Overlay), sized(DRAWN));
            let n = scene.append(
                holder,
                NodeKind::Rect(Color(200, 40, 40, 255)),
                sized(DRAWN),
            );
            scene.set_mask(
                n,
                Some(ImageMask {
                    image: make(W, H, [255, 255, 255, 255]),
                    offset: (0., 0.),
                    size: DRAWN,
                }),
            );
        }
        let mut gpu = GpuRenderer::new(window.0, window.1).unwrap();
        let all = Rect::new(0., 0., window.0 as f32, window.1 as f32);
        let _ = scene.flush();
        // Every frame stands; until decoded, nothing shows where it masks.
        let first = gpu.render(&scene, &[all]);
        assert!(first.is_ok(), "{name}: {:?}", first.err().map(|e| e.0));
        if name == "kept" {
            assert_eq!(
                pixel(&mut gpu, window, (50, 50))[0],
                0,
                "not shown unmasked"
            );
        }
        settle(&mut gpu, &mut scene, window);
        for _ in 0..3 {
            gpu.render(&scene, &[all]).expect("a frame");
        }
        assert_eq!(
            pixel(&mut gpu, window, (50, 50))[0],
            200,
            "{name}: masked in"
        );
        let stats = gpu.debug_cache_stats();
        assert!(
            stats.image_bytes <= TWO_DRAWN,
            "{name}: {}",
            stats.image_bytes
        );
        assert_eq!(gpu.take_issues(), Vec::<String>::new(), "{name}");
    }
}

/// A mask's alpha lands where it should at its fitted size, and follows when
/// drawn larger: its texture is another then.
#[test]
fn a_fitted_mask_masks_in_the_right_place_and_follows_its_size() {
    // Its left half opaque.
    let (width, height) = (1000u32, 500u32);
    let mut pixels = vec![255u8; (width * height * 4) as usize];
    for (at, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        pixel[3] = if (at as u32 % width) < width / 2 {
            255
        } else {
            0
        };
    }
    let image = Arc::new(ImageData::new(width, height, pixels).unwrap());
    let window = (400, 200);
    let mut scene = Scene::new(window.0 as f32, window.1 as f32);
    let n = scene.append(
        scene.root(),
        NodeKind::Rect(Color(200, 40, 40, 255)),
        sized((400., 200.)),
    );
    let mask = |size: (f32, f32)| ImageMask {
        image: image.clone(),
        offset: (0., 0.),
        size,
    };
    scene.set_mask(n, Some(mask((100., 50.))));
    let mut gpu = GpuRenderer::new(window.0, window.1).unwrap();
    settle(&mut gpu, &mut scene, window);
    assert_eq!(pixel(&mut gpu, window, (20, 25))[0], 200, "its opaque half");
    assert_eq!(pixel(&mut gpu, window, (80, 25))[0], 0, "its clear half");
    assert!(gpu.debug_cache_stats().image_bytes <= 101 * 51 * 4);
    scene.set_mask(n, Some(mask((400., 200.))));
    settle(&mut gpu, &mut scene, window);
    assert_eq!(
        pixel(&mut gpu, window, (150, 100))[0],
        200,
        "larger: opaque half"
    );
    assert_eq!(
        pixel(&mut gpu, window, (250, 100))[0],
        0,
        "larger: clear half"
    );
    assert!(gpu.debug_cache_stats().image_bytes <= 401 * 201 * 4);
}

/// Laid out larger than the budget holds, clipped to a card: softer, and
/// uploaded once, not again on every frame.
#[test]
fn a_picture_past_the_budget_uploads_once() {
    let window = WINDOWS[0];
    let (width, height) = (5000u32, 2500u32);
    for (name, make) in MAKERS {
        let mut scene = Scene::new(window.0 as f32, window.1 as f32);
        let stack = stack(&mut scene);
        let mut pictures = Vec::new();
        for shade in [40, 90, 140] {
            let card = scene.append(
                stack,
                NodeKind::Container(Layout::Overlay),
                Style {
                    clip: true,
                    ..sized(DRAWN)
                },
            );
            let image = make(width, height, [shade, 20, 230, 255]);
            pictures.push(scene.append(
                card,
                NodeKind::Image(image),
                sized((width as f32, height as f32)),
            ));
        }
        let mut gpu = GpuRenderer::new(window.0, window.1).unwrap();
        // (They come to their shares over a frame or two.)
        for _ in 0..4 {
            settle(&mut gpu, &mut scene, window);
        }
        // Fading: every frame draws them, and none uploads again.
        for frame in 0..4 {
            for picture in &pictures {
                let opacity = 0.9 - frame as f32 * 0.1;
                scene.set_effects(
                    *picture,
                    Effects {
                        opacity,
                        ..Default::default()
                    },
                );
            }
            let uploads = settle(&mut gpu, &mut scene, window);
            assert_eq!(uploads, 0, "{name}: frame {frame}");
        }
        let stats = gpu.debug_cache_stats();
        assert_eq!(stats.image_textures, 3, "{name}");
        // No issue, but devtools can find them.
        assert!(stats.images_softer >= 1, "{name}");
        assert!(
            stats.image_bytes <= 64 << 20,
            "{name}: {}",
            stats.image_bytes
        );
        assert_eq!(gpu.take_issues(), Vec::<String>::new(), "{name}");
    }
}

/// However many are past the budget, each shows and no frame fails. (Its last
/// ones came to nothing, an error raised, with the budget used up.)
#[test]
fn a_budget_used_up_leaves_no_picture_out() {
    let window = WINDOWS[0];
    let (width, height) = (1400u32, 1400u32);
    let pixels: Arc<[u8]> = [20u8, 200, 20, 255]
        .iter()
        .copied()
        .cycle()
        .take((width * height * 4) as usize)
        .collect();
    let mut scene = Scene::new(window.0 as f32, window.1 as f32);
    let stack = stack(&mut scene);
    const COUNT: usize = 48;
    for _ in 0..COUNT {
        let image = Arc::new(ImageData::new(width, height, pixels.clone()).unwrap());
        scene.append(
            stack,
            NodeKind::Image(image),
            sized((width as f32, height as f32)),
        );
    }
    let mut gpu = GpuRenderer::new(window.0, window.1).unwrap();
    settle(&mut gpu, &mut scene, window);
    let stats = gpu.debug_cache_stats();
    assert_eq!(stats.image_textures, COUNT);
    assert!(
        stats.image_bytes <= (64 << 20) + COUNT * 64 * 64 * 4,
        "{}",
        stats.image_bytes
    );
    assert_eq!(gpu.take_issues(), Vec::<String>::new());
    assert_eq!(
        pixel(&mut gpu, window, (50, 50))[1],
        200,
        "the last one shows"
    );
}

/// Kept files past the budget decode no larger than their textures will be.
#[test]
fn kept_files_past_the_budget_decode_no_more_than_they_show() {
    let window = WINDOWS[0];
    let (width, height) = (4000u32, 2500u32);
    let mut scene = Scene::new(window.0 as f32, window.1 as f32);
    let stack = stack(&mut scene);
    let images: Vec<_> = (0..4)
        .map(|i| kept(width, height, [40 * i, 20, 230, 255]))
        .collect();
    for image in &images {
        scene.append(
            stack,
            NodeKind::Image(image.clone()),
            sized((width as f32, height as f32)),
        );
    }
    let mut gpu = GpuRenderer::new(window.0, window.1).unwrap();
    settle(&mut gpu, &mut scene, window);
    let stats = gpu.debug_cache_stats();
    assert_eq!(stats.image_textures, 4);
    // Their decoded copies, before a host gives them back: four whole were
    // 160 MB. (As they come to even shares, some decode twice.)
    let decoded: usize = (images.iter())
        .map(|image| image.pixels_at((1, 1)).expect("decoded").pixels.len())
        .sum();
    assert!(decoded < 128 << 20, "{decoded}");
    assert_eq!(gpu.take_issues(), Vec::<String>::new());
}

/// Softer and coarser for their budgets raises no issue: `zgx inspect memory`
/// is where a developer finds them.
#[test]
fn the_memory_report_names_what_went_softer() {
    let window = WINDOWS[0];
    let (width, height) = (5000u32, 2500u32);
    let mut scene = Scene::new(window.0 as f32, window.1 as f32);
    let stack = stack(&mut scene);
    for shade in [40, 90] {
        let image = raw(width, height, [shade, 20, 230, 255]);
        scene.append(
            stack,
            NodeKind::Image(image),
            sized((width as f32, height as f32)),
        );
    }
    let layer = scene.append(
        stack,
        NodeKind::Rect(Color(220, 30, 30, 255)),
        Style {
            flex_shrink: 0.,
            ..sized((12_000., 12_000.))
        },
    );
    scene.set_isolated(layer, true);
    let mut gpu = GpuRenderer::new(window.0, window.1).unwrap();
    settle(&mut gpu, &mut scene, window);
    assert_eq!(gpu.take_issues(), Vec::<String>::new());
    let mut report = zgx::memory::Report::default();
    gpu.memory(&mut report);
    let line = (report.lines.iter())
        .find(|line| line.name == "images softer for the budget")
        .expect("its line");
    assert_eq!(line.entries, Some(1));
    assert!(
        report
            .lines
            .iter()
            .any(|line| line.area == "gpu layer" && line.name.contains("coarser")),
        "{:?}",
        report
            .lines
            .iter()
            .map(|line| &line.name)
            .collect::<Vec<_>>()
    );
}

/// Pictures that don't all fit share the budget evenly, whichever is drawn
/// first, and small ones among them stay sharp. As they go, the rest take
/// their room back.
#[test]
fn pictures_past_the_budget_share_it_evenly() {
    let window = WINDOWS[0];
    let (width, height) = (5000u32, 2500u32);
    let full = (width as f32, height as f32);
    let mut scene = Scene::new(window.0 as f32, window.1 as f32);
    let stack = stack(&mut scene);
    let (a, b) = (
        raw(width, height, [200, 20, 20, 255]),
        raw(width, height, [20, 200, 20, 255]),
    );
    let small = raw(100, 50, [20, 20, 200, 255]);
    let first = scene.append(stack, NodeKind::Image(a.clone()), sized(full));
    scene.append(stack, NodeKind::Image(b.clone()), sized(full));
    scene.append(stack, NodeKind::Image(small.clone()), sized((100., 50.)));
    let mut gpu = GpuRenderer::new(window.0, window.1).unwrap();
    for _ in 0..4 {
        settle(&mut gpu, &mut scene, window);
    }
    let (at_a, at_b) = (
        gpu.debug_image_size(&a).unwrap(),
        gpu.debug_image_size(&b).unwrap(),
    );
    assert_eq!(at_a, at_b, "even");
    let each = at_a.0 as usize * at_a.1 as usize * 4;
    assert!(each > 28 << 20, "not far under its share: {at_a:?}");
    assert_eq!(gpu.debug_image_size(&small), Some((100, 50)), "stays sharp");
    assert!(gpu.debug_cache_stats().image_bytes <= 64 << 20);
    assert_eq!(gpu.debug_cache_stats().images_softer, 2);
    // Settled: no more uploads.
    assert_eq!(settle(&mut gpu, &mut scene, window), 0);
    // One gone: the other takes its room back.
    scene.remove(first);
    for _ in 0..4 {
        settle(&mut gpu, &mut scene, window);
    }
    assert_eq!(gpu.debug_image_size(&b), Some((width, height)));
    assert_eq!(gpu.debug_cache_stats().images_softer, 0);
    assert_eq!(gpu.take_issues(), Vec::<String>::new());
}

/// A box laid out at a picture's full size under a parent scaling it down
/// rasterizes at what shows, not at half of what it was laid out as.
#[test]
fn a_picture_scaled_down_by_its_parent_takes_what_it_shows() {
    let window = WINDOWS[1];
    let mut scene = Scene::new(window.0 as f32, window.1 as f32);
    let stack = stack(&mut scene);
    let image = raw(W, H, [200, 20, 20, 255]);
    let holder = scene.append(
        stack,
        NodeKind::Container(Layout::Overlay),
        Style::default(),
    );
    scene.append(
        holder,
        NodeKind::Image(image.clone()),
        sized((W as f32, H as f32)),
    );
    let scale = DRAWN.0 / W as f32;
    scene.set_local_transform(
        holder,
        LocalTransform {
            scale: (scale, scale),
            origin: (0., 0.),
            ..Default::default()
        },
    );
    let mut gpu = GpuRenderer::new(window.0, window.1).unwrap();
    settle(&mut gpu, &mut scene, window);
    let stats = gpu.debug_cache_stats();
    // A sixteenth, the finest it goes: about a megabyte, where it took 13.
    assert!(stats.image_bytes <= 2 << 20, "{}", stats.image_bytes);
    assert_eq!(pixel(&mut gpu, window, (100, 50))[0], 200);
    assert_eq!(gpu.take_issues(), Vec::<String>::new());
}
