//! Many renders in a process hold no more than a few: each stage's thread
//! and pixels go with it. A process of its own, as it reads the process's
//! memory, which other tests freeing theirs would hide.
mod common;
use common::*;
use zgx_video::{RenderOptions, render};

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
fn renders_hold_no_more_than_a_few() {
    let dir = temp_dir("memory");
    std::fs::write(dir.join("video.luau"), MOVES).unwrap();
    let render_all = |count: usize| {
        for i in 0..count {
            let out = RenderOptions {
                script: dir.to_path_buf(),
                output: dir.join(format!("m{i}")),
                frames: Some((0, 1)),
                ..Default::default()
            };
            render(&out, |_| true).unwrap();
        }
    };
    // What a first few renders fill grows for a while after (on Windows,
    // working sets read on a GPU rose for some 50 renders, and then wandered
    // within a few MiB): the bound counts from after that.
    render_all(52);
    let settled = resident();
    render_all(48);
    let grown = resident().saturating_sub(settled);
    assert!(grown < 24 << 20, "48 more renders kept {} MiB", grown >> 20);
}

/// The process's resident memory, in bytes: a value it read, or the test
/// fails, since a reading of nothing would pass any bound.
#[cfg(unix)]
fn resident() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    let kilobytes = String::from_utf8_lossy(&out.stdout);
    let kilobytes: u64 = kilobytes
        .trim()
        .parse()
        .expect("ps says the process's size");
    assert!(kilobytes > 0, "ps says the process holds nothing");
    kilobytes * 1024
}
/// The process's working set, in bytes (`resident` elsewhere).
#[cfg(windows)]
fn resident() -> u64 {
    use std::ffi::c_void;
    /// PROCESS_MEMORY_COUNTERS in psapi.h.
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        size: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        rest: [usize; 6],
    }
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn K32GetProcessMemoryInfo(process: *mut c_void, counters: *mut Counters, size: u32)
        -> i32;
    }
    let mut counters = Counters {
        size: size_of::<Counters>() as u32,
        ..Counters::default()
    };
    // SAFETY: the current process's pseudo-handle, and counters of the size
    // given.
    let read =
        unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.size) };
    assert_ne!(read, 0, "the process's size can't be read");
    assert!(counters.working_set > 0, "the process holds nothing");
    counters.working_set as u64
}
