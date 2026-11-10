//! A fade's frame recolours the newest runs of a paragraph, its text laid
//! out as it was: the frame damages those runs' glyphs, not the paragraph's
//! lines, and that damage still covers every pixel the recolour changes,
//! accents stacked above the line, descenders below it and an underline
//! included.
use std::sync::Arc;
use zgx::{
    rich_text::{Decoration, RichText, TextRun},
    scene::*,
};
use zgx_gpu::GpuRenderer;

const W: u32 = 360;
const H: u32 = 220;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Wrapping words, the run that fades (accented capitals, stacked marks,
/// descenders, underlined) on the third line or so, more words after it.
const TEXT: &str = "Plain words that came first, wrapping over a line or two of this \
     paragraph before the newest ones, ÅÉÎÕÜ Ŷ̃ǺṎ gypsy jig quay, and plain words after.";

fn paragraph(alpha: u8) -> NodeKind {
    let start = TEXT.find("ÅÉ").unwrap();
    let end = TEXT.find(", and plain").unwrap();
    let run =
        |range: std::ops::Range<usize>, color: Color, underline: Option<Decoration>| TextRun {
            range,
            font_size: 22.,
            color,
            underline,
            ..TextRun::default()
        };
    let white = Color(255, 255, 255, 255);
    NodeKind::RichText {
        text: Arc::new(
            RichText::new(
                TEXT,
                vec![
                    run(0..start, white, None),
                    run(
                        start..end,
                        Color(255, 230, 120, alpha),
                        Some(Decoration::new(2.)),
                    ),
                    run(end..TEXT.len(), white, None),
                ],
            )
            .unwrap(),
        ),
    }
}

fn area(rects: &[Rect]) -> f32 {
    rects.iter().map(|r| r.width * r.height).sum()
}

#[test]
fn a_recoloured_runs_damage_covers_its_glyphs_ink() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(W, H).expect("a Vulkan/Metal adapter is required");
    let mut scene = Scene::new(W as f32, H as f32);
    gpu.install_text(&mut scene);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    scene.append(
        root,
        NodeKind::Rect(Color(20, 24, 30, 255)),
        Style {
            width: Some(W as f32),
            height: Some(H as f32),
            ..Style::default()
        },
    );
    let text = scene.append(
        root,
        paragraph(30),
        Style {
            width: Some(330.),
            text_wrap: true,
            ..Style::default()
        },
    );
    scene.set_transform(text, Transform { x: 14., y: 12. });
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    let before = gpu.readback().unwrap();
    let lines = scene.bounds(text);
    assert!(lines.height > 3. * 22., "the paragraph wraps: {lines:?}");
    let viewport = Rect::new(0., 0., W as f32, H as f32);
    for alpha in [90, 170, 255] {
        let was = gpu.readback().unwrap();
        scene.set_leaf_kind(text, paragraph(alpha));
        let report = scene.flush();
        assert_eq!(report.layout_nodes, 0, "a recolour lays nothing out");
        let damage = report.damage;
        assert!(!damage.is_empty());
        // Its runs' glyphs, not the paragraph's lines.
        assert!(
            area(&damage) < 0.6 * lines.width * lines.height,
            "damage {damage:?} against the paragraph's {lines:?}"
        );
        gpu.render(&scene, &damage).unwrap();
        let partial = gpu.readback().unwrap();
        gpu.render(&scene, &[viewport]).unwrap();
        let fresh = gpu.readback().unwrap();
        // Every pixel the recolour changed is damaged...
        let mut changed = 0;
        for (index, (a, b)) in was.chunks(4).zip(fresh.chunks(4)).enumerate() {
            if a != b {
                changed += 1;
                let (x, y) = (
                    (index as u32 % W) as f32 + 0.5,
                    (index as u32 / W) as f32 + 0.5,
                );
                assert!(
                    damage
                        .iter()
                        .any(|r| r.x <= x && x <= r.x + r.width && r.y <= y && y <= r.y + r.height),
                    "pixel ({x}, {y}) changed outside the damage {damage:?}"
                );
            }
        }
        assert!(changed > 200, "the run changes colour: {changed} pixels");
        // ...so the frame drawn from it is the frame drawn whole.
        assert!(
            partial == fresh,
            "alpha {alpha}: the damaged frame differs from a full one"
        );
    }
    // The run's accents, descenders and underline changed with it: pixels
    // above its capitals' tops and below its baseline.
    let after = gpu.readback().unwrap();
    let rows: Vec<u32> = (0..H)
        .filter(|y| {
            (0..W).any(|x| {
                let at = ((y * W + x) * 4) as usize;
                before[at..at + 4] != after[at..at + 4]
            })
        })
        .collect();
    assert!(rows.len() > 22, "rows changed: {rows:?}");
}

/// A recolour and a change of where its text paints in one frame: the
/// whole text is damaged, as any change but a recolour damages it.
#[test]
fn a_recolour_with_a_wash_damages_the_whole_text() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(W, H).expect("a Vulkan/Metal adapter is required");
    let mut scene = Scene::new(W as f32, H as f32);
    gpu.install_text(&mut scene);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    let text = scene.append(
        root,
        paragraph(30),
        Style {
            width: Some(330.),
            text_wrap: true,
            ..Style::default()
        },
    );
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    let lines = scene.bounds(text);
    scene.set_leaf_kind(text, paragraph(120));
    let NodeKind::RichText { text: rich } = paragraph(120) else {
        unreachable!()
    };
    let mut runs = rich.runs().to_vec();
    runs[0].background = Some(Color(60, 0, 0, 255));
    scene.set_leaf_kind(
        text,
        NodeKind::RichText {
            text: Arc::new(RichText::new(TEXT, runs).unwrap()),
        },
    );
    let damage = scene.flush().damage;
    assert!(
        area(&damage) >= 0.8 * lines.width * lines.height,
        "damage {damage:?} against the paragraph's {lines:?}"
    );
    gpu.render(&scene, &damage).unwrap();
    let partial = gpu.readback().unwrap();
    gpu.render(&scene, &[Rect::new(0., 0., W as f32, H as f32)])
        .unwrap();
    assert!(partial == gpu.readback().unwrap());
}
