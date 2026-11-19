use std::sync::Arc;
use zgx::{affine::Affine, scene::*, svg::SvgData};
use zgx_gpu::GpuRenderer;
fn data() -> Arc<SvgData> {
    Arc::new(SvgData::new(&br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"><rect width="1" height="1" fill="red"/><rect x="1" width="1" height="1" fill="green"/></svg>"##[..]).unwrap())
}
#[test]
fn svg_resizes_at_device_scale_tints_and_transforms_without_reupload() {
    let mut scene = Scene::new(140., 120.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let source = data();
    let node = scene.append(
        scene.root(),
        NodeKind::Svg(source.clone()),
        Style {
            width: Some(40.),
            height: Some(20.),
            ..Default::default()
        },
    );
    scene.set_transform(node, Transform { x: 40., y: 40. });
    let mut gpu = GpuRenderer::new(140, 120).unwrap();
    let report = scene.flush();
    let stats = gpu.render(&scene, &report.damage).unwrap();
    assert_eq!(stats.svg_rasterizations, 1);
    assert_eq!(stats.image_uploads, 1);
    scene.set_kind(
        node,
        NodeKind::Svg(Arc::new(
            source.transformed(Affine::rotation(std::f32::consts::FRAC_PI_2)),
        )),
    );
    let report = scene.flush();
    assert_eq!(report.layout_nodes, 0);
    let stats = gpu.render(&scene, &report.damage).unwrap();
    assert_eq!(stats.svg_rasterizations, 0);
    assert_eq!(stats.image_uploads, 0);
    let pixels = gpu.readback().unwrap();
    assert_eq!(&pixels[(35 * 140 + 60) * 4..][..3], &[255, 0, 0]);
    let mut fresh = GpuRenderer::new(140, 120).unwrap();
    fresh
        .render(&scene, &[Rect::new(0., 0., 140., 120.)])
        .unwrap();
    assert_eq!(pixels, fresh.readback().unwrap());
    scene.set_kind(
        node,
        NodeKind::Svg(Arc::new(source.tinted(Color(20, 40, 255, 255)))),
    );
    let report = scene.flush();
    assert_eq!(
        gpu.render(&scene, &report.damage)
            .unwrap()
            .svg_rasterizations,
        1
    );
    let pixels = gpu.readback().unwrap();
    assert_eq!(&pixels[(45 * 140 + 50) * 4..][..3], &[20, 40, 255]);
    gpu.set_scale_factor(2.);
    gpu.render(&scene, &[Rect::new(0., 0., 140., 120.)])
        .unwrap();
    // The device-scale raster, and the two 1x ones no node shows kept (in a
    // small budget, see `svg::UNUSED_BUDGET`) for a node that shows them again.
    let kept = 2 * 40 * 20 * 4;
    assert_eq!(gpu.debug_cache_stats().svg_raster_bytes, 80 * 40 * 4 + kept);
    scene.remove(node);
    let report = scene.flush();
    gpu.render(&scene, &report.damage).unwrap();
    assert_eq!(gpu.debug_cache_stats().svg_raster_bytes, 80 * 40 * 4 + kept);
}
#[test]
fn icon_sized_svgs_batch_with_text_instead_of_a_draw_each() {
    // A tool list: icon, label, icon, label... as in a transcript.
    let mut scene = Scene::new(200., 400.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Column));
    for i in 0..16 {
        let row = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Row),
            Style {
                width: Some(200.),
                height: Some(24.),
                ..Default::default()
            },
        );
        scene.append(
            row,
            NodeKind::Svg(Arc::new(data().tinted(Color(200, 200, 200, 255)))),
            Style {
                width: Some(16.),
                height: Some(16.),
                ..Default::default()
            },
        );
        scene.append(
            row,
            NodeKind::Text {
                text: format!("Read file_{i}.rs").into(),
                font_size: 12.,
                color: Color(255, 255, 255, 200),
            },
            Style {
                width: Some(150.),
                height: Some(18.),
                ..Default::default()
            },
        );
    }
    let mut gpu = GpuRenderer::new(200, 400).unwrap();
    let report = scene.flush();
    let stats = gpu.render(&scene, &report.damage).unwrap();
    assert!(
        stats.draw_calls <= 3,
        "{} draws for 16 icons and 16 labels",
        stats.draw_calls
    );
    let pixels = gpu.readback().unwrap();
    // The red half of the first icon.
    assert_eq!(&pixels[(8 * 200 + 3) * 4..][..3], &[200, 200, 200]);
}
#[test]
fn atlas_icons_survive_the_atlas_filling_up() {
    // Large, ever-changing glyphs fill the atlas every few frames, forcing it
    // to grow to its limit and then recycle while an icon lives in it.
    let mut scene = Scene::new(160., 120.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    scene.append(
        scene.root(),
        NodeKind::Svg(Arc::new(data().tinted(Color(240, 180, 40, 255)))),
        Style {
            width: Some(20.),
            height: Some(10.),
            ..Default::default()
        },
    );
    let text = scene.append(
        scene.root(),
        NodeKind::Text {
            text: "".into(),
            font_size: 70.,
            color: Color(255, 255, 255, 255),
        },
        Style {
            width: Some(160.),
            height: Some(100.),
            ..Default::default()
        },
    );
    let mut gpu = GpuRenderer::new(160, 120).unwrap();
    for step in 0..160_u32 {
        let letters: String = (0..8)
            .map(|i| char::from_u32(0x41 + (step * 8 + i) % 58).unwrap())
            .collect();
        scene.set_kind(
            text,
            NodeKind::Text {
                text: letters.into(),
                font_size: 70. + (step % 11) as f32 * 7.,
                color: Color(255, 255, 255, 255),
            },
        );
        let report = scene.flush();
        gpu.render(&scene, &report.damage).unwrap();
        let mut fresh = GpuRenderer::new(160, 120).unwrap();
        fresh
            .render(&scene, &[Rect::new(0., 0., 160., 120.)])
            .unwrap();
        let (got, want) = (gpu.readback().unwrap(), fresh.readback().unwrap());
        // A recycled atlas holds a glyph in another cell than a fresh one
        // does. Glyphs sit at fractional y, so both sample the same raster
        // between rows, and an adapter quantizing bilinear weights (llvmpipe:
        // to 1/256) can round them differently by cell: alpha one level off,
        // premultiplied colour two, along the glyph's edges. A lost or stale
        // icon or glyph differs by far more.
        let off =
            |a: &[u8], b: &[u8]| (0..3).any(|i| a[i].abs_diff(b[i]) > 2) || a[3].abs_diff(b[3]) > 1;
        let count = got
            .chunks(4)
            .zip(want.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        if count > 256 || got.chunks(4).zip(want.chunks(4)).any(|(a, b)| off(a, b)) {
            let diffs: Vec<_> = got
                .chunks(4)
                .zip(want.chunks(4))
                .enumerate()
                .filter(|(_, (a, b))| off(a, b))
                .map(|(i, (a, b))| (i % 160, i / 160, a.to_vec(), b.to_vec()))
                .take(5)
                .collect();
            panic!("step {step}: {count} pixels differ, e.g. {diffs:?}");
        }
    }
}
#[test]
fn atlas_icons_survive_a_change_of_scale_and_back() {
    // A window zoomed in and back (or moved to another display and back):
    // its text, then an icon, in the glyph atlas. The other scale's glyphs
    // take the atlas from its start, over where the icon was; back at the
    // first scale the icon's raster is the one it had, and must be drawn
    // from cells holding it, not those glyphs.
    let mut scene = Scene::new(160., 120.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Column));
    scene.append(
        scene.root(),
        NodeKind::Text {
            text: "Agents and tabs".into(),
            font_size: 24.,
            color: Color(255, 255, 255, 255),
        },
        Style {
            width: Some(160.),
            height: Some(40.),
            ..Default::default()
        },
    );
    scene.append(
        scene.root(),
        NodeKind::Svg(Arc::new(data().tinted(Color(240, 180, 40, 255)))),
        Style {
            width: Some(20.),
            height: Some(10.),
            ..Default::default()
        },
    );
    let whole = [Rect::new(0., 0., 160., 120.)];
    let mut gpu = GpuRenderer::new(200, 150).unwrap();
    for scale in [1., 1.25, 1.] {
        gpu.set_scale_factor(scale);
        scene.flush();
        gpu.render(&scene, &whole).unwrap();
    }
    let mut fresh = GpuRenderer::new(200, 150).unwrap();
    fresh.render(&scene, &whole).unwrap();
    let (got, want) = (gpu.readback().unwrap(), fresh.readback().unwrap());
    // The icon's rows (40 to 50), whole.
    for y in 40..50 {
        let row = |pixels: &[u8]| pixels[y * 200 * 4..][..20 * 4].to_vec();
        assert_eq!(row(&got), row(&want), "row {y}");
    }
}

/// The pixels a 160×60 window at `scale` shows of `mark`, in a padded row at
/// a fraction of a point, with the artwork `id`ed `"slot"` drawn in that box
/// by hand at its size (as an icon was once its box had one) if `by_hand`.
fn icon_pixels(scale: f32, mark: zgx::compose::View, by_hand: Option<Arc<SvgData>>) -> Vec<u8> {
    use zgx::{compose::prelude::*, widgets::Ui};
    let (w, h) = (160., 60.);
    let mut gpu = GpuRenderer::new((w * scale) as u32, (h * scale) as u32).unwrap();
    gpu.set_scale_factor(scale);
    let mut ui = Ui::new(w, h);
    let view = ui.mount(row().pt(7.25).pl(13.5).child(mark));
    ui.prepare_frame();
    if let Some(artwork) = by_hand {
        let slot = view.find("slot").unwrap();
        let size = ui.scene.borrow().layout_bounds(slot);
        ui.scene.borrow_mut().append(
            slot,
            zgx::scene::NodeKind::Svg(artwork),
            zgx::scene::Style {
                absolute: true,
                width: Some(size.width),
                height: Some(size.height),
                ..Default::default()
            },
        );
        ui.prepare_frame();
    }
    ui.scene.borrow_mut().flush();
    gpu.render(&ui.scene.borrow(), &[zgx::scene::Rect::new(0., 0., w, h)])
        .unwrap();
    gpu.readback().unwrap()
}

/// An icon draws as its artwork drawn in its box at its size: a square, a
/// non-square one, and a square one kept square in a wide box, at 1x and 2x.
#[test]
fn an_icon_draws_as_its_artwork_at_its_boxs_size() {
    use zgx::compose::prelude::*;
    let artwork = |w: u32, h: u32| {
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}"><rect width="{w}" height="{h}" fill="#2050e0"/><circle cx="{}" cy="{}" r="{}" fill="#e04020"/></svg>"##,
            w as f32 * 0.3,
            h as f32 * 0.6,
            w.min(h) as f32 * 0.3,
        );
        Arc::new(SvgData::new(svg.into_bytes()).unwrap())
    };
    let cases = [
        ("square", artwork(16, 16), (24., 24.)),
        ("non-square", artwork(32, 16), (40., 20.)),
        ("square in a wide box", artwork(16, 16), (120., 24.)),
    ];
    for scale in [1., 2.] {
        for (name, art, (width, height)) in &cases {
            let icon = icon_pixels(scale, svg("Icon", art.clone()).size(*width, *height), None);
            let by_hand = icon_pixels(
                scale,
                div().id("slot").size(*width, *height),
                Some(art.clone()),
            );
            let inked = icon
                .chunks(4)
                .filter(|p| p[3] > 0 && (p[0] > 100 || p[2] > 100))
                .count();
            assert!(inked > 0, "{name} at {scale}x draws");
            assert!(
                icon == by_hand,
                "{name} at {scale}x draws as its artwork at its box's size"
            );
        }
    }
}
