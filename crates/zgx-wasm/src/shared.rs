//! What a session's two threads share: the guest's notes for the UI thread,
//! the newest frame it presented, and the textures the UI no longer shows.
use crate::runtime::bindings::zgx::app::types as wit;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use zgx_gpu::wgpu;

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
    Ended(Ended),
}

/// A frame the guest presented.
pub(crate) enum Presented {
    /// A texture on the renderer's device (the painter's `generation`th),
    /// drawn by the GPU.
    Texture {
        texture: wgpu::Texture,
        generation: u64,
    },
    /// RGBA8 premultiplied rows from the CPU (`present-pixels`).
    Pixels {
        pixels: Vec<u8>,
        width: u32,
        height: u32,
    },
}

#[derive(Default)]
pub(crate) struct Shared {
    pub notes: Vec<Note>,
    /// The newest frame presented, not yet shown.
    pub frame: Option<Presented>,
    /// Textures the UI no longer shows, the guest's to draw into again.
    pub returned: Vec<wgpu::Texture>,
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
    /// Textures the UI gave back.
    pub fn returned(&self) -> Vec<wgpu::Texture> {
        std::mem::take(&mut self.shared.lock().unwrap().returned)
    }
}
