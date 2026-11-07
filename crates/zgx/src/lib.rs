//! Fine-grained reactive state and a retained, damage-tracked scene.
//! The core has no platform dependencies and can be targeted by template compilers.
#![warn(missing_docs)]
pub mod collections;
pub mod reactive;
pub mod scene;
pub mod task;
pub mod view;
pub mod virtual_list;

pub mod input;
pub mod semantics;
pub mod text;
pub mod text_edit;
mod words;

pub mod widgets;

pub mod keyed;

pub mod image;

pub mod gpu_view;

pub mod components;

pub mod i18n;
pub mod issues;
pub mod local_time;
pub mod timer;

pub mod env;
pub mod frame;
pub mod idle;
pub mod profile;
pub mod switches;
#[cfg(feature = "trace")]
pub mod trace;

pub mod caret_blink;
pub mod clock;
pub(crate) mod editor_autocorrect;
pub(crate) mod editor_lines;
pub(crate) mod editor_menu;
pub(crate) mod editor_spell;
pub(crate) mod elastic;
pub mod find;
mod find_text;
pub mod rich_text;
mod scroll_keys;
pub mod selection;
pub mod text_layout;
mod typeahead;
pub mod zoom;

pub mod affine;
pub mod background;

pub mod compose;
mod compose_checkbox;
mod compose_menu;
mod compose_menu_scroll;
mod compose_modal;
mod compose_popover;
mod compose_rich;
mod compose_scrollbar;
mod compose_split;
mod compose_style;
mod compose_tooltip;
mod compose_virtual_keyboard;
pub mod container_query;
pub mod style;
pub mod theme;

#[cfg(doctest)]
#[doc = include_str!("../../../docs/composition.md")]
mod composition_guide {}
#[cfg(doctest)]
#[doc = include_str!("../../../docs/styling.md")]
mod styling_guide {}
#[cfg(doctest)]
#[doc = include_str!("../../../docs/i18n.md")]
mod i18n_guide {}

pub mod actions;

mod action_context;

pub mod canvas;

pub mod animation;

pub mod grid_tracks;
pub mod layout;
pub mod size_calc;

pub mod image_cache;
pub mod memory;

pub mod decoration;
pub mod glass;
pub mod shader_drive;

pub mod svg;

pub mod cursor;

pub mod inspect;

mod compose_context_menu;
mod compose_drag;

#[cfg(target_os = "macos")]
pub mod native_surface;

pub mod native_view;
/// The clipboard and file dialogs, as a service a host provides.
pub mod platform;
pub mod terminal;
pub mod web_view;

/// Gives a map's room back once it is mostly empty (a page of thousands of
/// nodes gone): maps keep their peak capacity otherwise. Only past 1024
/// slots and under a quarter used, so rehashing is rare.
pub(crate) fn shrink_sparse<K: std::hash::Hash + Eq, V, S: std::hash::BuildHasher>(
    map: &mut std::collections::HashMap<K, V, S>,
) {
    if map.capacity() > 1024 && map.len() * 4 < map.capacity() {
        map.shrink_to(map.len() * 2);
    }
}

/// The hasher of keys from an app's data (a list's row keys): fast, and
/// seeded once per process from std's random seed, so keys made to collide
/// cannot slow every lookup, as they can a fixed hasher's. What depends on
/// order iterates in the list's own (a map's differs from run to run).
pub(crate) type KeyHasher = foldhash::fast::FixedState;
pub(crate) fn key_hasher() -> KeyHasher {
    static SEED: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    foldhash::fast::FixedState::with_seed(*SEED.get_or_init(|| {
        use std::hash::BuildHasher;
        std::collections::hash_map::RandomState::new().hash_one(0_u8)
    }))
}

/// Runs `f`, level `level` of a recursion over a tree (mounting, lowering,
/// layout), with room on the stack for it: when the thread's stack runs
/// low, `f` runs on a fresh segment instead. Trees nest as deep as an app
/// makes them and a debug build's frames are large; a 1 MB stack (the main
/// thread's on Windows and iOS) would otherwise end the process. The stack is
/// looked at every fourth level, which costs less than the levels between;
/// the room kept covers them.
pub fn deep<R>(level: u32, f: impl FnOnce() -> R) -> R {
    #[cfg(any(target_os = "ios", all(test, unix)))]
    if stack_low(level) {
        on_segment(f)
    } else {
        f()
    }
    #[cfg(not(any(target_os = "ios", all(test, unix))))]
    if stack_low(level) {
        stacker::grow(STACK_SEGMENT, f)
    } else {
        f()
    }
}
/// Whether level `level` of a recursion should go on a fresh stack segment
/// (`deep`), for a caller that would rather not move what it passes into a
/// closure on the common path.
pub(crate) fn stack_low(level: u32) -> bool {
    #[cfg(any(target_os = "ios", all(test, unix)))]
    let left = segment::remaining;
    #[cfg(not(any(target_os = "ios", all(test, unix))))]
    let left = stacker::remaining_stack;
    level.is_multiple_of(4) && left().is_none_or(|left| left < 256 * 1024)
}
/// The stack a recursion goes on when the thread's runs low (`deep`).
pub(crate) const STACK_SEGMENT: usize = 2 * 1024 * 1024;
/// Runs `f` on a segment of `STACK_SEGMENT`: the thread's kept one where it
/// isn't in use already, so that the siblings of a level past the low mark
/// (a list's thousand rows) share one, not each map, guard and free its own.
#[cfg(any(target_os = "ios", all(test, unix)))]
pub(crate) fn on_segment<R>(f: impl FnOnce() -> R) -> R {
    segment::run(f)
}
/// `on_segment` where no segment is kept: one of `stacker`'s own.
#[cfg(not(any(target_os = "ios", all(test, unix))))]
#[inline(always)]
pub(crate) fn on_segment<R>(f: impl FnOnce() -> R) -> R {
    stacker::grow(STACK_SEGMENT, f)
}

/// The segment each thread keeps for `deep`, on iOS, whose main thread has
/// 1 MB of stack and is well under the low mark by the time a flush runs:
/// a fresh `stacker` segment for each of a list's rows mapped, guarded and
/// freed one per row. Elsewhere (a desktop's roomy main thread, Windows'
/// fibers) `stacker`'s own as it was, with nothing more read on the path
/// every recursion takes; zgx's own tests run the kept one on any Unix.
#[cfg(any(target_os = "ios", all(test, unix)))]
mod segment {
    use std::cell::Cell;

    /// The kept segment's lowest usable address (above its guard page), if
    /// made, and whether a recursion runs on it now.
    struct Kept {
        base: Cell<usize>,
        busy: Cell<bool>,
    }
    thread_local! {
        static KEPT: Kept = const { Kept { base: Cell::new(0), busy: Cell::new(false) } };
    }
    /// Segments this thread has mapped, for tests.
    #[cfg(test)]
    pub(crate) fn made() -> usize {
        MADE.with(Cell::get)
    }
    #[cfg(test)]
    thread_local! {
        static MADE: Cell<usize> = const { Cell::new(0) };
        static GROWN: Cell<usize> = const { Cell::new(0) };
    }
    /// Segments `stacker` made this thread, the kept one being in use.
    #[cfg(test)]
    pub(crate) fn grown() -> usize {
        GROWN.with(Cell::get)
    }

    fn page() -> usize {
        // SAFETY: sysconf reads a constant of the system's.
        let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        usize::try_from(size)
            .ok()
            .filter(|&size| size > 0)
            .unwrap_or(16 * 1024)
    }
    /// The kept segment's base, mapped the first time: `STACK_SEGMENT` with
    /// a page below it that faults, as a thread's own stack has. None if
    /// the system won't map it.
    fn base() -> Option<usize> {
        KEPT.with(|kept| {
            if kept.base.get() != 0 {
                return Some(kept.base.get());
            }
            let guard = page();
            // SAFETY: An anonymous private mapping of our own, then its first
            // page made unreadable; nothing else refers to it.
            let base = unsafe {
                let mapped = libc::mmap(
                    std::ptr::null_mut(),
                    super::STACK_SEGMENT + guard,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                );
                if mapped == libc::MAP_FAILED {
                    return None;
                }
                if libc::mprotect(mapped, guard, libc::PROT_NONE) != 0 {
                    libc::munmap(mapped, super::STACK_SEGMENT + guard);
                    return None;
                }
                mapped as usize + guard
            };
            #[cfg(test)]
            MADE.with(|made| made.set(made.get() + 1));
            kept.base.set(base);
            Some(base)
        })
    }
    /// The stack left: on the kept segment, down to its base; elsewhere as
    /// `stacker` counts it (the thread's own, or a segment it made).
    #[inline(always)]
    pub(super) fn remaining() -> Option<usize> {
        // Off the kept segment (always, where the thread's stack is roomy):
        // as `stacker` counts it, with nothing more read.
        let base = KEPT.with(|kept| kept.busy.get().then(|| kept.base.get()));
        let Some(base) = base else {
            return stacker::remaining_stack();
        };
        on_kept(base)
    }
    /// `remaining` with the kept segment in use: down to its base while on
    /// it, else (a `stacker` segment grown past it) as `stacker` counts.
    #[inline(never)]
    fn on_kept(base: usize) -> Option<usize> {
        let at = psm::stack_pointer() as usize;
        match (base..base + super::STACK_SEGMENT).contains(&at) {
            true => Some(at - base),
            false => stacker::remaining_stack(),
        }
    }
    pub(super) fn run<R>(f: impl FnOnce() -> R) -> R {
        let free = KEPT.with(|kept| !kept.busy.get());
        let Some(base) = free.then(base).flatten() else {
            // In use further up (a recursion deeper than a segment), or
            // none to be had: one of `stacker`'s own.
            #[cfg(test)]
            GROWN.with(|grown| grown.set(grown.get() + 1));
            return stacker::grow(super::STACK_SEGMENT, f);
        };
        struct Busy;
        impl Drop for Busy {
            fn drop(&mut self) {
                KEPT.with(|kept| kept.busy.set(false));
            }
        }
        KEPT.with(|kept| kept.busy.set(true));
        let _busy = Busy;
        let mut f = Some(f);
        let mut out = None;
        // SAFETY: `base` is `STACK_SEGMENT` of writable memory this thread
        // keeps and nothing else runs on (`busy`), page aligned; the callback
        // doesn't unwind out of it (a panic is caught and resumed here).
        let panic = unsafe {
            psm::on_stack(base as *mut u8, super::STACK_SEGMENT, || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    out = Some((f.take().expect("run once"))());
                }))
                .err()
            })
        };
        if let Some(panic) = panic {
            std::panic::resume_unwind(panic);
        }
        out.expect("ran")
    }
}

#[cfg(all(test, unix))]
mod segment_tests {
    /// Levels each with a frame of `bytes`, `deep` at each: as a tree's
    /// recursion goes.
    fn descend(level: u32, levels: u32) -> u32 {
        let frame = std::hint::black_box([level as u8; 2048]);
        if level == levels {
            return u32::from(frame[0]);
        }
        let below = super::deep(level, || descend(level + 1, levels));
        // The frame stays live across the call, as a real level's does.
        std::hint::black_box(&frame);
        below
    }

    #[test]
    fn siblings_past_the_low_mark_share_one_segment() {
        // A stack already under the low mark (as iOS's main thread is,
        // deep in a flush): every sibling goes to a segment.
        let made = std::thread::Builder::new()
            .stack_size(160 * 1024)
            .spawn(|| {
                assert!(super::stack_low(0), "under the mark");
                for _ in 0..1000 {
                    super::deep(0, || std::hint::black_box(1));
                }
                super::segment::made()
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(made, 1, "one kept, not one each");
    }

    #[test]
    fn a_recursion_deeper_than_a_segment_goes_on_to_more() {
        // 4,000 levels of 2 KB: four times a segment, on a thread of 160 KB.
        let got = std::thread::Builder::new()
            .stack_size(160 * 1024)
            .spawn(|| {
                let got = descend(0, 4000);
                // The kept segment is free again for the next.
                super::deep(0, || ());
                (got, super::segment::made())
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(got, (4000u32 as u8 as u32, 1));
    }

    /// Runs `f` with less than `left` of the thread's stack remaining, as a
    /// flush runs deep in a platform's own calls.
    fn with_stack_left<R>(left: usize, f: impl FnOnce() -> R) -> R {
        let frame = std::hint::black_box([0u8; 4096]);
        let out = match stacker::remaining_stack() {
            Some(now) if now > left => with_stack_left(left, f),
            _ => f(),
        };
        std::hint::black_box(&frame);
        out
    }

    /// A thousand rows mounted and laid out on a thread of 1 MB (iOS's main
    /// thread, and Windows'), with its stack already under the low mark:
    /// one segment mapped for them all, none made beside it.
    #[test]
    fn a_deep_mount_on_a_small_stack_maps_one_segment() {
        let (made, grown, nodes) = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                with_stack_left(200 * 1024, || {
                    use crate::compose::prelude::*;
                    let mut ui = crate::widgets::Ui::new(400., 800.);
                    let rows = (0..1000).map(|i| row().px(16.).child(text(format!("Row {i}"))));
                    let offset = ui.signal(0.);
                    let _mounted = ui.mount(scroll(offset).child(column().children(rows)));
                    ui.prepare_frame();
                    let nodes = ui.scene.borrow().len();
                    (super::segment::made(), super::segment::grown(), nodes)
                })
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(nodes > 2000, "{nodes} nodes");
        assert_eq!((made, grown), (1, 0), "one kept segment, none made per row");
    }

    #[test]
    fn a_panic_on_the_segment_comes_back_and_frees_it() {
        std::thread::Builder::new()
            .stack_size(160 * 1024)
            .spawn(|| {
                let caught = std::panic::catch_unwind(|| super::deep(0, || panic!("in a row")));
                assert!(caught.is_err());
                assert_eq!(super::deep(0, || 7), 7, "usable after");
                assert_eq!(super::segment::made(), 1);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
