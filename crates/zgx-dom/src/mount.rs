//! Mounting views as DOM, and keeping each node's dynamic parts current.
//!
//! A mounted subtree is a [`Block`] (as Ripple's blocks): it owns its nodes,
//! effects, handlers and the regions inside it, and goes as one: removing a
//! keyed row detaches its root node and drops its block, with no walk over
//! its descendants.

use crate::{
    js,
    keyed::longest_increasing,
    ops::{self, Ops, Tag},
    sheet, template,
};
use std::{any::Any, cell::RefCell, collections::HashMap, rc::Rc};
use wasm_bindgen::prelude::*;
use zgx::{
    compose::{
        __dom::{
            self, Element, ElementKind, KeyedRegion, Region, Scope, SwitchRegion, VirtualRegion,
        },
        View,
    },
    input::Modifiers,
    reactive::{Effect, Runtime, Signal},
};

/// What the JS side reports, for a handler.
pub(crate) enum Event {
    Click(Modifiers),
    Text(String),
    Number(f64),
    Check(bool),
    Scroll(f64),
    /// Width, height, and where it lies in the viewport.
    Resize(f32, f32, f32, f32),
    Visible(bool),
}

/// What runs for a node's events. A click handler is the view's own closure
/// with the runtime to batch it in: no closure around it, so a row's two
/// clicks cost two allocations, not four.
enum Handler {
    Event(Box<dyn FnMut(Event)>),
    Click(Runtime, Box<dyn FnMut(Modifiers)>),
}
impl Handler {
    fn call(&mut self, event: Event) {
        match self {
            Handler::Event(f) => f(event),
            Handler::Click(runtime, click) => {
                if let Event::Click(modifiers) = event {
                    runtime.batch(|| click(modifiers));
                }
            }
        }
    }
}

enum Slot {
    Empty,
    /// Running: put back unless removed meanwhile.
    Taken,
    Full(Handler),
}

#[derive(Default)]
struct Handlers {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

thread_local! {
    static HANDLERS: RefCell<Handlers> = RefCell::default();
    static STARTED: RefCell<Option<Started>> = const { RefCell::new(None) };
}

/// The closures the JS side calls, kept for the page's life.
struct Started {
    _dispatch: Closure<dyn FnMut(u32, u32, f64, Option<String>)>,
    _flush: Closure<dyn FnMut()>,
}

fn handler(f: impl FnMut(Event) + 'static) -> u32 {
    register(Handler::Event(Box::new(f)))
}

fn register(handler: Handler) -> u32 {
    HANDLERS.with(|handlers| {
        let mut handlers = handlers.borrow_mut();
        let slot = Slot::Full(handler);
        match handlers.free.pop() {
            Some(id) => {
                handlers.slots[id as usize] = slot;
                id
            }
            None => {
                handlers.slots.push(slot);
                handlers.slots.len() as u32 - 1
            }
        }
    })
}

fn release_handlers(ids: &[u32]) {
    if ids.is_empty() {
        return;
    }
    // Dropped outside the borrow: a handler's captures may drop blocks.
    let mut dropped = Vec::with_capacity(ids.len());
    HANDLERS.with(|handlers| {
        let mut handlers = handlers.borrow_mut();
        for &id in ids {
            let slot = std::mem::replace(&mut handlers.slots[id as usize], Slot::Empty);
            if let Slot::Full(handler) = slot {
                dropped.push(handler);
            }
            handlers.free.push(id);
        }
        // Mostly free (a big list cleared): the free slots at the end go,
        // so a list's peak doesn't stay held as slots.
        if handlers.free.len() > 1024 && handlers.free.len() * 2 > handlers.slots.len() {
            while matches!(handlers.slots.last(), Some(Slot::Empty)) {
                handlers.slots.pop();
            }
            let len = handlers.slots.len() as u32;
            handlers.free.retain(|&id| id < len);
            handlers.slots.shrink_to_fit();
            handlers.free.shrink_to_fit();
        }
    });
    drop(dropped);
}

fn dispatch(id: u32, kind: u32, number: f64, text: Option<String>) {
    let taken = HANDLERS.with(|handlers| {
        let mut handlers = handlers.borrow_mut();
        match handlers.slots.get_mut(id as usize) {
            Some(slot @ Slot::Full(_)) => match std::mem::replace(slot, Slot::Taken) {
                Slot::Full(handler) => Some(handler),
                _ => None,
            },
            _ => None,
        }
    });
    let Some(mut handler) = taken else {
        return;
    };
    let event = match kind {
        0 => Event::Click(modifiers(number as u32)),
        1 => Event::Text(text.unwrap_or_default()),
        2 => Event::Number(number),
        3 => Event::Check(number == 1.),
        4 => Event::Scroll(number),
        5 => {
            let mut parts = text
                .as_deref()
                .unwrap_or("")
                .split(',')
                .map(|p| p.parse::<f32>().unwrap_or(0.));
            let (h, x, y) = (
                parts.next().unwrap_or(0.),
                parts.next().unwrap_or(0.),
                parts.next().unwrap_or(0.),
            );
            Event::Resize(number as f32, h, x, y)
        }
        _ => Event::Visible(number == 1.),
    };
    handler.call(event);
    let dropped = HANDLERS.with(|handlers| {
        let mut handlers = handlers.borrow_mut();
        let slot = &mut handlers.slots[id as usize];
        match slot {
            Slot::Taken => {
                *slot = Slot::Full(handler);
                None
            }
            // Removed while it ran.
            _ => Some(handler),
        }
    });
    drop(dropped);
    // What input wrote shows before the browser reads or paints; scroll
    // reads and observers' reports, delivered many at once, are applied
    // together in this task's one flush (scheduled already).
    if kind <= 3 {
        flush();
    }
}

fn modifiers(bits: u32) -> Modifiers {
    Modifiers {
        shift: bits & 1 != 0,
        control: bits & 2 != 0,
        alt: bits & 4 != 0,
        meta: bits & 8 != 0,
    }
}

fn start() {
    STARTED.with(|started| {
        if started.borrow().is_some() {
            return;
        }
        let dispatch = Closure::<dyn FnMut(u32, u32, f64, Option<String>)>::new(dispatch);
        let flush = Closure::<dyn FnMut()>::new(flush);
        js::zgx_init(sheet::BASE_CSS, &dispatch, &flush);
        *started.borrow_mut() = Some(Started {
            _dispatch: dispatch,
            _flush: flush,
        });
    });
}

/// Apply the DOM operations written since the last flush, now (they are
/// applied at the end of the current task otherwise).
pub fn flush() {
    ops::OPS.with(|ops| {
        let mut ops = ops.borrow_mut();
        ops.seal();
        ops.clear();
    });
}

/// How many nodes the JS side's table holds (for tests and benchmarks).
pub fn live_nodes() -> u32 {
    js::zgx_live_nodes()
}

/// A mounted subtree: its nodes, effects, handlers and regions, dropped as
/// one.
#[derive(Default)]
pub(crate) struct Block {
    root: u32,
    ids: Vec<u32>,
    handlers: Vec<u32>,
    effects: Vec<Effect>,
    resources: __dom::Resources,
    regions: Vec<Rc<dyn Any>>,
}
impl Drop for Block {
    fn drop(&mut self) {
        // Effects first, so none runs over nodes being freed.
        self.effects.clear();
        release_handlers(&self.handlers);
        self.regions.clear();
        // Last made first: the node table shrinks from its top.
        self.ids.reverse();
        ops::with(|ops| ops.free(&self.ids));
    }
}
impl Block {
    fn holding(resources: __dom::Resources) -> Block {
        let mut block = Block::default();
        block.resources = resources;
        block
    }
    fn node(&mut self, ops: &mut Ops) -> u32 {
        let id = ops.id();
        self.ids.push(id);
        id
    }
    fn effect(&mut self, runtime: &Runtime, f: impl FnMut() + 'static) {
        self.effects.push(runtime.effect(f));
    }
    fn on(&mut self, node: u32, kind: u32, f: impl FnMut(Event) + 'static) {
        let id = handler(f);
        self.handlers.push(id);
        ops::with(|ops| ops.op(&[ops::HANDLER, node, kind, id]));
    }
    fn on_click(&mut self, node: u32, runtime: Runtime, click: Box<dyn FnMut(Modifiers)>) {
        let id = register(Handler::Click(runtime, click));
        self.handlers.push(id);
        ops::with(|ops| ops.op(&[ops::HANDLER, node, ops::ON_CLICK, id]));
    }
}

/// A view mounted into the DOM: unmount it to remove it.
pub struct Mounted {
    block: Option<Block>,
    /// The element it mounted into, adopted into the node table.
    parent: u32,
}
impl Mounted {
    /// Remove the view, its nodes and everything it holds.
    pub fn unmount(mut self) {
        self.remove();
    }
    fn remove(&mut self) {
        if let Some(block) = self.block.take() {
            ops::with(|ops| ops.op(&[ops::REMOVE, block.root]));
            drop(block);
            // The element it mounted into, no longer held.
            ops::with(|ops| ops.free(&[self.parent]));
            flush();
        }
    }
}

/// Mount `view` as the last child of `parent`, with the theme's text colour,
/// size and family, and apply its DOM at once.
pub fn mount(runtime: &Runtime, parent: &web_sys::Element, view: View) -> Mounted {
    start();
    let scope = Scope::root(runtime);
    // Adopting writes the table at once, so what is pending (a free of the
    // id it reuses, say) goes first.
    flush();
    let parent_id = ops::with(|ops| ops.id());
    js::zgx_adopt(parent_id, parent);
    let block = runtime.batch(|| {
        runtime.untracked(|| {
            let mut block = Block::default();
            let root = ops::with(|ops| {
                let root = block.node(ops);
                ops.op(&[ops::CREATE, root, Tag::Div as u32]);
                // Its typography inherits through it; it takes no box.
                let class = sheet::plain(ops, "zc");
                ops.op(&[ops::CLASS, root, class]);
                root
            });
            block.root = root;
            let css_scope = scope.clone();
            block.effect(runtime, move || {
                let css = css_scope.root_css();
                ops::with(|ops| ops.op_str(ops::STYLE, root, &css));
            });
            let child = mount_view(view, &scope, &mut block);
            ops::with(|ops| {
                ops.op(&[ops::APPEND, root, child]);
                ops.op(&[ops::APPEND, parent_id, root]);
            });
            block
        })
    });
    flush();
    Mounted {
        block: Some(block),
        parent: parent_id,
    }
}

/// [`mount`] into the document's body.
pub fn mount_in_body(runtime: &Runtime, view: View) -> Mounted {
    let body = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.body())
        .expect("a document with a body");
    mount(runtime, &body, view)
}

/// Whether an element is only its text: a bare text node then, no element.
fn bare(el: &Element) -> bool {
    el.look.is_empty()
        && el.dynamic_css.is_none()
        && el.attrs.is_empty()
        && el.bound_attrs.is_empty()
        && el.click.is_none()
        && el.disabled.is_none()
        && el.children.is_empty()
        && el.conditional.is_empty()
        && el.on_mount.is_empty()
}

/// Mount `view` into `block`, returning its root node (not yet placed).
pub(crate) fn mount_view(view: View, scope: &Scope, block: &mut Block) -> u32 {
    mount_placed(view, scope, block).0
}

/// [`mount_view`], and whether the view positions out of flow (`absolute`).
fn mount_placed(view: View, scope: &Scope, block: &mut Block) -> (u32, bool) {
    let mut el = __dom::element(view, scope);
    block.resources.append(&mut el.resources);
    let own_scope = el.scope.take();
    let scope = own_scope.as_ref().unwrap_or(scope);
    let runtime = scope.runtime().clone();
    // Bare text: one text node, as in a template.
    if bare(&el) {
        match &mut el.kind {
            ElementKind::Text(text) => {
                return ops::with(|ops| {
                    let id = block.node(ops);
                    ops.op_str(ops::TEXT, id, text);
                    (id, false)
                });
            }
            ElementKind::BoundText(_) => {
                let ElementKind::BoundText(mut text) =
                    std::mem::replace(&mut el.kind, ElementKind::Text(String::new()))
                else {
                    unreachable!()
                };
                let id = ops::with(|ops| {
                    let id = block.node(ops);
                    ops.op_str(ops::TEXT, id, "");
                    id
                });
                block.effect(&runtime, move || {
                    let value = text();
                    ops::with(|ops| ops.op_str(ops::SET_TEXT, id, &value));
                });
                return (id, false);
            }
            _ => {}
        }
    }
    let (tag, base) = match &el.kind {
        ElementKind::Text(_) | ElementKind::BoundText(_) => (Tag::Span, "zt"),
        ElementKind::Button => (Tag::Button, "z zb"),
        ElementKind::Editor {
            multiline: true, ..
        } => (Tag::TextArea, "z ze"),
        ElementKind::Editor { .. } | ElementKind::Slider { .. } => (Tag::Input, "z ze"),
        ElementKind::Checkbox { .. } => (Tag::Label, "z"),
        ElementKind::Progress { .. } => (Tag::Progress, "z"),
        ElementKind::Image { .. } => (Tag::Canvas, "z"),
        ElementKind::Unsupported(_) => (Tag::Div, "z zu"),
        _ => (Tag::Div, "z"),
    };
    let base = if el.contents { "z zc" } else { base };
    let look = std::mem::replace(&mut el.look, __dom::Look::none());
    let absolute = look.absolute();
    let id = ops::with(|ops| {
        let id = block.node(ops);
        ops.op(&[ops::CREATE, id, tag as u32]);
        let class = sheet::class(ops, base, look);
        ops.op(&[ops::CLASS, id, class]);
        if tag == Tag::Button {
            ops.op_str(ops::ATTR, id, "type");
            ops.str("button");
        }
        for (name, value) in &el.attrs {
            if *name == "data-selectable" {
                // A class: an attribute selector is tried on every element.
                let class = sheet::plain(ops, if value == "1" { "zs1" } else { "zs0" });
                ops.op(&[ops::CLASS_ADD, id, class]);
                continue;
            }
            ops.op_str(ops::ATTR, id, name);
            ops.str(value);
        }
        id
    });
    if let Some(mut css) = el.dynamic_css.take() {
        // Inline, over the class; written only when it changes (an empty
        // patch, the commonest, is never written).
        let mut last = String::new();
        block.effect(&runtime, move || {
            let css = css();
            if css != last {
                ops::with(|ops| ops.op_str(ops::STYLE, id, &css));
                last = css;
            }
        });
    }
    for (name, mut value) in std::mem::take(&mut el.bound_attrs) {
        block.effect(&runtime, move || {
            let value = value();
            ops::with(|ops| match value {
                Some(value) => {
                    ops.op_str(ops::ATTR, id, name);
                    ops.str(&value);
                }
                None => ops.op_str(ops::REMOVE_ATTR, id, name),
            });
        });
    }
    if let Some(mut disabled) = el.disabled.take() {
        let native = matches!(tag, Tag::Button | Tag::Input | Tag::TextArea);
        block.effect(&runtime, move || {
            let on = disabled();
            ops::with(|ops| {
                if native {
                    ops.op_str(ops::PROP, id, "disabled");
                    ops.words.push(on as u32);
                } else if on {
                    ops.op_str(ops::ATTR, id, "aria-disabled");
                    ops.str("true");
                } else {
                    ops.op_str(ops::REMOVE_ATTR, id, "aria-disabled");
                }
            });
        });
    }
    if let Some(click) = el.click.take() {
        block.on_click(id, runtime.clone(), click);
    }
    let kind = std::mem::replace(&mut el.kind, ElementKind::Text(String::new()));
    mount_kind(kind, id, scope, &runtime, block);
    let mut holds_absolute = false;
    for child in std::mem::take(&mut el.children) {
        let (child, absolute) = mount_placed(child, scope, block);
        holds_absolute |= absolute;
        ops::with(|ops| ops.op(&[ops::APPEND, id, child]));
    }
    if holds_absolute {
        // Its absolute children position against it, as in zgx (CSS's
        // would go past it to a positioned ancestor).
        ops::with(|ops| {
            let class = sheet::plain(ops, "zr");
            ops.op(&[ops::CLASS_ADD, id, class]);
        });
    }
    for region in std::mem::take(&mut el.conditional) {
        let holder = ops::with(|ops| {
            let holder = block.node(ops);
            ops.op(&[ops::CREATE, holder, Tag::Div as u32]);
            let class = sheet::plain(ops, "z zc");
            ops.op(&[ops::CLASS, holder, class]);
            ops.op(&[ops::APPEND, id, holder]);
            holder
        });
        mount_region(region, holder, scope, block);
    }
    let on_mount = std::mem::take(&mut el.on_mount);
    if !on_mount.is_empty() {
        let hooks = Rc::new(NodeHooks {
            node: id,
            runtime: runtime.clone(),
            state: RefCell::new(HookState {
                alive: true,
                ..HookState::default()
            }),
        });
        block.regions.push(Rc::new(HooksOwner(hooks.clone())));
        for f in on_mount {
            f(__dom::node_handle(id, hooks.clone()));
        }
    }
    (id, absolute)
}

/// A mounted node's measurements, for `NodeHandle`'s observers: a
/// ResizeObserver's sizes and an IntersectionObserver's visibility, each
/// asked for at most once, reported into signals.
struct NodeHooks {
    node: u32,
    runtime: Runtime,
    state: RefCell<HookState>,
}

/// A node's size, and its bounds in the viewport.
type Sizes = (Signal<(f32, f32)>, Signal<zgx::scene::Rect>);

#[derive(Default)]
struct HookState {
    alive: bool,
    size: Option<Sizes>,
    visible: Vec<(f32, Signal<bool>)>,
    handlers: Vec<u32>,
    retained: Vec<Box<dyn Any>>,
}

impl NodeHooks {
    fn sizes(&self) -> Option<Sizes> {
        let mut state = self.state.borrow_mut();
        if !state.alive {
            return None;
        }
        if let Some(sizes) = &state.size {
            return Some(sizes.clone());
        }
        let size = self.runtime.signal((0_f32, 0_f32));
        let bounds = self.runtime.signal(zgx::scene::Rect {
            x: 0.,
            y: 0.,
            width: 0.,
            height: 0.,
        });
        let (write_size, write_bounds) = (size.clone(), bounds.clone());
        let id = handler(move |event| {
            if let Event::Resize(width, height, x, y) = event {
                write_size.set((width, height));
                write_bounds.set(zgx::scene::Rect {
                    x,
                    y,
                    width,
                    height,
                });
            }
        });
        state.handlers.push(id);
        ops::with(|ops| ops.op(&[ops::HANDLER, self.node, ops::ON_RESIZE, id]));
        state.size = Some((size.clone(), bounds.clone()));
        Some((size, bounds))
    }
}

impl __dom::Hooks for NodeHooks {
    fn observe_size(&self) -> Option<Signal<(f32, f32)>> {
        self.sizes().map(|(size, _)| size)
    }
    fn observe_bounds(&self) -> Option<Signal<zgx::scene::Rect>> {
        self.sizes().map(|(_, bounds)| bounds)
    }
    fn observe_visibility(&self, margin: f32) -> Option<Signal<bool>> {
        let mut state = self.state.borrow_mut();
        if !state.alive {
            return None;
        }
        if let Some((_, signal)) = state.visible.iter().find(|(m, _)| *m == margin) {
            return Some(signal.clone());
        }
        let visible = self.runtime.signal(false);
        let write = visible.clone();
        let id = handler(move |event| {
            if let Event::Visible(shown) = event {
                write.set(shown);
            }
        });
        state.handlers.push(id);
        ops::with(|ops| ops.op(&[ops::VISIBLE, self.node, id, margin.to_bits()]));
        state.visible.push((margin, visible.clone()));
        Some(visible)
    }
    fn is_mounted(&self) -> bool {
        self.state.borrow().alive
    }
    fn retain(&self, resource: Box<dyn Any>) {
        let mut state = self.state.borrow_mut();
        if state.alive {
            state.retained.push(resource);
        }
    }
}

/// Ends a node's hooks with its block: handlers released, what was
/// retained dropped.
struct HooksOwner(Rc<NodeHooks>);
impl Drop for HooksOwner {
    fn drop(&mut self) {
        let state = std::mem::take(&mut *self.0.state.borrow_mut());
        release_handlers(&state.handlers);
        drop(state);
    }
}

fn mount_kind(kind: ElementKind, id: u32, scope: &Scope, runtime: &Runtime, block: &mut Block) {
    match kind {
        ElementKind::Text(text) => ops::with(|ops| {
            let node = block.node(ops);
            ops.op_str(ops::TEXT, node, &text);
            ops.op(&[ops::APPEND, id, node]);
        }),
        ElementKind::BoundText(mut text) => {
            let node = ops::with(|ops| {
                let node = block.node(ops);
                ops.op_str(ops::TEXT, node, "");
                ops.op(&[ops::APPEND, id, node]);
                node
            });
            block.effect(runtime, move || {
                let value = text();
                ops::with(|ops| ops.op_str(ops::SET_TEXT, node, &value));
            });
        }
        ElementKind::Scroll { offset, horizontal } => {
            let (prop, kind) = if horizontal {
                ops::with(|ops| ops.op(&[ops::HANDLER, id, ops::SCROLL_X, 0]));
                (ops::NUM_SCROLL_LEFT, ops::ON_SCROLL)
            } else {
                (ops::NUM_SCROLL_TOP, ops::ON_SCROLL)
            };
            let write = offset.clone();
            block.on(id, kind, move |event| {
                if let Event::Scroll(value) = event {
                    write.set(value as f32);
                }
            });
            block.effect(runtime, move || {
                let value = offset.get();
                ops::with(|ops| ops.op(&[ops::NUMBER, id, prop, value.to_bits()]));
            });
        }
        ElementKind::Editor {
            label,
            value,
            placeholder,
            secure,
            ..
        } => {
            ops::with(|ops| {
                ops.op_str(ops::ATTR, id, "aria-label");
                ops.str(&label);
                if secure {
                    ops.op_str(ops::ATTR, id, "type");
                    ops.str("password");
                }
            });
            if let Some(mut placeholder) = placeholder {
                block.effect(runtime, move || {
                    let text = placeholder();
                    ops::with(|ops| {
                        ops.op_str(ops::ATTR, id, "placeholder");
                        ops.str(&text);
                    });
                });
            }
            let write = value.clone();
            block.on(id, ops::ON_INPUT, move |event| {
                if let Event::Text(text) = event {
                    write.set(text);
                }
            });
            block.effect(runtime, move || {
                let text = value.get();
                ops::with(|ops| ops.op_str(ops::VALUE, id, &text));
            });
        }
        ElementKind::Checkbox {
            label,
            value,
            mixed,
        } => {
            let input = ops::with(|ops| {
                let input = block.node(ops);
                ops.op(&[ops::CREATE, input, Tag::Input as u32]);
                ops.op_str(ops::ATTR, input, "type");
                ops.str("checkbox");
                ops.op(&[ops::APPEND, id, input]);
                let text = block.node(ops);
                ops.op_str(ops::TEXT, text, &label);
                ops.op(&[ops::APPEND, id, text]);
                ops.op_str(
                    ops::STYLE,
                    id,
                    "flex-direction:row;align-items:center;gap:6px;",
                );
                input
            });
            let write = value.clone();
            block.on(input, ops::ON_INPUT, move |event| {
                if let Event::Check(on) = event {
                    write.set(on);
                }
            });
            block.effect(runtime, move || {
                let on = value.get();
                ops::with(|ops| ops.op(&[ops::CHECKED, input, on as u32]));
            });
            if let Some(mixed) = mixed {
                block.effect(runtime, move || {
                    let on = mixed.get();
                    ops::with(|ops| {
                        ops.op_str(ops::PROP, input, "indeterminate");
                        ops.words.push(on as u32);
                    });
                });
            }
        }
        ElementKind::Slider {
            label,
            value,
            min,
            max,
            step,
        } => {
            ops::with(|ops| {
                for (name, v) in [
                    ("type", "range".to_owned()),
                    ("aria-label", label),
                    ("min", min.to_string()),
                    ("max", max.to_string()),
                    (
                        "step",
                        step.map_or_else(|| "any".to_owned(), |s| s.to_string()),
                    ),
                ] {
                    ops.op_str(ops::ATTR, id, name);
                    ops.str(&v);
                }
            });
            let write = value.clone();
            block.on(id, ops::ON_INPUT, move |event| {
                if let Event::Number(v) = event {
                    write.set(v as f32);
                }
            });
            block.effect(runtime, move || {
                let v = value.get();
                ops::with(|ops| ops.op(&[ops::NUMBER, id, ops::NUM_VALUE, v.to_bits()]));
            });
        }
        ElementKind::Progress { label, value } => {
            ops::with(|ops| {
                ops.op_str(ops::ATTR, id, "max");
                ops.str("1");
                ops.op_str(ops::ATTR, id, "aria-label");
                ops.str(&label);
            });
            block.effect(runtime, move || {
                let v = value.get();
                let v = if v.is_nan() { v } else { v.clamp(0., 1.) };
                ops::with(|ops| ops.op(&[ops::NUMBER, id, ops::NUM_PROGRESS, v.to_bits()]));
            });
        }
        ElementKind::Image { label, mut source } => {
            ops::with(|ops| {
                ops.op_str(ops::ATTR, id, "role");
                ops.str("img");
                ops.op_str(ops::ATTR, id, "aria-label");
                ops.str(&label);
            });
            block.effect(runtime, move || {
                let image = source();
                let (w, h) = (image.width(), image.height());
                let pixels = image.pixels();
                if pixels.is_empty() {
                    return;
                }
                let (ptr, len) = (pixels.as_ptr() as u32, pixels.len() as u32);
                ops::with(|ops| {
                    ops.op(&[ops::IMAGE, id, w, h, ptr, len]);
                    ops.keep.push(image.clone());
                });
            });
        }
        ElementKind::Region(region) => mount_region(region, id, scope, block),
        ElementKind::Canvas(draw) => mount_canvas(draw, id, runtime, block),
        ElementKind::Unsupported(name) => ops::with(|ops| {
            let text = block.node(ops);
            ops.op_str(ops::TEXT, text, &format!("{name}: not shown in the DOM"));
            ops.op(&[ops::APPEND, id, text]);
            ops.op_str(ops::ATTR, id, "data-unsupported");
            ops.str(name);
        }),
        ElementKind::Box(_) | ElementKind::Button => {}
    }
}

/// A drawing (`canvas`): a `<canvas>` filling the element, drawn with
/// Canvas 2D at the element's size (a ResizeObserver's) and the screen's
/// pixel ratio, again as the size or what the drawing reads changes, and
/// only when the commands differ from the last drawn.
fn mount_canvas(
    mut draw: Box<dyn FnMut((f32, f32)) -> zgx::canvas::Canvas>,
    id: u32,
    runtime: &Runtime,
    block: &mut Block,
) {
    let size = runtime.signal((0_f32, 0_f32));
    let surface = ops::with(|ops| {
        let surface = block.node(ops);
        ops.op(&[ops::CREATE, surface, Tag::Canvas as u32]);
        let class = sheet::plain(ops, "zcv");
        ops.op(&[ops::CLASS, surface, class]);
        ops.op(&[ops::APPEND, id, surface]);
        // It holds the canvas, positioned over it.
        let class = sheet::plain(ops, "zr");
        ops.op(&[ops::CLASS_ADD, id, class]);
        surface
    });
    let write = size.clone();
    block.on(id, ops::ON_RESIZE, move |event| {
        if let Event::Resize(width, height, _, _) = event {
            write.set((width, height));
        }
    });
    let mut last: Option<((f32, f32), zgx::canvas::Canvas)> = None;
    block.effect(runtime, move || {
        let (width, height) = size.get();
        // Measured first: until then there's nothing to draw at.
        if width <= 0. || height <= 0. {
            return;
        }
        let drawn = draw((width, height));
        if last
            .as_ref()
            .is_some_and(|(at, was)| *at == (width, height) && *was == drawn)
        {
            return;
        }
        ops::with(|ops| {
            ops.op(&[
                ops::PAINT,
                surface,
                width.to_bits(),
                height.to_bits(),
                crate::canvas::count(&drawn),
            ]);
            crate::canvas::encode(ops, &drawn);
        });
        last = Some(((width, height), drawn));
    });
}

fn mount_region(region: Region, holder: u32, scope: &Scope, block: &mut Block) {
    match region {
        Region::Switch(region) => mount_switch(region, holder, scope, block),
        Region::Keyed(region) => mount_keyed(region, holder, scope, block),
        Region::Virtual(region) => mount_virtual(region, holder, scope, block),
    }
}

/// A child block: `view` mounted in a block of its own.
fn child_block(view: View, scope: &Scope, resources: __dom::Resources) -> Block {
    let mut block = Block::holding(resources);
    // Its structure captured, to be stamped from a template (`template`).
    let capturing = ops::with(|ops| {
        let capturing = !ops.capturing();
        if capturing {
            ops.capture(template::spare());
        }
        capturing
    });
    block.root = mount_view(view, scope, &mut block);
    if capturing {
        ops::with(|ops| {
            let captured = ops.captured();
            template::stamp(ops, captured, &block.ids);
        });
    }
    block
}

fn mount_switch(region: SwitchRegion, holder: u32, scope: &Scope, block: &mut Block) {
    let SwitchRegion {
        mut changed,
        mut build,
    } = region;
    let shown: Rc<RefCell<Option<Block>>> = Rc::default();
    let runtime = scope.runtime().clone();
    let (state, scope) = (shown.clone(), scope.clone());
    block.effect(&runtime.clone(), move || {
        if !changed() {
            return;
        }
        runtime.untracked(|| {
            let mut cx = scope.context();
            let view = build(&mut cx);
            let resources = __dom::finish(cx);
            if let Some(old) = state.borrow_mut().take() {
                ops::with(|ops| ops.op(&[ops::REMOVE, old.root]));
                drop(old);
            }
            if let Some(view) = view {
                let child = child_block(view, &scope, resources);
                ops::with(|ops| ops.op(&[ops::APPEND, holder, child.root]));
                *state.borrow_mut() = Some(child);
            }
        });
    });
    block.regions.push(shown);
}

#[derive(Default)]
struct Keyed {
    order: Vec<u32>,
    rows: HashMap<u32, Block>,
    next: Vec<u32>,
    sources: Vec<u32>,
    keep: Vec<bool>,
    places: HashMap<u32, u32>,
}

fn mount_keyed(region: KeyedRegion, holder: u32, scope: &Scope, block: &mut Block) {
    let KeyedRegion { mut keys, build } = region;
    let state: Rc<RefCell<Keyed>> = Rc::default();
    let build = Rc::new(RefCell::new(build));
    let runtime = scope.runtime().clone();
    let (keyed, scope) = (state.clone(), scope.clone());
    block.effect(&runtime.clone(), move || {
        let mut guard = keyed.borrow_mut();
        let state = &mut *guard;
        keys(&mut state.next);
        runtime.untracked(|| {
            let mut row = |id: u32| {
                let mut cx = scope.context();
                let view = (build.borrow_mut())(id, &mut cx);
                let resources = __dom::finish(cx);
                child_block(view, &scope, resources)
            };
            reconcile(state, holder, &mut row);
        });
    });
    block.regions.push(state);
}

/// Bring `holder`'s rows from `state.order` to `state.next`, moving the
/// fewest.
fn reconcile(state: &mut Keyed, holder: u32, row: &mut dyn FnMut(u32) -> Block) {
    let Keyed {
        order,
        rows,
        next,
        sources,
        keep,
        places,
    } = state;
    if next.is_empty() {
        if !order.is_empty() {
            // Clear: one operation, applied before the rows' blocks are
            // dropped whole (measured faster: the browser lets go of the
            // nodes first).
            ops::with(|ops| {
                ops.op(&[ops::CLEAR, holder]);
                if !ops.capturing() {
                    ops.apply();
                }
            });
            // Newest first, so their nodes come off the top of the table;
            // what the list's peak took goes with it.
            for id in order.iter().rev() {
                rows.remove(id);
            }
            *rows = HashMap::new();
            *order = Vec::new();
            *sources = Vec::new();
            *keep = Vec::new();
            *places = HashMap::new();
        }
        next.shrink_to_fit();
        return;
    }
    if order.is_empty() {
        // Its room made once: a table doubling as it fills holds both
        // sizes for a moment, which a list's peak memory would keep.
        rows.reserve(next.len());
        for &id in next.iter() {
            let block = row(id);
            ops::with(|ops| ops.op(&[ops::APPEND, holder, block.root]));
            rows.insert(id, block);
        }
        std::mem::swap(order, next);
        return;
    }
    // The common start and end stay.
    let (mut start, mut old_end, mut new_end) = (0, order.len(), next.len());
    while start < old_end && start < new_end && order[start] == next[start] {
        start += 1;
    }
    while old_end > start && new_end > start && order[old_end - 1] == next[new_end - 1] {
        old_end -= 1;
        new_end -= 1;
    }
    // What comes after the changed part: the first row of the common end.
    let after =
        |rows: &HashMap<u32, Block>, next: &[u32], at: usize| next.get(at).map(|id| rows[id].root);
    if start == old_end {
        // Only added rows.
        let before = after(rows, next, new_end);
        for &id in &next[start..new_end] {
            let block = row(id);
            place(holder, block.root, before);
            rows.insert(id, block);
        }
    } else if start == new_end {
        // Only removed rows.
        for id in &order[start..old_end] {
            if let Some(block) = rows.remove(id) {
                ops::with(|ops| ops.op(&[ops::REMOVE, block.root]));
            }
        }
    } else {
        places.clear();
        for (at, &id) in next[start..new_end].iter().enumerate() {
            places.insert(id, at as u32);
        }
        sources.clear();
        sources.resize(new_end - start, u32::MAX);
        for (old, id) in order[start..old_end].iter().enumerate() {
            match places.get(id) {
                Some(&at) => sources[at as usize] = old as u32,
                None => {
                    if let Some(block) = rows.remove(id) {
                        ops::with(|ops| ops.op(&[ops::REMOVE, block.root]));
                    }
                }
            }
        }
        longest_increasing(sources, keep);
        // From the end, so each row's next sibling is already in place.
        let mut before = after(rows, next, new_end);
        for at in (0..new_end - start).rev() {
            let id = next[start + at];
            if sources[at] == u32::MAX {
                let block = row(id);
                place(holder, block.root, before);
                before = Some(block.root);
                rows.insert(id, block);
            } else {
                let root = rows[&id].root;
                if !keep[at] {
                    place(holder, root, before);
                }
                before = Some(root);
            }
        }
    }
    std::mem::swap(order, next);
}

fn place(holder: u32, node: u32, before: Option<u32>) {
    ops::with(|ops| match before {
        Some(before) => ops.op(&[ops::INSERT, holder, node, before]),
        None => ops.op(&[ops::APPEND, holder, node]),
    });
}

struct VirtualRow {
    block: Block,
    holder: u32,
    index: Signal<usize>,
}

fn mount_virtual(region: VirtualRegion, holder: u32, scope: &Scope, block: &mut Block) {
    let VirtualRegion {
        offset,
        row_height,
        overscan,
        mut count,
        mut key,
        build,
    } = region;
    let runtime = scope.runtime().clone();
    let viewport = runtime.signal(0_f32);
    let space = ops::with(|ops| {
        ops.op_str(
            ops::STYLE,
            holder,
            "overflow:hidden auto;display:block;height:240px;",
        );
        let space = block.node(ops);
        ops.op(&[ops::CREATE, space, Tag::Div as u32]);
        let class = sheet::plain(ops, "z");
        ops.op(&[ops::CLASS, space, class]);
        ops.op(&[ops::APPEND, holder, space]);
        space
    });
    let (write_offset, write_viewport) = (offset.clone(), viewport.clone());
    block.on(holder, ops::ON_SCROLL, move |event| {
        if let Event::Scroll(value) = event {
            write_offset.set(value as f32);
        }
    });
    block.on(holder, ops::ON_RESIZE, move |event| {
        if let Event::Resize(_, height, _, _) = event {
            write_viewport.set(height);
        }
    });
    let scroll = offset.clone();
    block.effect(&runtime, move || {
        let value = scroll.get();
        ops::with(|ops| ops.op(&[ops::NUMBER, holder, ops::NUM_SCROLL_TOP, value.to_bits()]));
    });
    let rows: Rc<RefCell<HashMap<u32, VirtualRow>>> = Rc::default();
    let build = Rc::new(RefCell::new(build));
    let (state, scope) = (rows.clone(), scope.clone());
    let mut wanted: Vec<(usize, u32)> = Vec::new();
    let mut extent = f32::NAN;
    block.effect(&runtime.clone(), move || {
        let n = count();
        let total = n as f32 * row_height;
        if total != extent {
            extent = total;
            ops::with(|ops| {
                ops.op_str(
                    ops::STYLE,
                    space,
                    &format!("height:{total}px;flex-shrink:0;"),
                )
            });
        }
        let (top, height) = (offset.get().max(0.), viewport.get().max(row_height));
        let first = ((top / row_height) as usize).saturating_sub(overscan);
        let last = (((top + height) / row_height).ceil() as usize + overscan).min(n);
        wanted.clear();
        for index in first.min(last)..last {
            wanted.push((index, key(index)));
        }
        runtime.untracked(|| {
            let mut rows = state.borrow_mut();
            let keep: std::collections::HashSet<u32> = wanted.iter().map(|(_, id)| *id).collect();
            rows.retain(|id, row| {
                let kept = keep.contains(id);
                if !kept {
                    ops::with(|ops| ops.op(&[ops::REMOVE, row.holder]));
                    let _ = &row.block;
                }
                kept
            });
            for &(index, id) in &wanted {
                let top = index as f32 * row_height;
                if let Some(row) = rows.get(&id) {
                    if row.index.with_untracked(|i| *i) != index {
                        row.index.set(index);
                        ops::with(|ops| {
                            ops.op_str(
                                ops::STYLE,
                                row.holder,
                                &format!("top:{top}px;height:{row_height}px;"),
                            )
                        });
                    }
                    continue;
                }
                let index_signal = runtime.signal(index);
                let mut cx = scope.context();
                let view = (build.borrow_mut())(index_signal.clone(), id, &mut cx);
                let resources = __dom::finish(cx);
                let mut row_block = Block::holding(resources);
                let row_holder = ops::with(|ops| {
                    let row_holder = row_block.node(ops);
                    ops.op(&[ops::CREATE, row_holder, Tag::Div as u32]);
                    let class = sheet::plain(ops, "z zv");
                    ops.op(&[ops::CLASS, row_holder, class]);
                    ops.op_str(
                        ops::STYLE,
                        row_holder,
                        &format!("top:{top}px;height:{row_height}px;"),
                    );
                    row_holder
                });
                let root = mount_view(view, &scope, &mut row_block);
                row_block.root = row_holder;
                ops::with(|ops| {
                    ops.op(&[ops::APPEND, row_holder, root]);
                    ops.op(&[ops::APPEND, space, row_holder]);
                });
                rows.insert(
                    id,
                    VirtualRow {
                        block: row_block,
                        holder: row_holder,
                        index: index_signal,
                    },
                );
            }
        });
    });
    block.regions.push(rows);
}
