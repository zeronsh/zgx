//! What each finished frame did, for profilers and diagnostics: the areas it
//! damaged, the nodes it laid out, painted and composited, and whether it drew
//! the whole window.
//!
//! A host publishes its frames to a [`Diagnostics`]; a profiler watches them
//! with [`Diagnostics::on_frame`] and draws an overlay from what it reads. The
//! hook runs on the UI thread between frames, before the next one is laid out,
//! so what it changes in the scene paints in the next frame. Nothing is
//! published unless a host publishes it, and with no hook a publish is one
//! `Option` check.
use crate::scene::Rect;
use std::{cell::RefCell, rc::Rc, time::Instant};

/// One finished frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameSample {
    /// The host's count of frames drawn, from its first.
    pub frame: u64,
    /// When the frame's scene flushed.
    pub at: Option<Instant>,
    /// The areas to repaint, in logical points.
    pub damage: Vec<Rect>,
    /// Nodes laid out, painted and composited again.
    pub layout_nodes: usize,
    /// Nodes painted and composited again.
    pub paint_nodes: usize,
    /// Nodes only composited again (moved, faded, transformed).
    pub composite_nodes: usize,
    /// The frame drew the whole window rather than its damage.
    pub full: bool,
}
impl FrameSample {
    /// The damaged area in square logical points.
    pub fn damage_area(&self) -> f32 {
        self.damage
            .iter()
            .map(|rect| rect.width.max(0.) * rect.height.max(0.))
            .sum()
    }
    /// Whether nothing changed: no damage and no work.
    pub fn is_idle(&self) -> bool {
        self.damage.is_empty()
            && self.layout_nodes == 0
            && self.paint_nodes == 0
            && self.composite_nodes == 0
    }
}
type Hook = Rc<dyn Fn(&FrameSample)>;

/// A window's finished frames. Cloning shares them.
#[derive(Clone, Default)]
pub struct Diagnostics(Rc<Inner>);
#[derive(Default)]
struct Inner {
    last: RefCell<Option<FrameSample>>,
    hook: RefCell<Option<Hook>>,
}
impl Diagnostics {
    /// A window's frames, with nothing watching them yet.
    pub fn new() -> Self {
        Self::default()
    }
    /// The last frame published, if any.
    pub fn last(&self) -> Option<FrameSample> {
        self.0.last.borrow().clone()
    }
    /// Watch every finished frame, replacing any hook there was. The hook runs
    /// before the next frame's layout, so its own changes paint in that frame.
    pub fn on_frame(&self, hook: impl Fn(&FrameSample) + 'static) {
        *self.0.hook.borrow_mut() = Some(Rc::new(hook));
    }
    /// Stop watching.
    pub fn clear_frame_hook(&self) {
        *self.0.hook.borrow_mut() = None;
    }
    /// Whether anything watches frames: hosts skip building a sample when not.
    pub fn watched(&self) -> bool {
        self.0.hook.borrow().is_some()
    }
    /// A host's finished frame, given to whatever watches.
    pub fn publish(&self, sample: FrameSample) {
        let hook = self.0.hook.borrow().clone();
        if let Some(hook) = hook {
            *self.0.last.borrow_mut() = Some(sample.clone());
            hook(&sample);
        }
    }
}
