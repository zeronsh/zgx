//! Isolate nested-scroll projection and AccessKit consumer updates without
//! a window, for repeated instruction comparisons of projection changes.
//! AX_ROWS=600 AX_STEPS=400; AX_APPLY=0 omits consumer application.
use accesskit_consumer::{Node, Tree, TreeChangeHandler};
use zgx::{compose::prelude::*, widgets::Ui};
use zgx_desktop::accessibility::AccessibilityTree;

struct Changes;
impl TreeChangeHandler for Changes {
    fn node_added(&mut self, _: &Node) {}
    fn node_updated(&mut self, _: &Node, _: &Node) {}
    fn focus_moved(&mut self, _: Option<&Node>, _: Option<&Node>) {}
    fn node_removed(&mut self, _: &Node) {}
}
fn setting(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
#[cfg(target_os = "macos")]
fn instructions() -> u64 {
    unsafe extern "C" {
        fn thread_selfcounts(kind: i32, buffer: *mut u64, size: usize) -> i32;
    }
    let mut counts = [0_u64; 2];
    // SAFETY: the kernel writes two u64 counters to this live buffer.
    assert_eq!(
        unsafe { thread_selfcounts(1, counts.as_mut_ptr(), std::mem::size_of_val(&counts)) },
        0
    );
    counts[0]
}
#[cfg(not(target_os = "macos"))]
fn instructions() -> u64 {
    0
}

fn main() {
    let padding: Vec<_> = (0..setting("AX_HEAP_PAD", 0))
        .map(|i| vec![i as u8; 37 + i % 193])
        .collect();
    let mut ui = Ui::new(800., 600.);
    ui.set_assistive_technology(true);
    let outer = ui.signal(0_f32);
    let inner = ui.signal(0_f32);
    ui.mount(
        scroll(outer.clone()).h(500.).child(
            column()
                .w(750.)
                .child(
                    scroll(inner.clone()).h(200.).child(
                        column().w(700.).children(
                            (0..setting("AX_ROWS", 600))
                                .map(|i| text(format!("Nested row {i}: words to read")).h(24.)),
                        ),
                    ),
                )
                .children((0..40).map(|i| text(format!("Outer row {i}")).h(24.))),
        ),
    );
    let mut projection = AccessibilityTree::new();
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    projection.note_flush(&ui.scene.borrow());
    let initial = projection.update(&ui.scene.borrow(), &ui.semantics.borrow(), None, "AX", 2.);
    let mut consumer = Tree::new(initial, true);
    let apply =
        std::env::var("AX_APPLY").is_ok_and(|v| v != "0") || std::env::var_os("AX_APPLY").is_none();
    let mut sent = 0;
    let (mut projection_instructions, mut consumer_instructions) = (0, 0);
    for step in 0..setting("AX_STEPS", 400) + 50 {
        outer.set((step % 50) as f32 * 2.);
        inner.set((step % 200) as f32 * 3.);
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        projection.note_flush(&ui.scene.borrow());
        let before = instructions();
        let update = projection.update(&ui.scene.borrow(), &ui.semantics.borrow(), None, "AX", 2.);
        sent += update.nodes.len();
        if apply {
            let projected = instructions();
            consumer.update_and_process_changes(update, &mut Changes);
            let applied = instructions();
            if step >= 50 {
                projection_instructions += projected - before;
                consumer_instructions += applied - projected;
            }
        } else {
            std::hint::black_box(update);
            let after = instructions();
            if step >= 50 {
                projection_instructions += after - before;
            }
        }
    }
    std::hint::black_box(consumer);
    std::hint::black_box(padding);
    println!(
        "AX nodes={sent} projection_instructions={projection_instructions} consumer_instructions={consumer_instructions}"
    );
}
