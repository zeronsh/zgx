//! A Luau list built in the browser keeps no more than it shows: the nodes
//! each row's build makes are garbage once lowered, their bytes in Rust
//! behind userdata Luau's collector sees as a pointer, and they step it as
//! allocating them would (`zgx_script::node`). Without that, a build after
//! a collection let them pile up to a peak wasm memory keeps for good. A
//! binary of its own: it counts every allocation of the page.
#![cfg(target_arch = "wasm32")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use zgx::reactive::Runtime;
use zgx_script::ScriptHost;

wasm_bindgen_test_configure!(run_in_browser);

struct Counting;
static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);
fn grew(by: isize) {
    let live = LIVE.fetch_add(by, Relaxed) + by;
    PEAK.fetch_max(live, Relaxed);
}
// SAFETY: each call is the system allocator's, and counts what it does.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grew(layout.size() as isize);
        // SAFETY: the caller's layout, as `GlobalAlloc::alloc` asks.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        grew(layout.size() as isize);
        // SAFETY: as `alloc`.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Relaxed);
        // SAFETY: the caller's block, from this allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        grew(size as isize - layout.size() as isize);
        // SAFETY: the caller's block and new size.
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

/// About halfway between what the build peaks at with the collector stepped
/// for the nodes' bytes (18.2 MiB, headless Chrome on macOS) and without
/// (31.1 MiB).
const BOUND: isize = 25 << 20;

/// The table as `examples/luau_bench.rs` writes it, its rows four nodes.
const TABLE: &str = r##"
local rows = ui.state("rows", {})
local selected = ui.state("selected", 0)
local function build(count)
    local made = table.create(count)
    for i = 1, count do
        made[i] = { id = i, label = `row {i}` }
    end
    return made
end
return ui.column {
    ui.button { "Run", on_click = function() rows:set(build(10000)) end },
    ui.button { "Clear", on_click = function() rows:set({}) end },
    ui.each(rows, function(item, _, current)
        return ui.row {
            h = 28,
            items = "center",
            bg = function()
                return if selected:get() == item.id then "#782828" else nil
            end,
            ui.text { tostring(item.id), w = 80 },
            ui.text { function() return current:get().label end, grow = 1, on_click = ui.set(selected, item.id) },
            ui.text { "x", w = 32 },
        }
    end, { key = function(item) return item.id end }),
}
"##;

#[wasm_bindgen_test]
fn a_build_after_a_collection_keeps_its_garbage_down() {
    let runtime = Runtime::new();
    let script = ScriptHost::inline(&runtime, "table.luau", TABLE);
    script.reload().expect("the script loads");
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let mounted = zgx_dom::mount(&runtime, &host, script.view());
    let buttons = host.query_selector_all("button").unwrap();
    let press = |i| {
        buttons
            .item(i)
            .unwrap()
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap()
            .click();
        zgx_dom::flush();
    };
    // A build, a clear, and the collection the idle trim runs.
    press(0);
    press(1);
    script.collect_garbage();
    let before = LIVE.load(Relaxed);
    PEAK.store(before, Relaxed);
    press(0);
    let peak = PEAK.load(Relaxed) - before;
    assert!(
        peak < BOUND,
        "a build of 10,000 rows peaked at {} MB over what was live before",
        peak >> 20
    );
    press(1);
    mounted.unmount();
}
