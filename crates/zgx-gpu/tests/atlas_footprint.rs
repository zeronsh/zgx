//! A page of text read through grows the glyph atlas; at rest, as a
//! desktop host rests a window (its trims, the atlas's and Metal's queue's),
//! the heap comes back to near where it was. Core Text kept the glyphs of
//! every font it drew with for as long as zgx kept the font: some 30 MB of
//! heap after CJK text at three sizes, for good, and 40 MB of footprint.
//! And what Core Text handed back autoreleased as each new character's
//! face was looked up stayed where no event loop drains a pool, as here:
//! 9 MB more.
//! The heap in use (every malloc zone: Core Text's are the system's) rather
//! than the footprint, which also counts Metal's driver blocks, given back
//! some time after its queue rests. A binary of its own: the heap is the
//! process's.
#![cfg(target_os = "macos")]
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

unsafe extern "C" {
    fn malloc_zone_statistics(zone: *mut std::ffi::c_void, stats: *mut [usize; 4]);
}

/// The bytes malloc's zones hold in use (`malloc_statistics_t`'s
/// `size_in_use`, every zone), in MB.
fn heap() -> f64 {
    let mut stats = [0usize; 4];
    // SAFETY: every zone (null), a `malloc_statistics_t` of four words.
    unsafe { malloc_zone_statistics(std::ptr::null_mut(), &mut stats) };
    stats[1] as f64 / 1048576.
}

/// What a desktop host does as its window goes to rest (zgx-desktop's
/// `application.rs`): the renderer's and the scene's trims, then the
/// atlas's trim and Metal's queue rested.
fn rest(scene: &mut Scene, gpu: &mut GpuRenderer) {
    gpu.trim();
    scene.rest();
    gpu.trim_atlas();
    gpu.rest_queue();
}

const PAGE: &str = "The quick brown fox jumps over the lazy dog. 0123456789 \
你好世界我们的生活中有很多美好的东西学习工作朋友家人时间快乐幸福健康天气今天明天昨天";

/// Sets each of `texts` to what `text` gives for its place, and draws.
fn show(
    scene: &mut Scene,
    gpu: &mut GpuRenderer,
    texts: &[NodeId],
    text: impl Fn(usize) -> (String, f32),
) {
    for (i, &node) in texts.iter().enumerate() {
        let (text, font_size) = text(i);
        let color = Color(20, 20, 20, 255);
        scene.set_kind(
            node,
            NodeKind::Text {
                text: text.into(),
                font_size,
                color,
            },
        );
    }
    let damage = scene.flush().damage;
    gpu.render(scene, &damage).unwrap();
    gpu.readback().unwrap();
}

#[test]
fn a_page_read_through_gives_its_memory_back_at_rest() {
    let (width, height) = (1200., 800.);
    let mut gpu = GpuRenderer::new(2400, 1600).expect("a GPU adapter");
    gpu.set_scale_factor(2.);
    let mut scene = Scene::new(width, height);
    gpu.install_text(&mut scene);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Column));
    let texts: Vec<_> = (0..6)
        .map(|_| {
            let kind = NodeKind::Text {
                text: PAGE.into(),
                font_size: 14.,
                color: Color(20, 20, 20, 255),
            };
            let style = Style {
                width: Some(width),
                ..Default::default()
            };
            scene.append(scene.root(), kind, style)
        })
        .collect();
    let page = |_| (PAGE.to_string(), 14.);
    show(&mut scene, &mut gpu, &texts, page);
    rest(&mut scene, &mut gpu);
    let start = heap();
    // New characters each frame at 14, 18 and 24 points (a long document
    // read through): the atlas grows to hold them.
    for step in 0..16 {
        show(&mut scene, &mut gpu, &texts, |i| {
            let first = 0x4E00 + (step * 6 + i) as u32 * 160;
            let text = (first..first + 160).filter_map(char::from_u32).collect();
            (text, [14., 18., 24.][i % 3])
        });
    }
    assert!(gpu.atlas_grown(), "the page grew the atlas");
    show(&mut scene, &mut gpu, &texts, page);
    let grown = heap();
    rest(&mut scene, &mut gpu);
    let rest = heap();
    // 1.5 MB above where it was (of 28 grown); with the lookups' autoreleased
    // objects kept, 11 MB, and with Core Text's fonts kept too, 30 MB.
    assert!(
        rest < start + 4.,
        "at rest: {rest:.1} MB, from {start:.1} (grown {grown:.1})"
    );
}
