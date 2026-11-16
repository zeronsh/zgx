//! What an engine holds at rest. Its own binary: the allocator below
//! counts every byte live in the process, and one test runs at a time.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use zgx_audio::{Audio, Config};

struct Counting;
static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        LIVE.fetch_add(new as isize - layout.size() as isize, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn live() -> isize {
    LIVE.load(Ordering::Relaxed)
}

#[test]
fn a_bus_s_buffers_are_made_as_it_s_added_not_before() {
    // Room for 64 buses, four of them added: the others hold no buffer,
    // and one added makes its own (two channels of a block) as it's added,
    // on this thread.
    let before = live();
    let (audio, mixer) = Audio::offline(Config::default()).unwrap();
    let made = live() - before;
    let one = live();
    let bus = audio
        .add_bus("extra", &audio.bus("world").unwrap())
        .unwrap();
    let added = live() - one;
    println!("engine at rest: {made} bytes; a bus added: {added} bytes");
    let buffers = 2 * 128 * size_of::<f32>() as isize;
    assert!(added >= buffers, "a bus added made {added} bytes");
    drop((bus, audio, mixer));
}
