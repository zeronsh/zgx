//! A render leaves nothing behind: the bytes the program holds come back to
//! where they were once it returns, render after render: a leak far too slow
//! for a process's working set to show. A process of its own: it counts every
//! allocation the process makes.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};
mod common;
use common::*;
use zgx_video::{RenderOptions, render};

/// The bytes held, of all the process's threads.
static HELD: AtomicIsize = AtomicIsize::new(0);
struct Counting;
// SAFETY: the system's allocator, with a count beside it.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        HELD.fetch_add(layout.size() as isize, Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        HELD.fetch_add(layout.size() as isize, Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        HELD.fetch_sub(layout.size() as isize, Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        HELD.fetch_add(size as isize - layout.size() as isize, Relaxed);
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

const MOVES: &str = r##"
local video = require("@video")
return video.compositions { video.composition {
    id = "M", w = 64, h = 48, fps = 30, duration = 20,
    render = function()
        local f = video.frame()
        return ui.div { w_full = true, h_full = true,
            ui.div { w = 20, h = 20, rounded = 4, bg = "#ff0000", x = f:map({ 0, 19 }, { 0, 40 }), y = f:spring { to = 20 } },
        }
    end,
} }
"##;

#[test]
fn a_render_leaves_nothing_behind() {
    let dir = temp_dir("leaves");
    std::fs::write(dir.join("video.luau"), MOVES).unwrap();
    let render_one = |i: usize| {
        let out = RenderOptions {
            script: dir.to_path_buf(),
            output: dir.join(format!("m{i}")),
            frames: Some((0, 1)),
            ..Default::default()
        };
        render(&out, |_| true).unwrap();
    };
    // The first few fill what lasts the process (fonts, compiled scripts
    // and shaders); after them, a render returns holding what it found.
    for i in 0..8 {
        render_one(i);
    }
    let before = HELD.load(Relaxed);
    for i in 8..40 {
        render_one(i);
    }
    let left = HELD.load(Relaxed) - before;
    assert!(
        left < 4096,
        "32 renders left {left} bytes, {} a render",
        left / 32
    );
}
