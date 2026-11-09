//! Who holds which buffer: the host's side of the ring (what was
//! presented, what shows, what goes back) and the client's (what is free to
//! draw into). Neither touches a GPU, so both are tested alone.
use std::collections::VecDeque;

/// A buffer by generation and index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Slot {
    /// Its generation.
    pub generation: u32,
    /// Its index in the ring.
    pub index: u32,
}

/// The host's side: the newest frame presented and not yet shown, and the
/// one showing. A frame presented over one not yet shown goes back at once
/// (it was never read); the one showing goes back once a newer shows and
/// the GPU work that read it is done.
#[derive(Debug, Default)]
pub struct HostRing {
    generation: u32,
    count: u32,
    pending: Option<Slot>,
    shown: Option<Slot>,
}
impl HostRing {
    /// A ring of nothing yet.
    pub fn new() -> Self {
        Self::default()
    }
    /// The current generation (0 before the first).
    pub fn generation(&self) -> u32 {
        self.generation
    }
    /// Start generation `generation + 1` of `count` buffers. A frame of
    /// the last presented and not yet shown is dropped (the client forgets
    /// an old generation's buffers); the one showing stays until a new one
    /// shows, stretched meanwhile.
    pub fn next_generation(&mut self, count: u32) -> u32 {
        self.generation += 1;
        self.count = count;
        self.pending = None;
        self.generation
    }
    /// The client presented `slot`. What to release at once, if anything:
    /// the frame it replaces before it showed, or `slot` itself if it is of
    /// an old generation or out of the ring. `Err` names a buffer the host
    /// holds being presented again, which a broken client does.
    pub fn present(&mut self, slot: Slot) -> Result<Option<Slot>, Slot> {
        if slot.generation != self.generation || slot.index >= self.count {
            // An old generation's frame: the client has forgotten it.
            return Ok(None);
        }
        if self.pending == Some(slot) || self.shown == Some(slot) {
            return Err(slot);
        }
        Ok(self.pending.replace(slot))
    }
    /// The newest frame presented becomes the one showing: it, and the one
    /// it replaces (to release once the GPU has read it for the last time).
    /// None when nothing new was presented.
    pub fn show_newest(&mut self) -> Option<(Slot, Option<Slot>)> {
        let newest = self.pending.take()?;
        let previous = self.shown.replace(newest);
        Some((newest, previous))
    }
    /// The newest frame presented, for a kind whose buffers go back as soon
    /// as they are read (shm, uploaded): nothing is kept as showing.
    pub fn take_newest(&mut self) -> Option<Slot> {
        self.pending.take()
    }
    /// The frame showing.
    pub fn shown(&self) -> Option<Slot> {
        self.shown
    }
    /// Whether a presented frame waits to show.
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
}

/// The client's side: the current generation's buffers free to draw into.
#[derive(Debug, Default)]
pub struct ClientRing {
    generation: u32,
    count: u32,
    free: VecDeque<u32>,
}
impl ClientRing {
    /// A ring of nothing yet.
    pub fn new() -> Self {
        Self::default()
    }
    /// The host sent `generation` of `count` buffers: all are free, the
    /// last generation's forgotten.
    pub fn reset(&mut self, generation: u32, count: u32) {
        self.generation = generation;
        self.count = count;
        self.free = (0..count).collect();
    }
    /// The current generation (0 before the first).
    pub fn generation(&self) -> u32 {
        self.generation
    }
    /// A buffer to draw into, if one is free.
    pub fn acquire(&mut self) -> Option<u32> {
        self.free.pop_front()
    }
    /// Give back a buffer acquired and not presented.
    pub fn cancel(&mut self, index: u32) {
        if index < self.count && !self.free.contains(&index) {
            self.free.push_front(index);
        }
    }
    /// The host released `slot`: free again, if of this generation.
    pub fn release(&mut self, slot: Slot) {
        if slot.generation == self.generation
            && slot.index < self.count
            && !self.free.contains(&slot.index)
        {
            self.free.push_back(slot.index);
        }
    }
    /// How many are free.
    pub fn free(&self) -> usize {
        self.free.len()
    }
}
