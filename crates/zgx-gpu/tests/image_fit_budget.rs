//! Full-size pictures drawn small, however they come: a texture of the size
//! drawn, no issue raised, none left out, and no frame failing for them.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Once, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant};
use zgx::{image::ImageData, scene::*};
use zgx_gpu::{GpuRenderer, GpuStats};

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
/// How many decodes have finished in this process, and the wait for the next.
static DECODED: (Mutex<u64>, Condvar) = (Mutex::new(0), Condvar::new());
/// No decode finished in this long, in any test: the decoding thread has stuck.
const STALL: Duration = Duration::from_secs(60);
/// Frames until nothing is decoding, each failing the test if it fails. Every
/// test running decodes on the one thread, a file at a time, so how long this
/// waits says little of its own pictures: it fails only if no decode finishes.
fn settle(gpu: &mut GpuRenderer, scene: &mut Scene, window: (u32, u32)) -> usize {
    static COUNTING: Once = Once::new();
    COUNTING.call_once(|| {
        zgx::image::set_decoded_wake(|| {
            *DECODED.0.lock().unwrap() += 1;
            DECODED.1.notify_all();
        })
    });
    let all = Rect::new(0., 0., window.0 as f32, window.1 as f32);
    let _ = scene.flush();
    let mut uploads = 0;
    let mut seen = *DECODED.0.lock().unwrap();
    let mut since = Instant::now();
    loop {
        uploads += gpu.render(scene, &[all]).expect("a frame").image_uploads;
        if !gpu.images_decoded() {
            return uploads;
        }
        // (Drawn again now and then regardless: a decode that fails tells no one.)
        let decoded = *DECODED
            .1
            .wait_timeout_while(
                DECODED.0.lock().unwrap(),
                Duration::from_millis(50),
                |decoded| *decoded == seen,
            )
            .unwrap()
            .0;
        if decoded != seen {
            (seen, since) = (decoded, Instant::now());
        }
        assert!(
            since.elapsed() < STALL,
            "still decoding: none done in {STALL:?}"
        );
    }
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
/// Held by each test: `zgx::image::release_decoded` gives back every kept
/// file's pixels in the process, so a test releasing them would take another's
/// from under it (a decoded copy it reads, or one its frames wait on). One that
/// releases runs alone; the rest together.
static KEPT_COPIES: RwLock<()> = RwLock::new(());
fn decoded_kept() -> RwLockReadGuard<'static, ()> {
    KEPT_COPIES
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
fn releases_decoded() -> RwLockWriteGuard<'static, ()> {
    KEPT_COPIES
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn two_snapshots_crossfading_take_the_size_they_are_drawn() {
    let _decoded = decoded_kept();
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
    let _decoded = decoded_kept();
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
    let _decoded = decoded_kept();
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
    let _decoded = decoded_kept();
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

/// Pictures past the budget whose pixels take a while: each uploads once, at
/// its share, and decodes once for it. Those still waiting count: without them
/// the rest looked as if they fit, and uploaded whole, to go again as they came.
#[test]
fn pictures_past_the_budget_take_their_shares_however_slowly_they_decode() {
    let _decoded = decoded_kept();
    const SIDE: (u32, u32) = (5000, 2500);
    static DECODES: AtomicUsize = AtomicUsize::new(0);
    fn slowly(_: &[u8]) -> Result<Arc<[u8]>, String> {
        DECODES.fetch_add(1, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(200));
        Ok(vec![200; (SIDE.0 * SIDE.1 * 4) as usize].into())
    }
    let window = WINDOWS[0];
    let mut scene = Scene::new(window.0 as f32, window.1 as f32);
    let stack = stack(&mut scene);
    let size = (SIDE.0 as f32, SIDE.1 as f32);
    for _ in 0..3 {
        let card = scene.append(
            stack,
            NodeKind::Container(Layout::Overlay),
            Style {
                clip: true,
                ..sized(DRAWN)
            },
        );
        let image = ImageData::kept(SIDE.0, SIDE.1, Arc::from(&b"file"[..]), slowly).unwrap();
        scene.append(card, NodeKind::Image(Arc::new(image)), sized(size));
    }
    let mut gpu = GpuRenderer::new(window.0, window.1).unwrap();
    let uploads = settle(&mut gpu, &mut scene, window);
    assert_eq!(uploads, 3, "uploads: each once");
    assert_eq!(settle(&mut gpu, &mut scene, window), 0);
    let stats = gpu.debug_cache_stats();
    assert_eq!(stats.image_textures, 3);
    assert_eq!(stats.images_softer, 3);
    assert!(stats.image_bytes <= 64 << 20, "{}", stats.image_bytes);
    assert_eq!(DECODES.load(Ordering::Relaxed), 3, "decodes: each once");
    assert_eq!(gpu.take_issues(), Vec::<String>::new());
}

/// Pictures on screen, when a new one makes the share smaller: each is drawn as
/// it was until what replaces it is ready, in every frame. A kept file whose
/// pixels have gone waits on a decode for that, and evicted it would be blank
/// meanwhile. The textures held for it go as it arrives.
#[test]
fn pictures_on_screen_stay_as_the_share_moves_until_their_replacements_are_ready() {
    let _decoded = releases_decoded();
    const SIDE: u32 = 2500;
    const WINDOW: u32 = 2000;
    const CORNERS: [(usize, usize); 3] = [(100, 100), (700, 700), (1300, 1300)];
    fn slowly(_: &[u8]) -> Result<Arc<[u8]>, String> {
        std::thread::sleep(Duration::from_millis(100));
        Ok(vec![200; (SIDE * SIDE * 4) as usize].into())
    }
    fn raw_one() -> ImageData {
        ImageData::new(SIDE, SIDE, vec![200; (SIDE * SIDE * 4) as usize]).unwrap()
    }
    fn kept_one() -> ImageData {
        ImageData::kept(SIDE, SIDE, Arc::from(&b"file"[..]), slowly).unwrap()
    }
    let all = Rect::new(0., 0., WINDOW as f32, WINDOW as f32);
    for (name, make) in [("raw", raw_one as fn() -> ImageData), ("kept", kept_one)] {
        let Ok(mut gpu) = GpuRenderer::new(WINDOW, WINDOW) else {
            return;
        };
        let mut scene = Scene::new(WINDOW as f32, WINDOW as f32);
        let root = stack(&mut scene);
        let side = (SIDE as f32, SIDE as f32);
        let add = |scene: &mut Scene, at: f32| {
            let node = scene.append(root, NodeKind::Image(Arc::new(make())), sized(side));
            scene.set_transform(node, Transform { x: at, y: at });
        };
        add(&mut scene, 0.);
        add(&mut scene, 600.);
        settle(&mut gpu, &mut scene, (WINDOW, WINDOW));
        assert_eq!(gpu.debug_cache_stats().image_bytes, 50_000_000, "{name}");
        // As a host does soon after they upload.
        zgx::image::release_decoded();
        add(&mut scene, 1200.);
        let _ = scene.flush();
        let (mut most, mut uploads, mut started, mut settled) = (0, 0, Instant::now(), 0);
        while settled < 3 {
            uploads += gpu.render(&scene, &[all]).expect("a frame").image_uploads;
            let waiting = gpu.images_decoded();
            let pixels = gpu.readback().unwrap();
            for (x, y) in &CORNERS[..2] {
                let alpha = pixels[(y * WINDOW as usize + x) * 4 + 3];
                assert!(alpha > 0, "{name}: blank on screen at {x},{y}");
            }
            most = most.max(gpu.debug_cache_stats().image_bytes);
            settled = if waiting { 0 } else { settled + 1 };
            if waiting {
                started = Instant::now();
                std::thread::sleep(Duration::from_millis(4));
            }
            assert!(started.elapsed() < STALL, "{name}: never settled");
        }
        let pixels = gpu.readback().unwrap();
        for (x, y) in CORNERS {
            let alpha = pixels[(y * WINDOW as usize + x) * 4 + 3];
            assert!(alpha > 0, "{name}: {x},{y} never showed");
        }
        let stats = gpu.debug_cache_stats();
        assert_eq!(stats.image_textures, 3, "{name}");
        assert!(stats.images_softer > 0, "{name}");
        assert!(
            stats.image_bytes <= 64 << 20,
            "{name}: what was held for it went: {}",
            stats.image_bytes
        );
        // (A raw one uploads as it comes, before the share moves, and again.)
        assert!(uploads <= 4, "{name}: {uploads} uploads");
        assert!(
            most <= (64 << 20) + 50_000_000,
            "{name}: at most the budget and what it replaced: {most}"
        );
    }
}

/// However many are past the budget, each shows and no frame fails. (Its last
/// ones came to nothing, an error raised, with the budget used up.)
#[test]
fn a_budget_used_up_leaves_no_picture_out() {
    let _decoded = decoded_kept();
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
    let _decoded = decoded_kept();
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
    let _decoded = decoded_kept();
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
    let _decoded = decoded_kept();
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
    let _decoded = decoded_kept();
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

/// Three pictures of 25 MB in a window of 2000: two fit the 64 MiB, three
/// don't. Each has a corner only it covers.
struct Crowded {
    gpu: GpuRenderer,
    scene: Scene,
    nodes: Vec<NodeId>,
}
impl Crowded {
    const SIDE: u32 = 2500;
    const WINDOW: u32 = 2000;
    const CORNERS: [(usize, usize); 3] = [(100, 100), (700, 700), (1300, 1300)];
    fn new() -> Option<Self> {
        let gpu = GpuRenderer::new(Self::WINDOW, Self::WINDOW).ok()?;
        let mut scene = Scene::new(Self::WINDOW as f32, Self::WINDOW as f32);
        let root = scene.root();
        scene.set_kind(root, NodeKind::Container(Layout::Overlay));
        Some(Self {
            gpu,
            scene,
            nodes: Vec::new(),
        })
    }
    fn add(&mut self, parent: NodeId, at: f32) {
        let side = Self::SIDE as f32;
        let pixels = vec![200; (Self::SIDE * Self::SIDE * 4) as usize];
        let picture = Arc::new(ImageData::new(Self::SIDE, Self::SIDE, pixels).unwrap());
        let node = self
            .scene
            .append(parent, NodeKind::Image(picture), sized((side, side)));
        self.scene.set_transform(node, Transform { x: at, y: at });
        self.nodes.push(node);
    }
    fn frame(&mut self) -> GpuStats {
        let _ = self.scene.flush();
        let all = Rect::new(0., 0., Self::WINDOW as f32, Self::WINDOW as f32);
        self.gpu.render(&self.scene, &[all]).expect("a frame")
    }
    /// Which corners show a picture.
    fn showing(&mut self) -> [bool; 3] {
        let pixels = self.gpu.readback().unwrap();
        Self::CORNERS.map(|(x, y)| pixels[(y * Self::WINDOW as usize + x) * 4 + 3] > 0)
    }
}

/// A picture on screen may not come through its upload in a frame (a frame
/// paints what it kept), so it isn't marked drawn when a new one needs room:
/// with none off screen to give way, the new one goes softer, and all show.
/// Whichever way the new one arrives: beside the others (the frame prepares
/// them too), or in a box of its own, before or after them (it prepares only
/// that).
#[test]
fn a_new_picture_past_the_budget_goes_softer_and_those_on_screen_stay() {
    let _decoded = decoded_kept();
    for (own_box, first) in [(false, false), (true, false), (true, true)] {
        let Some(mut w) = Crowded::new() else {
            return;
        };
        let root = w.scene.root();
        let boxed = |scene: &mut Scene| {
            scene.append(root, NodeKind::Container(Layout::Overlay), Style::default())
        };
        let before = (own_box && first).then(|| boxed(&mut w.scene));
        w.add(root, 0.);
        w.add(root, 600.);
        let after = (own_box && !first).then(|| boxed(&mut w.scene));
        w.frame();
        // A frame that paints what it kept, as a window's does.
        w.frame();
        w.add(before.or(after).unwrap_or(root), 1200.);
        let case = format!("own box {own_box}, first {first}");
        for frame in 0..2 {
            w.frame();
            assert_eq!(w.showing(), [true; 3], "{case}, frame {frame}");
            let cache = w.gpu.debug_cache_stats();
            assert_eq!(cache.image_textures, 3, "{case}: one gave way");
            assert!(cache.images_softer > 0, "{case}: none went softer");
            assert!(
                cache.image_bytes <= 64 << 20,
                "{case}: {} bytes",
                cache.image_bytes
            );
        }
    }
}

/// A picture scrolled off the window does give way, whole, to one on it: only
/// those showing are spared.
#[test]
fn a_picture_off_the_window_gives_way_to_a_new_one() {
    let _decoded = decoded_kept();
    let Some(mut w) = Crowded::new() else {
        return;
    };
    let root = w.scene.root();
    w.add(root, 0.);
    w.add(root, 600.);
    let own = w
        .scene
        .append(root, NodeKind::Container(Layout::Overlay), Style::default());
    w.frame();
    // Off the window, and a frame on.
    let first = w.nodes[0];
    w.scene.set_transform(first, Transform { x: 0., y: 3000. });
    w.frame();
    w.add(own, 1200.);
    w.frame();
    assert_eq!(w.showing(), [false, true, true]);
    let cache = w.gpu.debug_cache_stats();
    assert_eq!(cache.image_textures, 2, "it kept its texture");
    assert_eq!(cache.images_softer, 0, "drawn softer with room to be had");
    // Back on the window, it uploads again.
    w.scene.set_transform(first, Transform { x: 0., y: 0. });
    assert_eq!(w.frame().image_uploads, 1);
    assert_eq!(w.showing(), [true; 3]);
}
