//! A reply streamed into a long text (`state:append`, `Shared::append`)
//! holds, once its host goes idle, what the same text set whole does: the
//! edits a long text keeps for its readers go as the stream stops, and all
//! of it as the text is replaced. A test of its own: it counts the heap.
#[path = "common/mod.rs"]
mod common;

use common::Harness;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};
use zgx_script::Value;

static HELD: AtomicIsize = AtomicIsize::new(0);
struct Counting;
// SAFETY: `System`'s, counting the bytes held.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        HELD.fetch_add(layout.size() as isize, Relaxed);
        unsafe { System.alloc(layout) }
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

const APP: &str =
    r#"return ui.column { w = 380, h = 600, ui.markdown_view { "{reply}", w_full = true } }"#;

/// What the heap holds, the host idle.
fn held(h: &Harness) -> isize {
    h.frames(1);
    zgx::idle::trim(&h.ui.runtime);
    HELD.load(Relaxed)
}

/// The bytes `value` alone holds: what goes with it.
fn weight(value: Value) -> isize {
    let before = HELD.load(Relaxed);
    drop(value);
    before - HELD.load(Relaxed)
}

#[test]
fn a_streamed_reply_holds_what_one_set_whole_does_once_idle() {
    let mut prefix = String::new();
    for i in 0..2_000 {
        prefix.push_str(&format!("Paragraph {i} of what came before.\n\n"));
    }
    let mut reply = None;
    let h = Harness::new(|runtime| {
        let host = zgx_script::ScriptHost::inline(runtime, "app", APP);
        reply = Some(host.shared("reply", Value::from("")));
        host
    });
    let reply = reply.unwrap();
    reply.set(Value::Text(zgx::text::Text::new(&prefix)));
    h.frames(1);
    let mut whole = prefix;
    for i in 0..200 {
        let token = format!(" token{i}");
        reply.append(&token).unwrap();
        whole.push_str(&token);
        h.frames(1);
    }
    // Streaming, the text keeps its last edits for its readers.
    let streaming = reply.with_untracked(Clone::clone);
    let Value::Text(text) = &streaming else {
        panic!("a long text")
    };
    let id = text.id();
    drop(streaming);
    held(&h);
    // Idle: the same text (its readers go on from it), its edits gone.
    let idle = reply.with_untracked(Clone::clone);
    let Value::Text(text) = &idle else {
        panic!("a long text")
    };
    assert_eq!(text.id(), id, "the same text");
    assert_eq!(text.inserts_since(id).map(Iterator::count), Some(0));
    assert_eq!(text.to_cow(), whole);
    // It holds what the same text made whole does, and goes, all of it,
    // as the reply is replaced.
    reply.set(Value::from("done"));
    held(&h);
    let streamed = weight(idle);
    let fresh = weight(Value::Text(zgx::text::Text::new(&whole)));
    assert!(
        streamed <= fresh,
        "streamed {streamed} B, made whole {fresh} B"
    );
    assert!(
        streamed > whole.len() as isize,
        "released whole: {streamed} B"
    );
}
