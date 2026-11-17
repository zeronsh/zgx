//! A scrolled frame gathers the observed bounds it moved into one list,
//! with room for them all taken once: however many observed rows move, the
//! frame allocates as often. (Its own process: a counting allocator.)
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::{Cell, RefCell},
    rc::Rc,
};
use zgx::{compose::prelude::*, reactive::Signal, scene::Rect, widgets::Ui};

thread_local! {
    // Const, destructor-free TLS keeps allocator bookkeeping allocation-free.
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static REQUESTS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAllocator;

fn record_request() {
    if COUNTING.try_with(Cell::get).unwrap_or(false) {
        let _ = REQUESTS.try_with(|count| count.set(count.get() + 1));
    }
}

// SAFETY: Every allocation operation delegates unchanged to System. TLS only
// records calls on the measured test thread and never allocates or owns memory.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_request();
        // SAFETY: The caller supplies the GlobalAlloc layout contract.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_request();
        // SAFETY: The caller supplies the GlobalAlloc layout contract.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_request();
        // SAFETY: The original allocation and new size are forwarded unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The original allocation and layout are forwarded unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn allocation_requests(f: impl FnOnce()) -> usize {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            COUNTING.with(|enabled| enabled.set(false));
        }
    }
    REQUESTS.with(|count| count.set(0));
    COUNTING.with(|enabled| enabled.set(true));
    let reset = Reset;
    f();
    drop(reset);
    REQUESTS.with(Cell::get)
}

/// A scroll view of `rows` observed rows, scrolled a few frames: the
/// allocations of the last, with every row moved, and the first row's
/// bounds as published.
fn scrolled_frame(rows: usize) -> (usize, Rect) {
    let mut ui = Ui::new(300., 400.);
    let offset = ui.signal(0_f32);
    let observed: Rc<RefCell<Vec<Signal<Rect>>>> = Rc::default();
    let mut list = column();
    for _ in 0..rows {
        let keep = observed.clone();
        list = list.child(div().w(100.).h(4.).on_mount(move |handle| {
            keep.borrow_mut().extend(handle.observe_bounds());
        }));
    }
    ui.mount(scroll(offset.clone()).w(300.).h(200.).child(list));
    ui.prepare_frame();
    assert_eq!(observed.borrow().len(), rows);
    for at in [3., 6., 9.] {
        offset.set(at);
        ui.prepare_frame();
    }
    offset.set(12.);
    let requests = allocation_requests(|| ui.prepare_frame());
    let first = observed.borrow()[0].get();
    (requests, first)
}

#[test]
fn a_scrolled_frame_allocates_as_often_however_many_observed_rows_move() {
    let (few, first) = scrolled_frame(64);
    assert_eq!(first.y, -12., "published as before");
    let (many, first) = scrolled_frame(512);
    assert_eq!(first.y, -12., "published as before");
    assert_eq!(
        few, many,
        "64 observed rows moved: {few} allocations; 512: {many}"
    );
}
