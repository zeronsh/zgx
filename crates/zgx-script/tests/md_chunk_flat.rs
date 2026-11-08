//! One chunk of a streamed Markdown reply costs what a chunk costs, however
//! long the reply has grown: each phase's instructions for a chunk at 100,
//! 200 and 400 sections, side by side (a `ui.markdown { key }` stream drawn
//! by `ui.each`, as `md_stream_bench` and the docs do it). Its own process:
//! it counts with `ZGX_PROFILE` (`zgx::profile::take`).
#[path = "md_chunk/mod.rs"]
mod md_chunk;
use zgx::profile::Phase;

/// Phases a chunk still costs more in as the reply grows, and whose: each
/// goes from here once it is flat (the test then fails until it does), and
/// any phase not here growing fails it.
const STILL_GROWING: &[(Phase, &str)] = &[
    // The parse's whole block list cloned and its table derived (flush lane).
    (Phase::Script, "flush"),
    // ui.each's and keyed's prefix scans, the rows' identities (flush lane);
    // damage marked across every block (render lane).
    (Phase::Update, "flush, render"),
    // The column measured and arranged whole as its last block grows
    // (layout lane).
    (Phase::Layout, "layout"),
];

#[test]
fn a_chunk_costs_the_same_however_long_the_reply() {
    md_chunk::check(STILL_GROWING, false, || {
        |_: &zgx::scene::Scene, _: &[zgx::scene::Rect]| {}
    });
}
