//! What a session's two threads share: the guest's notes for the UI thread,
//! the newest frame it presented, and the textures the UI no longer shows.
use crate::runtime::bindings::zgx::app::types as wit;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

/// How a session ended.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Ended {
    /// The guest exited (`wasi:cli/exit`), with its code.
    Exited(Option<i32>),
    /// It trapped, ran too long, or couldn't start: why.
    Failed(String),
    /// Closed by the host.
    Closed,
}

/// What the guest's thread hands the UI thread.
pub(crate) enum Note {
    /// Instantiated: it runs.
    Running,
    Title(String),
    Cursor(wit::Cursor),
    Lock(bool),
    Animating(bool),
    RequestFrame,
    /// A frame to show (`Shared::frame`).
    Presented,
    /// The guest's device can't import zgx's textures: it reads its frames
    /// back, and zgx stops making them.
    Unshared,
    Ended(Ended),
}

/// A frame the guest presented.
pub(crate) enum Presented {
    /// One of zgx's shared textures, by its ring's generation and index,
    /// drawn by the guest's device and its work done.
    Shared { generation: u32, index: u32 },
    /// Premultiplied rows from the CPU (`present-pixels`, RGBA) or read
    /// back from the guest's device (BGRA).
    Pixels {
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        bgra: bool,
    },
}

#[derive(Default)]
pub(crate) struct Shared {
    pub notes: Vec<Note>,
    /// The newest frame presented, not yet shown.
    pub frame: Option<Presented>,
    /// Shared textures zgx no longer reads (generation, index): the
    /// guest's to draw into again.
    pub released: Vec<(u32, u32)>,
}

/// The guest's way to the UI thread.
#[derive(Clone)]
pub(crate) struct Link {
    pub shared: Arc<Mutex<Shared>>,
    pub wake: zgx::task::LatestSender<()>,
    /// Input sent and not yet read by the guest (`MAX_QUEUED_INPUT`).
    pub input: Arc<AtomicUsize>,
}
impl Link {
    pub fn note(&self, note: Note) {
        self.shared.lock().unwrap().notes.push(note);
        self.wake.send(());
    }
    /// Show `frame` next: what it replaces, never shown, is returned.
    pub fn present(&self, frame: Presented) -> Option<Presented> {
        let replaced = {
            let mut shared = self.shared.lock().unwrap();
            let replaced = shared.frame.replace(frame);
            shared.notes.push(Note::Presented);
            replaced
        };
        self.wake.send(());
        replaced
    }
    /// The guest read an input event.
    pub fn input_read(&self) {
        // Every event sent was counted first (`Inner::input`).
        self.input.fetch_sub(1, Ordering::AcqRel);
    }
    /// Shared textures zgx gave back.
    pub fn released(&self) -> Vec<(u32, u32)> {
        std::mem::take(&mut self.shared.lock().unwrap().released)
    }
}
