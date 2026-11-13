use super::*;
use zgx::scene::{Color, Rect};
/// A board of rounded cards, each clipped, at fractional points: wider and
/// taller than the box.
fn board(shade: u8) -> Arc<Canvas> {
    let mut canvas = Canvas::new();
    for row in 0..9 {
        for col in 0..12 {
            let (x, y) = (col as f32 * 157.3 + 3.7, row as f32 * 131.9 + 5.2);
            let card = Rect::new(x, y, 120.4, 101.7);
            canvas.clip(Path::rounded_rect(card, 9.5).unwrap(), FillRule::Winding);
            canvas.fill(
                Path::rounded_rect(card, 9.5).unwrap(),
                Color(shade, (col * 20) as u8, (row * 25) as u8, 255),
            );
            canvas.fill(
                Path::rounded_rect(Rect::new(x + 10.3, y + 60.1, 140., 20.6), 4.).unwrap(),
                Color(20, 30, 200, 180),
            );
            canvas.restore();
        }
    }
    Arc::new(canvas)
}
const W: f32 = 700.;
const H: f32 = 450.;
/// What `out` shows, unscaled, put together into one image of the box.
fn composed(out: &[(Raster, Placement)], scale: f32) -> Vec<u8> {
    let size = [(W * scale).ceil() as usize, (H * scale).ceil() as usize];
    let mut image = vec![0u8; size[0] * size[1] * 4];
    for (raster, placement) in out {
        assert!(!placement.scaled, "every tile of the zoom shown");
        let pixels = raster.pixels.as_ref().unwrap();
        let [x, y, w, h] = placement.rect.map(|v| (v * scale).round() as usize);
        let (u, v) = (
            (placement.uv[0] * TILE as f32).round() as usize,
            (placement.uv[1] * TILE as f32).round() as usize,
        );
        for row in 0..h.min(size[1] - y) {
            for col in 0..w.min(size[0] - x) {
                let src = ((v + row) * TILE as usize + u + col) * 4;
                let dst = ((y + row) * size[0] + x + col) * 4;
                image[dst..dst + 4].copy_from_slice(&pixels.pixels()[src..src + 4]);
            }
        }
    }
    image
}
/// What the box's raster shows of `canvas` drawn under `drawn`, panned by
/// `shift` device pixels.
fn raster(canvas: &Arc<Canvas>, drawn: View, shift: [i32; 2], scale: f32) -> Vec<u8> {
    let size = [(W * scale).ceil() as i32, (H * scale).ceil() as i32];
    let text = SharedText::default();
    let painted = panned_region(canvas, drawn, size, scale, None).painted;
    let seen = [-shift[0], -shift[1], size[0] - shift[0], size[1] - shift[1]];
    // With a margin, cut away: the raster's own edge is antialiased a
    // little differently (as each tile's would be, without theirs).
    let m = MARGIN;
    let grown = [seen[0] - m, seen[1] - m, seen[2] + m, seen[3] + m];
    let image = rasterize_region(canvas, drawn, scale, grown, None, &text).unwrap();
    let stride = (size[0] + 2 * m) as usize;
    let mut pixels = vec![0u8; (size[0] * size[1] * 4) as usize];
    for (row, line) in pixels.chunks_exact_mut(size[0] as usize * 4).enumerate() {
        let src = ((row + m as usize) * stride + m as usize) * 4;
        line.copy_from_slice(&image.pixels()[src..src + line.len()]);
    }
    // Outside what it paints, nothing (as the box's raster leaves it).
    for y in 0..size[1] {
        for x in 0..size[0] {
            let (dx, dy) = (x + seen[0], y + seen[1]);
            if dx < painted[0] || dy < painted[1] || dx >= painted[2] || dy >= painted[3] {
                let i = (y * size[0] + x) as usize * 4;
                pixels[i..i + 4].fill(0);
            }
        }
    }
    pixels
}
/// Whether `a` (tiles) and `b` (the raster) are the same pixels, but on
/// drawn edges: tiny-skia's antialiasing depends a little on where a
/// pixmap starts, and it cuts a curve where it leaves one, which flattens
/// it a little differently (as two strips of the box's raster do). Away
/// from edges, within 2.
fn same(a: &[u8], b: &[u8], scale: f32) -> bool {
    let width = (W * scale).ceil() as usize;
    alike(a, b, width)
}
/// Tiles until none is awaited.
fn settled(
    cache: &mut CanvasCache,
    node: NodeId,
    canvas: &Arc<Canvas>,
    view: View,
    scale: f32,
) -> Vec<(Raster, Placement)> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut out = Vec::new();
        let awaiting =
            (cache.get_tiles(node, canvas, view, W, H, scale, false, 0, &mut out)).unwrap();
        if !awaiting {
            return out;
        }
        assert!(Instant::now() < deadline, "tiles land");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn node() -> NodeId {
    let mut scene = Scene::new(100., 100.);
    let root = scene.root();
    scene.append(
        root,
        NodeKind::Canvas(Arc::new(Canvas::new())),
        Default::default(),
    )
}
fn view(x: f32, y: f32, scale: f32) -> View {
    View { x, y, scale }
}
#[test]
fn a_pan_past_the_raster_draws_tiles_off_the_frame_matching_the_raster() {
    for scale in [1., 1.5] {
        let canvas = board(200);
        let node = node();
        let mut cache = CanvasCache::default();
        let first = view(-40.3, -25.7, 1.);
        cache
            .get_panned(node, &canvas, first, W, H, scale, false)
            .unwrap();
        assert!(!cache.tiled(node));
        // Past it: tiled from now on, nothing rasterized on the frame.
        let panned = view(-640.3, -425.2, 1.);
        let (_, none) = cache
            .get_panned(node, &canvas, panned, W, H, scale, false)
            .unwrap();
        assert_eq!(none.rect[2] * none.rect[3], 0.);
        assert!(cache.tiled(node));
        let mut out = Vec::new();
        let begun = Instant::now();
        (cache.get_tiles(node, &canvas, panned, W, H, scale, false, 0, &mut out)).unwrap();
        assert!(
            begun.elapsed() < Duration::from_millis(50),
            "nothing made on the frame"
        );
        let out = settled(&mut cache, node, &canvas, panned, scale);
        let shift = [
            ((panned.x - first.x) * scale).round() as i32,
            ((panned.y - first.y) * scale).round() as i32,
        ];
        let (tiles, raster) = (composed(&out, scale), raster(&canvas, first, shift, scale));
        assert!(
            same(&tiles, &raster, scale),
            "tiles at rest are the raster's pixels (scale {scale})"
        );
        if scale == 1. {
            assert!(tiles == raster, "at whole pixels, exactly");
        }
    }
}
#[test]
fn a_zoom_shows_another_level_scaled_then_its_own_tiles_matching_its_raster() {
    let canvas = board(120);
    let node = node();
    let mut cache = CanvasCache::default();
    let first = view(-10., -10., 1.);
    cache
        .get_panned(node, &canvas, first, W, H, 1., false)
        .unwrap();
    let zoomed = view(-300.25, -180.5, 1.7);
    cache
        .get_panned(node, &canvas, zoomed, W, H, 1., false)
        .unwrap();
    assert!(cache.tiled(node));
    let mut out = Vec::new();
    let awaiting = (cache.get_tiles(node, &canvas, zoomed, W, H, 1., false, 0, &mut out)).unwrap();
    assert!(awaiting);
    assert!(
        out.iter().all(|(_, p)| p.scaled),
        "the zoom before stands in"
    );
    let out = settled(&mut cache, node, &canvas, zoomed, 1.);
    assert!(same(
        &composed(&out, 1.),
        &raster(&canvas, zoomed, [0, 0], 1.),
        1.
    ));
    // Back at the first zoom: its level's tiles, made off the frame.
    let mut out = Vec::new();
    (cache.get_tiles(node, &canvas, first, W, H, 1., false, 0, &mut out)).unwrap();
    let out = settled(&mut cache, node, &canvas, first, 1.);
    assert!(same(
        &composed(&out, 1.),
        &raster(&canvas, first, [0, 0], 1.),
        1.
    ));
}
#[test]
fn a_changed_drawing_shows_at_once_and_tiles_keep_to_their_budget() {
    let canvas = board(60);
    let node = node();
    let mut cache = CanvasCache::default();
    let mut at = view(0., 0., 1.);
    cache
        .get_panned(node, &canvas, at, W, H, 2., false)
        .unwrap();
    at.x -= 900.;
    cache
        .get_panned(node, &canvas, at, W, H, 2., false)
        .unwrap();
    settled(&mut cache, node, &canvas, at, 2.);
    let changed = board(250);
    let mut out = Vec::new();
    let awaiting = (cache.get_tiles(node, &changed, at, W, H, 2., false, 0, &mut out)).unwrap();
    assert!(!awaiting, "a change is drawn before it shows");
    assert!(same(
        &composed(&out, 2.),
        &raster(&changed, at, [0, 0], 2.),
        2.
    ));
    // Across the whole board and back, at several zooms.
    for step in 0..40 {
        let k = [1., 0.5, 2., 0.25][step / 10];
        let v = view(-((step % 10) as f32) * 180., -((step % 7) as f32) * 120., k);
        settled(&mut cache, node, &changed, v, 2.);
        let p = &cache.pyramids[&node];
        assert!(p.bytes <= TILE_BUDGET.max(p.shown.len() * 3 * TILE_BYTES));
        assert!(p.levels.len() <= MAX_LEVELS);
    }
    assert!(
        !cache.take_retired().is_empty(),
        "evicted tiles' textures go"
    );
}
/// A gradient under a clip, ellipses and strokes: tiles as its raster.
fn shaded_with(gradient: bool, clip: bool, ellipses: bool, strokes: bool) -> Arc<Canvas> {
    let mut board = Canvas::new();
    let stops = vec![
        GradientStop {
            offset: 0.,
            color: Color(250, 240, 220, 255),
        },
        GradientStop {
            offset: 1.,
            color: Color(40, 60, 120, 255),
        },
    ];
    if gradient {
        board.fill(
            Path::rectangle(Rect::new(0., 0., 400., 300.)),
            Brush::linear(Point::new(0., 0.), Point::new(400., 300.), stops).unwrap(),
        );
    }
    if clip {
        board.clip(
            Path::rounded_rect(Rect::new(30., 20., 300., 200.), 24.).unwrap(),
            FillRule::Winding,
        );
    }
    for i in 0..12 {
        let at = i as f32 * 31.;
        if ellipses {
            board.fill(
                Path::ellipse(Rect::new(at, at * 0.6, 70., 40.)).unwrap(),
                Color(200, (i * 20) as u8, 40, 255),
            );
        }
        if !strokes {
            continue;
        }
        let mut curve = Path::builder();
        curve.move_to(at, 250.).cubic_to(
            Point::new(at + 20., 150.),
            Point::new(at + 50., 290.),
            Point::new(at + 80., 200.),
        );
        board.stroke(
            curve.build().unwrap(),
            Color(20, 20, 20, 255),
            Stroke {
                width: 3.,
                ..Stroke::default()
            },
        );
    }
    if clip {
        board.restore();
    }
    Arc::new(board)
}
fn shaded() -> Arc<Canvas> {
    shaded_with(true, true, true, true)
}
#[test]
fn a_shaded_board_zoomed_shows_its_raster() {
    let canvas = shaded();
    let node = node();
    let mut cache = CanvasCache::default();
    cache
        .get_panned(node, &canvas, view(-10., -6., 1.5), W, H, 1.5, false)
        .unwrap();
    let zoomed = view(-40., -20., 2.);
    cache
        .get_panned(node, &canvas, zoomed, W, H, 1.5, false)
        .unwrap();
    let out = settled(&mut cache, node, &canvas, zoomed, 1.5);
    assert!(same(
        &composed(&out, 1.5),
        &raster(&canvas, zoomed, [0, 0], 1.5),
        1.5
    ));
}
/// Whether `a` and `b`, images `width` pixels across, differ only on
/// edges, each pixel by no more than `b` changes within two pixels of it
/// (what antialiasing rounded the other way can make), and there in less
/// than one pixel in 20.
pub(crate) fn alike(a: &[u8], b: &[u8], width: usize) -> bool {
    let height = a.len() / 4 / width;
    let at = |image: &[u8], x: usize, y: usize, c: usize| image[(y * width + x) * 4 + c];
    let (mut off, mut flat) = (0, 0);
    for y in 0..height {
        for x in 0..width {
            let d = (0..4)
                .map(|c| at(a, x, y, c).abs_diff(at(b, x, y, c)))
                .max()
                .unwrap();
            if d <= 2 {
                continue;
            }
            off += 1;
            let contrast = (y.saturating_sub(2)..(y + 3).min(height))
                .flat_map(|ny| (x.saturating_sub(2)..(x + 3).min(width)).map(move |nx| (nx, ny)))
                .flat_map(|(nx, ny)| (0..4).map(move |c| (nx, ny, c)))
                .map(|(nx, ny, c)| at(b, nx, ny, c).abs_diff(at(b, x, y, c)))
                .max()
                .unwrap();
            // (The raster compared with is cut at the box's edge too.)
            let border = x < 2 || y < 2 || x + 2 >= width || y + 2 >= height;
            if d > contrast && !border {
                flat += 1;
            }
        }
    }
    if flat > 0 || off * 20 > width * height {
        eprintln!("{off} pixels differ, {flat} of them past their edge's contrast");
        return false;
    }
    true
}
#[test]
fn a_promoted_raster_stands_in_for_its_level_until_unshown_a_while() {
    let canvas = board(90);
    let node = node();
    let mut cache = CanvasCache::default();
    let first = view(-10., -10., 1.);
    cache
        .get_panned(node, &canvas, first, W, H, 1., false)
        .unwrap();
    let raster = cache.entries[&node].id;
    let zoomed = view(-300.25, -180.5, 1.7);
    cache
        .get_panned(node, &canvas, zoomed, W, H, 1., false)
        .unwrap();
    assert!(cache.stands_in(node, raster));
    // Its texture is the renderer's already: nothing to upload, nothing cut.
    let mut out = Vec::new();
    (cache.get_tiles(node, &canvas, zoomed, W, H, 1., false, 1, &mut out)).unwrap();
    assert!(
        !out.is_empty()
            && out
                .iter()
                .all(|(r, p)| r.id == raster && r.pixels.is_none() && p.scaled)
    );
    assert_eq!(cache.pyramids[&node].bytes, 0, "no tiles cut on the frame");
    // Its level's own tiles shown, it goes a few frames on.
    let out = settled(&mut cache, node, &canvas, zoomed, 1.);
    assert!(out.iter().all(|(r, _)| r.id != raster));
    for frame in 2..2 + TEXTURE_FRAMES as u64 + 1 {
        (cache.get_tiles(
            node,
            &canvas,
            zoomed,
            W,
            H,
            1.,
            false,
            frame,
            &mut Vec::new(),
        ))
        .unwrap();
    }
    assert!(!cache.stands_in(node, raster));
    assert!(cache.take_retired().contains(&raster), "its texture goes");
}
#[test]
fn a_frame_uploads_what_its_time_allows_and_never_more_than_a_dozen() {
    let mut cache = CanvasCache::default();
    let quick = Duration::from_micros(10);
    for _ in 0..5 {
        cache.tile_uploaded(7, quick);
    }
    assert_eq!(cache.tile_uploads(7), UPLOADS_MOST - 5);
    for _ in 0..20 {
        cache.tile_uploaded(7, quick);
    }
    assert_eq!(cache.tile_uploads(7), 0);
    // The next frame's are its own.
    assert_eq!(cache.tile_uploads(8), UPLOADS_MOST);
    // Slow uploads: what the time left takes.
    cache.tile_uploaded(9, Duration::from_millis(3));
    assert!(cache.tile_uploads(9) <= 2);
}
#[test]
fn a_backdrop_cut_into_tiles_is_its_pixels_and_uploads_ahead_of_a_zoom_out() {
    let canvas = board(160);
    let text = SharedText::default();
    let image = rasterize_region(
        &canvas,
        view(0., 0., 0.3),
        1.,
        [0, 0, 600, 300],
        None,
        &text,
    )
    .unwrap();
    let cut = pieces(&image);
    assert_eq!(cut.len(), 3 * 2);
    for (at, piece) in &cut {
        for y in at[1]..at[3] {
            for x in at[0]..at[2] {
                let p = ((y - at[1]) * TILE + x - at[0]) as usize * 4;
                let got = &piece.pixels()[p..p + 4];
                match x < 600 && y < 300 {
                    true => {
                        let i = (y * 600 + x) as usize * 4;
                        assert_eq!(got, &image.pixels()[i..i + 4]);
                    }
                    false => assert_eq!(got, &[0; 4], "nothing past its edge"),
                }
            }
        }
    }
    // Zoomed: a backdrop is made; once it lands its pieces upload, a few a
    // frame, before a zoom out shows them.
    let node = node();
    let mut cache = CanvasCache::default();
    let first = view(-10., -10., 1.);
    cache
        .get_panned(node, &canvas, first, W, H, 1., false)
        .unwrap();
    let zoomed = view(-300.25, -180.5, 2.);
    cache
        .get_panned(node, &canvas, zoomed, W, H, 1., false)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut frame = 1;
    loop {
        let mut out = Vec::new();
        (cache.get_tiles(node, &canvas, zoomed, W, H, 1., false, frame, &mut out)).unwrap();
        frame += 1;
        let ahead = out
            .iter()
            .filter(|(_, p)| p.rect[2] * p.rect[3] == 0.)
            .count();
        assert!(ahead <= UPLOADS_MOST);
        let pieces = cache.backdrops[&node].pieces.len();
        if pieces > 0 && cache.pyramids[&node].pinned.len() == pieces {
            break;
        }
        assert!(Instant::now() < deadline, "its backdrop lands and uploads");
        std::thread::sleep(Duration::from_millis(5));
    }
    // (The raster it was promoted from long unshown: no stand-in.)
    settled(&mut cache, node, &canvas, zoomed, 1.);
    for _ in 0..=TEXTURE_FRAMES {
        (cache.get_tiles(
            node,
            &canvas,
            zoomed,
            W,
            H,
            1.,
            false,
            frame,
            &mut Vec::new(),
        ))
        .unwrap();
        frame += 1;
    }
    let pinned = cache.pyramids[&node].pinned.clone();
    let mut out = Vec::new();
    (cache.get_tiles(
        node,
        &canvas,
        view(40., 30., 0.3),
        W,
        H,
        1.,
        false,
        frame,
        &mut out,
    ))
    .unwrap();
    let shown: Vec<u64> = out.iter().map(|(r, _)| r.id).collect();
    assert!(
        !shown.is_empty() && shown.iter().all(|id| pinned.contains(id)),
        "the backdrop's pieces, uploaded"
    );
}
