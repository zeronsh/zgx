//! The direct Metal path's first-frame libraries (draw.metal, copy_offset.metal)
//! are compiled when zgx is built, where Xcode's Metal toolchain is, and
//! loaded rather than compiled at start: a launch whose shader cache misses
//! drew its first frame about 150 ms later for compiling them. What they draw
//! is what the same sources compiled at start draw, byte for byte, and a
//! library built from a source edited since is never loaded.
#![cfg(target_os = "macos")]
use zgx::{compose::prelude::*, widgets::Ui};
use zgx_gpu::GpuRenderer;

const W: f32 = 240.;
const H: f32 = 180.;

/// Rounded and bordered boxes, a see-through one over them, a shadow and
/// text: both of draw.metal's pipelines (`vs`/`fs`, `vs_basic`/`fs_basic`).
fn drawn(scale: f32) -> Vec<u8> {
    let mut gpu = GpuRenderer::new((W * scale) as u32, (H * scale) as u32).unwrap();
    gpu.set_scale_factor(scale);
    let mut ui = Ui::new(W, H);
    ui.mount(
        overlay().w(W).h(H).bg(rgb(0x101820)).child(
            column()
                .w(W)
                .h(H)
                .p(12.)
                .gap(8.)
                .child(
                    div()
                        .w(200.)
                        .h(40.)
                        .rounded(10.)
                        .border(2.)
                        .border_color(rgb(0xd0a040))
                        .bg(rgb(0x3a6a9a))
                        .shadow(zgx::scene::BoxShadow {
                            color: rgba(0x000000a0),
                            offset: zgx::scene::Transform { x: 0., y: 4. },
                            blur_radius: 12.,
                            spread: 0.,
                        }),
                )
                .child(div().w(200.).h(40.).rounded(20.).bg(rgb(0x9a3a6a)))
                .child(
                    text("Built ahead, drawn the same")
                        .text_size(14.)
                        .text_color(rgb(0xffffff)),
                )
                .child(
                    div()
                        .absolute()
                        .left(60.)
                        .top(30.)
                        .w(100.)
                        .h(80.)
                        .rounded(6.)
                        .bg(rgba(0xffffff60)),
                ),
        ),
    );
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    // The first frame uploads its glyphs, which wgpu draws; the next, all
    // drawn again, goes direct.
    gpu.render(&ui.scene.borrow(), &damage).unwrap();
    gpu.render(&ui.scene.borrow(), &[zgx::scene::Rect::new(0., 0., W, H)])
        .unwrap();
    gpu.debug_present_offscreen().unwrap()
}

/// One test drawing both ways: the counters and the switch are the process's.
#[test]
fn frames_from_the_libraries_built_ahead_match_those_compiled_at_start() {
    for scale in [1., 2.] {
        zgx_gpu::debug_compile_metal_libraries_at_start(false);
        let (loaded, compiled) = zgx_gpu::debug_metal_library_counts();
        let ahead = drawn(scale);
        let after = zgx_gpu::debug_metal_library_counts();
        let built = zgx_gpu::debug_built_metal_libraries().is_some();
        // Drawn on the direct path, its two libraries made for it: loaded
        // where they were built (this toolchain), else compiled.
        if built {
            assert_eq!(after, (loaded + 2, compiled), "loaded at {scale}x");
        } else {
            assert_eq!(after, (loaded, compiled + 2), "compiled at {scale}x");
        }
        zgx_gpu::debug_compile_metal_libraries_at_start(true);
        let at_start = drawn(scale);
        zgx_gpu::debug_compile_metal_libraries_at_start(false);
        assert_eq!(
            zgx_gpu::debug_metal_library_counts(),
            (after.0, after.1 + 2),
            "compiled at start at {scale}x"
        );
        assert!(
            ahead == at_start,
            "the libraries built ahead draw otherwise at {scale}x: first difference at byte {:?}",
            ahead.iter().zip(&at_start).position(|(a, b)| a != b)
        );
    }
}

/// Built from the sources compiled in: build.rs hashes each source as it
/// compiles it, and one edited since (a rebuild that couldn't compile it)
/// would be compiled at start rather than run stale.
#[test]
fn the_libraries_built_ahead_are_of_the_sources_compiled_in() {
    if let Some(fresh) = zgx_gpu::debug_built_metal_libraries() {
        assert_eq!(fresh, [true, true], "draw.metal, copy_offset.metal");
    }
}
