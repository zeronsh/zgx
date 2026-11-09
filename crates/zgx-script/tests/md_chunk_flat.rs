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
const STILL_GROWING: &[(Phase, &str)] = &[];

#[test]
fn a_chunk_costs_the_same_however_long_the_reply() {
    // As it starts: plainly, and with what is or looks like a link definition
    // near its top, which once made every later chunk parse the whole reply.
    for start in [
        "",
        "[x]: http://y\n\nWords.\n\n",
        "- [x]: y\n\nWords.\n\n",
        "foo]: y\n\nWords.\n\n",
        "```\n[x]: y\n```\n\nWords.\n\n",
    ] {
        md_chunk::check(start, STILL_GROWING, false, || {
            |_: &zgx::scene::Scene, _: &[zgx::scene::Rect]| {}
        });
    }
}
