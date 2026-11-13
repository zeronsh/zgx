//! A frame's DOM operations, written into one buffer and applied by the JS
//! side in one call (`zgx_apply`): the boundary is crossed once per flush,
//! not once per node. The op codes match `js/zgx-dom.js`.

use std::cell::RefCell;

pub(crate) const CREATE: u32 = 1;
pub(crate) const TEXT: u32 = 2;
pub(crate) const SET_TEXT: u32 = 3;
pub(crate) const CLASS: u32 = 4;
pub(crate) const STYLE: u32 = 5;
pub(crate) const ATTR: u32 = 6;
pub(crate) const REMOVE_ATTR: u32 = 7;
pub(crate) const APPEND: u32 = 8;
pub(crate) const INSERT: u32 = 9;
pub(crate) const REMOVE: u32 = 10;
pub(crate) const FREE: u32 = 11;
pub(crate) const CLEAR: u32 = 12;
pub(crate) const RULE: u32 = 13;
pub(crate) const HANDLER: u32 = 14;
pub(crate) const VALUE: u32 = 15;
pub(crate) const CHECKED: u32 = 16;
pub(crate) const NUMBER: u32 = 17;
pub(crate) const PROP: u32 = 18;
pub(crate) const CLONE: u32 = 19;
pub(crate) const FIRST: u32 = 20;
pub(crate) const NEXT: u32 = 21;
pub(crate) const IMAGE: u32 = 22;
pub(crate) const CLASS_NAME: u32 = 24;
pub(crate) const CLASS_ADD: u32 = 25;
pub(crate) const VISIBLE: u32 = 26;
pub(crate) const TRUNCATE: u32 = 27;
pub(crate) const WALK: u32 = 28;
pub(crate) const OVERLAY: u32 = 29;
pub(crate) const SHOW: u32 = 30;

/// Words a frame holds before it applies what it has (`Ops::op`).
pub(crate) const EARLY: usize = 1 << 12;

/// Element tags, by their index in the JS side's `TAGS`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u32)]
pub(crate) enum Tag {
    Div = 0,
    Span = 1,
    Button = 2,
    Input = 3,
    TextArea = 4,
    Progress = 5,
    Label = 6,
    Canvas = 8,
    Dialog = 9,
}

/// Handler kinds (`HANDLER`'s second word).
pub(crate) const ON_CLICK: u32 = 0;
pub(crate) const ON_INPUT: u32 = 1;
pub(crate) const ON_SCROLL: u32 = 2;
pub(crate) const SCROLL_X: u32 = 3;
pub(crate) const ON_RESIZE: u32 = 4;

/// `NUMBER`'s properties.
pub(crate) const NUM_VALUE: u32 = 0;
pub(crate) const NUM_SCROLL_TOP: u32 = 1;
pub(crate) const NUM_SCROLL_LEFT: u32 = 2;
pub(crate) const NUM_PROGRESS: u32 = 3;

#[derive(Default)]
pub(crate) struct Ops {
    pub(crate) words: Vec<u32>,
    /// The strings the words point into, as UTF-8; the words hold UTF-16
    /// offsets, which JS strings index by.
    pub(crate) text: String,
    text16: u32,
    /// Node ids: 0 is the root, freed ones are reused.
    next: u32,
    /// The lowest and highest the table's length was this frame, for
    /// shortening the JS side's (`TRUNCATE`).
    low: u32,
    high: u32,
    free: Vec<u32>,
    /// Pixels the JS side copies out this frame.
    pub(crate) keep: Vec<std::sync::Arc<zgx::image::ImageData>>,
    scheduled: bool,
    /// While a block is captured, the nodes something writes after it is
    /// made (an effect, a handler, a region, an observer): what a template's
    /// clone must reach (`template::stamp`).
    pub(crate) used: Vec<u32>,
    /// While a block's structure is captured (`capture`), the frame's own
    /// words, text and text length, set aside.
    main: Option<(Vec<u32>, String, u32)>,
}

/// A block's structural operations, captured to be stamped from a template
/// or replayed as they are (`template`).
pub(crate) struct Captured {
    pub(crate) words: Vec<u32>,
    pub(crate) text: String,
}

thread_local! {
    pub(crate) static OPS: RefCell<Ops> = RefCell::new(Ops { next: 1, low: 1, high: 1, ..Ops::default() });
}

impl Ops {
    #[inline]
    pub(crate) fn id(&mut self) -> u32 {
        match self.free.pop() {
            Some(id) => id,
            None => {
                let id = self.next;
                self.next += 1;
                self.high = self.high.max(self.next);
                id
            }
        }
    }
    #[inline]
    pub(crate) fn op(&mut self, words: &[u32]) {
        self.schedule();
        // A long frame (thousands of rows made) applies what it has so far
        // between operations, so its buffer stays small: what JS applies
        // is in order either way. Not while a block is captured.
        if self.words.len() >= EARLY && self.main.is_none() {
            self.apply();
        }
        self.words.extend_from_slice(words);
    }
    /// Make room for what follows: what is written so far applied, if the
    /// frame is long (as `op` does), before text is appended that words
    /// written after it point into (`template::replay_mapped`).
    pub(crate) fn room(&mut self) {
        self.schedule();
        if self.words.len() >= EARLY && self.main.is_none() {
            self.apply();
        }
    }
    /// Words written as they are: no early apply between them and the text
    /// they point into (see `room`).
    pub(crate) fn op_raw(&mut self, words: &[u32]) {
        self.schedule();
        self.words.extend_from_slice(words);
    }
    /// Apply the words written so far (without the frame's frees, which
    /// wait for its end).
    pub(crate) fn apply(&mut self) {
        if !self.words.is_empty() {
            crate::js::zgx_apply(
                &wasm_bindgen::memory(),
                self.words.as_ptr() as u32,
                self.words.len() as u32,
                self.text.as_ptr() as u32,
                self.text.len() as u32,
            );
        }
        self.words.clear();
        self.text.clear();
        self.text16 = 0;
        self.keep.clear();
    }
    #[inline]
    pub(crate) fn str(&mut self, s: &str) {
        let (at, len) = self.text_at(s);
        self.words.push(at);
        self.words.push(len);
    }
    #[inline]
    pub(crate) fn op_str(&mut self, code: u32, id: u32, s: &str) {
        self.op(&[code, id]);
        self.str(s);
    }
    /// `node` is written after its block is made (see `used`).
    #[inline]
    pub(crate) fn mark(&mut self, node: u32) {
        if self.main.is_some() {
            self.used.push(node);
        }
    }
    /// Ids that were never written to the JS side's table, free again.
    pub(crate) fn release(&mut self, ids: &[u32]) {
        for &id in ids {
            if id + 1 == self.next {
                self.next = id;
                while self.free.last() == Some(&(self.next - 1)) {
                    self.free.pop();
                    self.next -= 1;
                }
            } else {
                self.free.push(id);
            }
        }
    }
    pub(crate) fn capturing(&self) -> bool {
        self.main.is_some()
    }
    /// Write what follows aside, until `captured`.
    pub(crate) fn capture(&mut self, spare: Captured) {
        debug_assert!(self.main.is_none());
        let words = std::mem::replace(&mut self.words, spare.words);
        let text = std::mem::replace(&mut self.text, spare.text);
        self.words.clear();
        self.text.clear();
        self.main = Some((words, text, std::mem::take(&mut self.text16)));
    }
    /// What was written since `capture`, the frame's own words back.
    pub(crate) fn captured(&mut self) -> Captured {
        let (words, text, text16) = self.main.take().expect("capturing");
        self.text16 = text16;
        Captured {
            words: std::mem::replace(&mut self.words, words),
            text: std::mem::replace(&mut self.text, text),
        }
    }
    /// An operation that defines (a rule, a class name), written to the
    /// frame even while capturing: what is captured may use it.
    pub(crate) fn define(&mut self, words: &[u32], s: &str) {
        self.schedule();
        match &mut self.main {
            None => {
                self.words.extend_from_slice(words);
                self.str(s);
            }
            Some((main, text, text16)) => {
                main.extend_from_slice(words);
                let len = utf16_len(s);
                main.push(*text16);
                main.push(len);
                text.push_str(s);
                *text16 += len;
            }
        }
    }
    /// Append `s` to the text, returning its UTF-16 offset and length.
    #[inline]
    pub(crate) fn text_at(&mut self, s: &str) -> (u32, u32) {
        let len = utf16_len(s);
        let at = self.text16;
        self.text.push_str(s);
        self.text16 += len;
        (at, len)
    }
    /// Let go of nodes: their table entries cleared, in order with what
    /// came before (so the ids are reusable at once: anything reusing one
    /// comes after), written as the frame goes rather than gathered for its
    /// end, which a big list's clear would grow a buffer for.
    pub(crate) fn free(&mut self, ids: &[u32]) {
        if ids.is_empty() {
            return;
        }
        self.schedule();
        match &mut self.main {
            // Captured: into the frame's own words, which come first.
            Some((main, _, _)) => {
                main.push(FREE);
                main.push(ids.len() as u32);
                main.extend_from_slice(ids);
            }
            None => {
                if self.words.len() >= EARLY {
                    self.apply();
                }
                self.words.push(FREE);
                self.words.push(ids.len() as u32);
                self.words.extend_from_slice(ids);
            }
        }
        // Freed from the top of the table (a list cleared newest first, a
        // block's nodes last first), the table shrinks instead of keeping
        // a list of free ids, which a big list's clear would grow.
        for &id in ids {
            if id + 1 == self.next {
                self.next = id;
                while self.free.last() == Some(&(self.next - 1)) {
                    self.free.pop();
                    self.next -= 1;
                }
            } else {
                self.free.push(id);
            }
        }
        self.low = self.low.min(self.next);
    }
    fn schedule(&mut self) {
        if !self.scheduled {
            self.scheduled = true;
            crate::js::zgx_schedule();
        }
    }
    /// The frame's words with its frees appended, for the JS side; ids freed
    /// are reusable once applied.
    pub(crate) fn seal(&mut self) {
        // Mostly free still (ids freed out of order): the free ids at the
        // end go too.
        if self.free.len() > 4096 && self.free.len() * 2 > self.next as usize {
            self.free.sort_unstable();
            while self.free.last() == Some(&(self.next - 1)) {
                self.free.pop();
                self.next -= 1;
            }
            self.low = self.low.min(self.next);
        }
        // The JS side's table shortens to what is in use.
        if self.low < self.high {
            self.words.push(TRUNCATE);
            self.words.push(self.next);
        }
        self.low = self.next;
        self.high = self.next;
    }
    pub(crate) fn clear(&mut self) {
        self.apply();
        self.scheduled = false;
    }
}

#[inline]
pub(crate) fn utf16_len(s: &str) -> u32 {
    if s.is_ascii() {
        s.len() as u32
    } else {
        s.encode_utf16().count() as u32
    }
}

#[inline]
pub(crate) fn with<R>(f: impl FnOnce(&mut Ops) -> R) -> R {
    OPS.with(|ops| f(&mut ops.borrow_mut()))
}
