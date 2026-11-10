//! A memory report names what an interface holds: of the heap a mounted
//! page takes, its lines account for most, so a holder no line names shows.
//! A process of its own: it counts every allocation of the process.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};

mod common;
use common::Harness;

struct Counting;
static LIVE: AtomicIsize = AtomicIsize::new(0);
// SAFETY: each call is the system allocator's, and counts what it does.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Relaxed);
        // SAFETY: the caller's layout, as `GlobalAlloc::alloc` asks.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Relaxed);
        // SAFETY: as `alloc`.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Relaxed);
        // SAFETY: the caller's block, from this allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        LIVE.fetch_add(size as isize - layout.size() as isize, Relaxed);
        // SAFETY: the caller's block and new size.
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

/// A page as an app's first screen is: a list of conversations, each a
/// row of a dot, two lines and a time, hovered and selected; a search
/// field; a composer.
const PAGE: &str = r##"
local selected = ui.state("selected", 1)
local ids = ui.state("ids", (function()
	local t = {}
	for i = 1, 200 do
		t[i] = i
	end
	return t
end)())
local function entry(i: number)
	local hover = ui.use_state(false)
	return ui.row {
		px = 10,
		py = 6,
		gap = 8,
		items = "center",
		rounded = 6,
		bg = function()
			return if selected:get() == i then "#E8E8ED" elseif hover:get() then "#F2F2F7" else nil
		end,
		on_hover = function(h)
			hover:set(h)
		end,
		on_click = function()
			selected:set(i)
		end,
		ui.div { w = 8, h = 8, rounded = 4, bg = if i % 3 == 0 then "#34C759" else "#C7C7CC" },
		ui.column {
			grow = 1,
			min_w = 0,
			ui.text { `Conversation about topic {i}`, text_size = 13, weight = 500, truncate = true },
			ui.text { `The last message in conversation {i}`, text_size = 12, truncate = true },
		},
		ui.text { `{i % 12 + 1}h`, text_size = 11 },
	}
end
return ui.row {
	w_full = true,
	h_full = true,
	ui.scroll {
		w = 280,
		h_full = true,
		ui.column { p = 8, gap = 2, ui.input { value = ui.state("query", ""), placeholder = "Search" }, ui.each(ids, entry) },
	},
	ui.column {
		grow = 1,
		ui.text { "What can I help with?", text_size = 24 },
		ui.input { value = ui.state("draft", ""), placeholder = "Message" },
	},
}
"##;

/// Whether a line's bytes are the program's heap and not part of another
/// line: mapped files and pooled pages are outside it, and one state's
/// lines are part of all named states'.
fn on_the_heap(line: &zgx::memory::Line) -> bool {
    !matches!(
        (line.area, line.name.as_str()),
        ("text", "fonts mapped from files (their size)")
            | ("luau", "of which pooled pages (every VM)")
    ) && (line.area != "state" || line.name == "all named states")
        && !matches!(line.area, "gpu" | "process" | "heap")
}

#[test]
fn a_memory_report_names_most_of_what_a_page_holds() {
    let before = LIVE.load(Relaxed);
    let h = Harness::inline(PAGE);
    h.settle();
    // At rest, as a host's idle trim leaves it: the script's garbage gone.
    h.host.collect_garbage();
    let held = (LIVE.load(Relaxed) - before) as usize;
    let mut report = zgx::memory::Report::default();
    h.ui.memory(&mut report);
    h.host.memory(&mut report);
    let named: usize = report
        .lines
        .iter()
        .filter(|line| on_the_heap(line))
        .filter_map(|line| line.bytes)
        .sum();
    let lines: String = report
        .lines
        .iter()
        .filter(|line| line.bytes.is_some_and(|b| b > 0))
        .map(|line| {
            format!(
                "\n  {} / {}: {:?}",
                line.area,
                line.name,
                line.bytes.unwrap()
            )
        })
        .collect();
    let nodes = report
        .lines
        .iter()
        .find(|line| line.name == "nodes mounted")
        .and_then(|line| line.entries)
        .unwrap();
    assert!(nodes > 1000, "{nodes} nodes: the page should be a real one");
    assert!(
        named * 100 >= held * 85,
        "the page holds {held} bytes, its report names {named} ({}%):{lines}",
        named * 100 / held
    );
}
