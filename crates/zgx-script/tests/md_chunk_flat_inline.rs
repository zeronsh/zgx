//! `md_chunk_flat` with inline boxes in the reply: its code runs drawn as
//! chips, elements in their paragraphs' lines. A chunk still costs what a
//! chunk costs, however long the reply has grown. Its own process, as
//! `md_chunk_flat`.
#[path = "md_chunk/mod.rs"]
mod md_chunk;
use zgx::profile::Phase;

/// As `md_chunk_flat`'s: no phase grows here that doesn't grow there.
const STILL_GROWING: &[(Phase, &str)] = &[];

#[test]
fn a_chunk_with_chips_costs_the_same_however_long_the_reply() {
    md_chunk::CHIPS.store(true, std::sync::atomic::Ordering::Relaxed);
    md_chunk::check("", STILL_GROWING, false, || {
        |_: &zgx::scene::Scene, _: &[zgx::scene::Rect]| {}
    });
}
