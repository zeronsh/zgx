//! What binaural effects hold: the spherical head made once a rate and
//! shared, the responses read where they lie, and all of it given back
//! when the last one goes. Its own binary: the allocator below counts
//! every byte live in the process, and one test runs at a time.
#![cfg(feature = "hrtf")]
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use zgx_audio::effects::{Binaural, HrtfSet};
use zgx_audio::{Effect, Format};

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
fn spherical_heads_at_one_rate_share_one_set_and_give_it_back() {
    for rate in [48_000u32, 96_000] {
        let before = live();
        // One set alone, for its size.
        let one = live();
        let set = HrtfSet::spherical_head(rate);
        let size = live() - one;
        drop(set);
        let mut heads: Vec<Binaural> = (0..16).map(|_| Binaural::spherical()).collect();
        for head in &mut heads {
            head.prepare(Format { rate, channels: 2 }, 512);
        }
        let held = live() - before;
        // One set, and each one's own filters and lines: far less than a
        // set each, and less than two in all.
        assert!(
            held < size * 2,
            "{rate} Hz: {held} bytes held for 16, a set is {size}"
        );
        drop(heads);
        let after = live() - before;
        assert!(
            after < 4_096,
            "{rate} Hz: {after} bytes left after all were dropped"
        );
    }
}
