//! `md_chunk_flat`, its reply followed to its end in a scroll view (a chat
//! following its stream) and each chunk drawn as a window draws it: only its
//! damage, by a renderer of its own (`GpuRenderer::render`), so painting,
//! text and encoding are counted too. Its own process: it counts with
//! `ZGX_PROFILE` (`zgx::profile::take`).
#[path = "md_chunk/mod.rs"]
mod md_chunk;
use zgx::profile::Phase;

/// As `md_chunk_flat`'s, with the phases drawing adds.
const STILL_GROWING: &[(Phase, &str)] = &[
    (Phase::Script, "flush"),
    (Phase::Update, "flush, render"),
    (Phase::Layout, "layout"),
    // The paint walk over every block of the column, shown or not, to find
    // the few a chunk's damage reaches (render lane).
    (Phase::Paint, "render"),
];

#[test]
fn a_drawn_chunk_costs_the_same_however_long_the_reply() {
    md_chunk::check(STILL_GROWING, true, || {
        let mut gpu = zgx_gpu::GpuRenderer::new(640, 900).expect("a GPU");
        move |scene: &zgx::scene::Scene, damage: &[zgx::scene::Rect]| {
            gpu.render(scene, damage).unwrap();
        }
    });
}
