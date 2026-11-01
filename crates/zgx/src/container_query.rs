//! Container queries: styles a view takes while the box an ancestor marks
//! (`View::container`) is wide or tall enough, as CSS's `@container` rules
//! apply, without script. Layout publishes a container's size only as it
//! crosses a bound one of its queries names, so the views that ask restyle
//! then, in the frame it changes, and at no other time.
use std::{cell::RefCell, rc::Rc};

use crate::{reactive::Signal, scene::NodeId, widgets::Ui};

/// A container's content box: width, height.
type Size = (f32, f32);

/// When a container query holds: each bound given, inclusive, on the
/// container's content box, in points. A container not yet laid out holds
/// none.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Query {
    /// At least this wide.
    pub min_w: Option<f32>,
    /// At most this wide.
    pub max_w: Option<f32>,
    /// At least this tall.
    pub min_h: Option<f32>,
    /// At most this tall.
    pub max_h: Option<f32>,
}
impl Query {
    /// A query that always holds, once its container is laid out.
    pub fn new() -> Self {
        Self::default()
    }
    /// At least `width` wide (CSS `min-width`).
    pub fn min_w(mut self, width: f32) -> Self {
        self.min_w = Some(width);
        self
    }
    /// At most `width` wide (CSS `max-width`).
    pub fn max_w(mut self, width: f32) -> Self {
        self.max_w = Some(width);
        self
    }
    /// At least `height` tall.
    pub fn min_h(mut self, height: f32) -> Self {
        self.min_h = Some(height);
        self
    }
    /// At most `height` tall.
    pub fn max_h(mut self, height: f32) -> Self {
        self.max_h = Some(height);
        self
    }
    /// Whether it holds for a container `size` (NaN, not laid out: no).
    pub fn holds(&self, (width, height): (f32, f32)) -> bool {
        !width.is_nan()
            && self.min_w.is_none_or(|min| width >= min)
            && self.max_w.is_none_or(|max| width <= max)
            && self.min_h.is_none_or(|min| height >= min)
            && self.max_h.is_none_or(|max| height <= max)
    }
    /// Where it changes: on which axis (`true`, height), at what size, and
    /// whether at the size itself (a minimum) or just past it (a maximum).
    fn edges(&self) -> impl Iterator<Item = Edge> {
        [
            self.min_w.map(|at| Edge {
                tall: false,
                at,
                past: false,
            }),
            self.max_w.map(|at| Edge {
                tall: false,
                at,
                past: true,
            }),
            self.min_h.map(|at| Edge {
                tall: true,
                at,
                past: false,
            }),
            self.max_h.map(|at| Edge {
                tall: true,
                at,
                past: true,
            }),
        ]
        .into_iter()
        .flatten()
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Edge {
    tall: bool,
    at: f32,
    past: bool,
}
impl Edge {
    fn crossed(&self, (width, height): (f32, f32)) -> bool {
        let size = if self.tall { height } else { width };
        if self.past {
            size > self.at
        } else {
            size >= self.at
        }
    }
}

/// A box views' queries read, its content size published as it crosses
/// their edges.
pub(crate) struct Container {
    size: Signal<Size>,
    edges: Vec<Edge>,
    /// Which edges its size was past when last published; `None`, never.
    key: Option<Vec<bool>>,
    /// Whether a debug build said it has no width.
    warned: bool,
}

/// Mark `node` a container: its descendants' queries read it.
pub(crate) fn register(ui: &mut Ui, node: NodeId) {
    let size = ui.signal((f32::NAN, f32::NAN));
    ui.storage.containers.borrow_mut().insert(
        node,
        Container {
            size,
            edges: Vec::new(),
            key: None,
            warned: false,
        },
    );
    ui.scene.borrow_mut().set_contained(node, true);
    // Told after each layout that changes, as layout listeners are: a
    // frame in an app without containers runs nothing for them.
    let weak = ui.downgrade();
    ui.on_layout(
        node,
        Box::new(move || {
            if let Some(ui) = weak.upgrade() {
                publish(&ui, node);
            }
        }),
    );
    let storage = Rc::downgrade(&ui.storage);
    ui.retain(node, Registered { storage, node });
}
/// Forgets a container with its node.
struct Registered {
    storage: std::rc::Weak<crate::widgets::UiStorage>,
    node: NodeId,
}
impl Drop for Registered {
    fn drop(&mut self) {
        if let Some(storage) = self.storage.upgrade()
            && let Ok(mut containers) = storage.containers.try_borrow_mut()
        {
            containers.remove(&self.node);
        }
    }
}

/// The size of the container nearest above `node`, for `query`: its edges
/// published from the next layout on. `None` without one.
pub(crate) fn link(ui: &Ui, node: NodeId, query: Query) -> Option<Signal<Size>> {
    let scene = ui.scene.borrow();
    let mut containers = ui.storage.containers.borrow_mut();
    let mut at = scene.parent(node);
    while let Some(ancestor) = at {
        if let Some(container) = containers.get_mut(&ancestor) {
            for edge in query.edges() {
                if !container.edges.contains(&edge) {
                    container.edges.push(edge);
                    container.key = None;
                }
            }
            return Some(container.size.clone());
        }
        at = scene.parent(ancestor);
    }
    None
}

/// After a layout: `node`'s content size, told to the queries reading it
/// if it crossed one of their edges.
#[cold]
#[inline(never)]
fn publish(ui: &Ui, node: NodeId) {
    let (told, unsized_) = {
        let scene = ui.scene.borrow();
        let mut containers = ui.storage.containers.borrow_mut();
        let Some(container) = containers.get_mut(&node) else {
            return;
        };
        if !scene.contains(node) {
            return;
        }
        let size = content_size(&scene, node);
        // Sized by its content, it is as wide as nothing: surprising.
        let unsized_ = cfg!(debug_assertions)
            && !container.warned
            && size.0 == 0.
            && !scene.children(node).is_empty();
        container.warned |= unsized_;
        let key: Vec<bool> = container.edges.iter().map(|e| e.crossed(size)).collect();
        let told = (container.key.as_ref() != Some(&key)).then(|| {
            container.key = Some(key);
            (container.size.clone(), size)
        });
        (told, unsized_)
    };
    if unsized_ {
        crate::issues::report_kind(
            &ui.runtime,
            crate::issues::IssueKind::Degraded,
            &format!(
                "container @{} is 0 wide: a container's content doesn't size its width \
                 (CSS's inline-size containment). Give it a width, a percentage or `grow`, \
                 or let its parent stretch it (docs/reference/events.md, container queries)",
                node.number()
            ),
        );
    }
    if let Some((signal, size)) = told {
        signal.set(size);
    }
}
/// `node`'s box less its padding, as CSS queries a container's content box.
fn content_size(scene: &crate::scene::Scene, node: NodeId) -> (f32, f32) {
    let bounds = scene.bounds(node);
    let style = scene.style(node);
    let padding = style
        .padding_edges
        .unwrap_or(crate::scene::Insets::all(style.padding));
    (
        (bounds.width - padding.left - padding.right).max(0.),
        (bounds.height - padding.top - padding.bottom).max(0.),
    )
}

/// `View::when_container`'s link: the container's size once mounted.
pub(crate) type Link = Rc<RefCell<Option<Signal<Size>>>>;
