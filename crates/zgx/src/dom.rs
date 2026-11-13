//! Plumbing for `zgx-dom`, which mounts views as browser DOM: what a view
//! holds, read out for a host that lays out and paints with the browser,
//! and its styles as CSS. Built only with the `dom` feature, and hidden: it
//! is not API, and changes with zgx-dom.
#![allow(missing_docs)]

use super::{Context, Environment, Kind, Rare, Slot, View};
use crate::{
    reactive::{Runtime, Signal},
    scene::{Align, Color, Justify, Layout},
    semantics::{Live, Role},
    style::{Styled, Styles},
};
use std::{any::Any, cell::RefCell, collections::HashMap, fmt::Write, rc::Rc};

/// What a subtree mounts in: its providers and theme.
#[derive(Clone)]
pub struct Scope {
    runtime: Runtime,
    environment: Environment,
}
impl Scope {
    /// A document's root scope, as `Ui::mount` makes one.
    pub fn root(runtime: &Runtime) -> Scope {
        use crate::theme::tok;
        let theme = crate::theme::Themes::of(runtime).root().clone();
        let typography = runtime.signal(crate::compose_style::Typography {
            display: Default::default(),
            color: crate::theme::Themed::Token(tok::TEXT_PRIMARY.into()),
            size: crate::theme::Themed::Token(tok::FONT_SIZE_MD.into()),
            wrap: false,
            font: Default::default(),
            family: Some(tok::FONT_SANS.into()),
        });
        Scope {
            runtime: runtime.clone(),
            environment: Environment {
                services: crate::reactive::ServiceScope::new(),
                typography,
                registry: Default::default(),
                portal: None,
                menu: None,
                measure: Default::default(),
                theme,
            },
        }
    }
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }
    /// A component context in this scope.
    pub fn context(&self) -> Context {
        Context::new(self.runtime.clone(), self.environment.clone())
    }
    /// The document's text colour, size and family from the theme, as CSS
    /// (subscribing the running effect to their tokens).
    pub fn root_css(&self) -> String {
        use crate::theme::tok;
        let theme = &self.environment.theme;
        let color = crate::compose_style::resolve_color_in(
            crate::theme::Themed::Token(tok::TEXT_PRIMARY.into()),
            Some(theme),
        );
        let size = crate::compose_style::resolve_size_in(
            crate::theme::Themed::Token(tok::FONT_SIZE_MD.into()),
            Some(theme),
        );
        let mut css = String::new();
        color_decl(&mut css, "color", color);
        let _ = write!(css, "font-size:{}px;", num(size));
        css.push_str(
            "font-family:system-ui,-apple-system,'Segoe UI',sans-serif;white-space:pre;line-height:normal;",
        );
        css
    }
}

/// What a DOM host measures of a node it mounted, for `NodeHandle`'s
/// observers (ResizeObserver, IntersectionObserver).
pub trait Hooks {
    fn observe_size(&self) -> Option<Signal<(f32, f32)>>;
    fn observe_bounds(&self) -> Option<Signal<crate::scene::Rect>>;
    fn observe_visibility(&self, margin: f32) -> Option<Signal<bool>>;
    fn is_mounted(&self) -> bool;
    fn retain(&self, resource: Box<dyn Any>);
}

/// A handle to a DOM node, for `on_mount`: its measurements from `hooks`,
/// its id the DOM node's.
pub fn node_handle(node: u32, hooks: Rc<dyn Hooks>) -> super::NodeHandle {
    super::NodeHandle {
        ui: crate::widgets::WeakUi::none(),
        node: crate::scene::NodeId::outside(node),
        dom: Some(hooks),
    }
}

/// What a component's context kept, for the host to drop as it unmounts.
pub type Resources = Vec<Box<dyn Any>>;
/// An attribute followed: `None` removes it.
pub type BoundAttr = (&'static str, Box<dyn FnMut() -> Option<String>>);
/// Builds a switch's view, if any.
pub type BuildOne = Box<dyn FnMut(&mut Context) -> Option<View>>;
/// Writes a pass's key ids.
pub type KeysInto = Box<dyn FnMut(&mut Vec<u32>)>;
/// Builds a keyed row from its key id.
pub type BuildRow = Box<dyn FnMut(u32, &mut Context) -> View>;
/// Builds a virtualized row from its index signal and key id.
pub type BuildListRow = Box<dyn FnMut(Signal<usize>, u32, &mut Context) -> View>;

/// The resources `cx` holds (its tasks among them).
pub fn finish(mut cx: Context) -> Resources {
    if let Some(tasks) = cx.tasks.take() {
        cx.resources.push(Box::new(tasks));
    }
    std::mem::take(&mut cx.resources)
}

/// A view's own element, components and providers around it built: what
/// to create, its CSS, its handlers and its children.
pub struct Element {
    pub kind: ElementKind,
    /// Its styles that never change (none when they follow theme tokens),
    /// and its hover, pressed, focus and disabled ones: a class.
    pub look: Look,
    /// CSS followed, laid inline over `look`: a `reactive_style`'s patch,
    /// or with theme tokens the whole set.
    pub dynamic_css: Option<Box<dyn FnMut() -> String>>,
    /// Attributes set once (`role`, `aria-label`, `tabindex`…).
    pub attrs: Vec<(&'static str, String)>,
    /// Attributes followed: `None` removes one.
    pub bound_attrs: Vec<BoundAttr>,
    pub click: Option<Box<dyn FnMut(crate::input::Modifiers)>>,
    pub disabled: Option<Box<dyn FnMut() -> bool>>,
    pub children: Vec<View>,
    /// `child_when` children, after the others.
    pub conditional: Vec<Region>,
    /// `on_mount` callbacks, given a handle backed by `DomNode`.
    pub on_mount: Vec<Box<dyn FnOnce(super::NodeHandle)>>,
    /// The scope its children mount in, when not its parent's.
    pub scope: Option<Scope>,
    /// What the components around it kept.
    pub resources: Resources,
    /// Lays out as its child, in its place (CSS `display: contents`).
    pub contents: bool,
    /// What of it the DOM can't show (listeners, keymaps…), for a warning.
    pub dropped: Vec<&'static str>,
}

/// An element's static styles, compared as they are: a host keeps the
/// class of each look it has seen, and writes CSS only for a new one.
#[derive(PartialEq)]
pub struct Look {
    base: &'static str,
    text: bool,
    own: Styles,
    hover: Option<Box<Styles>>,
    active: Option<Box<Styles>>,
    focus: Option<Box<Styles>>,
    disabled: Option<Box<Styles>>,
}
impl Look {
    /// No styles at all.
    pub fn none() -> Look {
        Look {
            base: "",
            text: false,
            own: Styles::new(),
            hover: None,
            active: None,
            focus: None,
            disabled: None,
        }
    }
    /// Whether it positions out of flow (`absolute`), against its parent
    /// unless a nearer ancestor is `relative`.
    pub fn absolute(&self) -> bool {
        self.own.absolute == Some(true)
    }
    /// Whether it has nothing for a class to say.
    pub fn is_empty(&self) -> bool {
        self.base.is_empty()
            && self.own == Styles::new()
            && self.hover.is_none()
            && self.active.is_none()
            && self.focus.is_none()
            && self.disabled.is_none()
    }
    /// Its own declarations.
    pub fn css(&self) -> String {
        let mut css = self.base.to_owned();
        styles_css(&self.own, self.text, &mut css);
        css
    }
    /// Its hover, pressed, focus and disabled declarations.
    pub fn variants_css(&self) -> [Option<String>; 4] {
        let css = |styles: &Option<Box<Styles>>| {
            styles.as_ref().map(|styles| {
                let mut css = String::new();
                styles_css(styles, self.text, &mut css);
                css
            })
        };
        [
            css(&self.hover),
            css(&self.active),
            css(&self.focus),
            css(&self.disabled),
        ]
    }
}

pub enum ElementKind {
    Box(Layout),
    Text(String),
    BoundText(Box<dyn FnMut() -> String>),
    Button,
    Scroll {
        offset: Signal<f32>,
        horizontal: bool,
    },
    Checkbox {
        label: String,
        value: Signal<bool>,
        mixed: Option<Signal<bool>>,
    },
    Slider {
        label: String,
        value: Signal<f32>,
        min: f32,
        max: f32,
        step: Option<f64>,
    },
    Progress {
        label: String,
        value: Signal<f32>,
    },
    Editor {
        label: String,
        value: crate::widgets::EditorValue,
        multiline: bool,
        placeholder: Option<Box<dyn FnMut() -> String>>,
        secure: bool,
    },
    Image {
        label: String,
        source: Box<dyn FnMut() -> std::sync::Arc<crate::image::ImageData>>,
    },
    Region(Region),
    /// A modal, popover, menu, submenu or tooltip: the element is its panel.
    Overlay(Box<Overlay>),
    /// What only a GPU or native host draws: a labelled box.
    Unsupported(&'static str),
}

/// What an overlay's panel is to the browser.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum OverlayKind {
    /// A modal (`modal`): a `<dialog>` shown with `showModal`.
    Dialog = 0,
    /// An anchored popup (`popover`): an auto popover.
    Popover = 1,
    /// `menu`: an auto popover of menu items.
    Menu = 2,
    /// `submenu`: one nested in its menu's.
    Submenu = 3,
    /// `View::tooltip`: a manual popover shown on hover and keyboard focus.
    Tooltip = 4,
    /// A non-modal layer (`View::layer`): a manual popover over the page.
    Layer = 5,
}

/// An overlay (`modal`, `popover`, `menu`, `submenu`, `View::tooltip`): its
/// panel is the element, its children the panel's.
pub struct Overlay {
    pub kind: OverlayKind,
    /// Whether it shows; the browser closing it (Escape, a click outside)
    /// sets it false.
    pub open: Signal<bool>,
    /// What it is anchored to, in the flow where the overlay is.
    pub anchor: Option<View>,
    /// The scope the anchor mounts in (the panel's own holds its menu).
    pub outer: Scope,
    /// Its placement against the anchor: side (bottom, top, left, right),
    /// alignment (start, centre, end) and offset.
    pub side: u32,
    pub align: u32,
    pub offset: f32,
    /// A click on a modal's backdrop closes it.
    pub dismiss_backdrop: bool,
    /// At least as wide as its anchor.
    pub match_anchor_width: bool,
    /// Focus moves into it as it opens.
    pub autofocus: bool,
    /// A tooltip: how long the pointer rests before it shows, and whether
    /// the pointer may move into it.
    pub delay_ms: u32,
    pub interactive: bool,
    /// A modal's backdrop, as CSS.
    pub backdrop: Option<String>,
}

/// A part of the tree that changes shape: one view at a time, keyed rows or
/// a virtualized list. Kept by `switch`, `keyed` and `virtual_list` beside
/// what they mount into a scene.
pub enum Region {
    Switch(SwitchRegion),
    Keyed(KeyedRegion),
    Virtual(VirtualRegion),
}

pub struct SwitchRegion {
    /// Reads the selector (tracked): whether its value changed.
    pub changed: Box<dyn FnMut() -> bool>,
    /// Builds the view for the value `changed` saw (untracked), if any.
    pub build: BuildOne,
}

pub struct KeyedRegion {
    /// Reads the keys (tracked): each key's id, stable while the key stays.
    pub keys: KeysInto,
    /// Builds the row of a key id `keys` gave.
    pub build: BuildRow,
}

pub struct VirtualRegion {
    pub offset: Signal<f32>,
    pub row_height: f32,
    pub overscan: usize,
    pub count: Box<dyn FnMut() -> usize>,
    pub key: Box<dyn FnMut(usize) -> u32>,
    /// A row, given its index signal and key id.
    pub build: BuildListRow,
}

/// Whether regions keep only their DOM form: in the browser, where no `Ui`
/// mounts them (their scene builders, and all they reach, left out of the
/// binary). Elsewhere, with the feature on, a region keeps both.
pub(crate) const DOM_ONLY: bool = cfg!(target_arch = "wasm32");

/// A view of `region` alone, for the DOM (`DOM_ONLY`).
pub(crate) fn region_only(region: Region) -> View {
    attach(View::new(Kind::Region(Box::new(|_, _, _| {}))), region)
}

/// `view` with the DOM's form of its region.
pub(crate) fn attach(mut view: View, region: Region) -> View {
    view.rare_mut().dom = Some(Box::new(region));
    view
}

/// Ids for keys, stable while a key stays among them.
struct KeyIds<K> {
    ids: HashMap<K, u32, crate::KeyHasher>,
    keys: HashMap<u32, K, crate::KeyHasher>,
    next: u32,
}
impl<K: Eq + std::hash::Hash + Clone> KeyIds<K> {
    fn new() -> Self {
        KeyIds {
            ids: HashMap::with_hasher(crate::key_hasher()),
            keys: HashMap::with_hasher(crate::key_hasher()),
            next: 0,
        }
    }
    fn id(&mut self, key: &K) -> u32 {
        if let Some(id) = self.ids.get(key) {
            return *id;
        }
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        self.ids.insert(key.clone(), id);
        self.keys.insert(id, key.clone());
        id
    }
    /// Forget the keys not among `kept` (sorted).
    fn retain(&mut self, kept: &[u32]) {
        if self.ids.len() == kept.len() {
            return;
        }
        self.keys.retain(|id, _| kept.binary_search(id).is_ok());
        let keys = &self.keys;
        self.ids.retain(|_, id| keys.contains_key(id));
    }
}

type SharedFn<F> = Rc<RefCell<F>>;

/// `keyed_grown`'s keys and builder shared between its scene region and
/// its DOM one.
#[allow(clippy::type_complexity)]
pub(crate) fn share_keyed<K: Eq + std::hash::Hash + Clone + 'static>(
    keys: impl FnMut() -> super::Keys<K> + 'static,
    build: impl FnMut(K, &mut Context) -> View + 'static,
) -> (
    Box<dyn FnMut() -> super::Keys<K>>,
    Box<dyn FnMut(K, &mut Context) -> View>,
    Region,
) {
    let keys: SharedFn<_> = Rc::new(RefCell::new(keys));
    let build: SharedFn<_> = Rc::new(RefCell::new(build));
    let ids = Rc::new(RefCell::new(KeyIds::<K>::new()));
    let last: Rc<RefCell<Vec<K>>> = Rc::default();
    let (dom_keys, dom_build, dom_ids) = (keys.clone(), build.clone(), ids.clone());
    let mut sorted = Vec::new();
    let region = Region::Keyed(KeyedRegion {
        keys: Box::new(move |out| {
            let pass = (dom_keys.borrow_mut())();
            let mut last = last.borrow_mut();
            match pass {
                super::Keys::All(all) => *last = all,
                super::Keys::Grown { unchanged, rest } => {
                    last.truncate(unchanged);
                    last.extend(rest);
                }
            }
            let mut ids = dom_ids.borrow_mut();
            out.clear();
            out.extend(last.iter().map(|key| ids.id(key)));
            sorted.clear();
            sorted.extend_from_slice(out);
            sorted.sort_unstable();
            assert!(
                sorted.windows(2).all(|w| w[0] != w[1]),
                "keyed children have duplicate keys"
            );
            ids.retain(&sorted);
        }),
        build: Box::new(move |id, cx| {
            let key = ids.borrow().keys[&id].clone();
            (build.borrow_mut())(key, cx)
        }),
    });
    let scene_build = dom_build;
    (
        Box::new(move || (keys.borrow_mut())()),
        Box::new(move |key, cx| (scene_build.borrow_mut())(key, cx)),
        region,
    )
}

/// `switch_with`'s selector and builder shared between its scene region and
/// its DOM one.
#[allow(clippy::type_complexity)]
pub(crate) fn share_switch<K: PartialEq + Clone + 'static>(
    select: impl FnMut() -> K + 'static,
    build: impl FnMut(K, &mut Context) -> Option<View> + 'static,
) -> (
    Box<dyn FnMut() -> K>,
    Box<dyn FnMut(K, &mut Context) -> Option<View>>,
    Region,
) {
    let select: SharedFn<_> = Rc::new(RefCell::new(select));
    let build: SharedFn<_> = Rc::new(RefCell::new(build));
    let shown: Rc<RefCell<Option<K>>> = Rc::default();
    let (dom_select, dom_build, seen) = (select.clone(), build.clone(), shown.clone());
    let region = Region::Switch(SwitchRegion {
        changed: Box::new(move || {
            let key = (dom_select.borrow_mut())();
            let mut seen = seen.borrow_mut();
            if seen.as_ref() == Some(&key) {
                return false;
            }
            *seen = Some(key);
            true
        }),
        build: Box::new(move |cx| {
            let key = shown.borrow().clone()?;
            (dom_build.borrow_mut())(key, cx)
        }),
    });
    (
        Box::new(move || (select.borrow_mut())()),
        Box::new(move |key, cx| (build.borrow_mut())(key, cx)),
        region,
    )
}

/// `child_when`'s, as `share_switch` (`None` shows nothing).
#[allow(clippy::type_complexity)]
pub(crate) fn share_when<K: PartialEq + Clone + 'static>(
    select: impl FnMut() -> Option<K> + 'static,
    build: impl FnMut(K, &mut Context) -> View + 'static,
) -> (
    Box<dyn FnMut() -> Option<K>>,
    Box<dyn FnMut(K, &mut Context) -> View>,
    Region,
) {
    let build: SharedFn<_> = Rc::new(RefCell::new(build));
    let dom_build = build.clone();
    let (select, _, region) = share_switch(select, move |key: Option<K>, cx| {
        key.map(|key| (dom_build.borrow_mut())(key, cx))
    });
    (
        select,
        Box::new(move |key, cx| (build.borrow_mut())(key, cx)),
        region,
    )
}

/// `virtual_list`'s count, keys and builder shared between its scene region
/// and its DOM one.
#[allow(clippy::type_complexity)]
pub(crate) fn share_virtual<K: Eq + std::hash::Hash + Clone + 'static>(
    offset: Signal<f32>,
    row_height: f32,
    overscan: usize,
    count: impl FnMut() -> usize + 'static,
    key: impl FnMut(usize) -> K + 'static,
    build: impl FnMut(Signal<usize>, K, &mut Context) -> View + 'static,
) -> (
    Box<dyn FnMut() -> usize>,
    Box<dyn FnMut(usize) -> K>,
    Box<dyn FnMut(Signal<usize>, K, &mut Context) -> View>,
    Region,
) {
    let count: SharedFn<_> = Rc::new(RefCell::new(count));
    let key: SharedFn<_> = Rc::new(RefCell::new(key));
    let build: SharedFn<_> = Rc::new(RefCell::new(build));
    let ids = Rc::new(RefCell::new(KeyIds::<K>::new()));
    let (dom_count, dom_key, dom_build, dom_ids) =
        (count.clone(), key.clone(), build.clone(), ids.clone());
    let region = Region::Virtual(VirtualRegion {
        offset,
        row_height,
        overscan,
        count: Box::new(move || (dom_count.borrow_mut())()),
        key: Box::new(move |index| {
            let key = (dom_key.borrow_mut())(index);
            let mut ids = dom_ids.borrow_mut();
            // Ids of keys long scrolled past are kept: a list's keys are
            // few beside its rows' views.
            ids.id(&key)
        }),
        build: Box::new(move |index, id, cx| {
            let key = ids.borrow().keys[&id].clone();
            (dom_build.borrow_mut())(index, key, cx)
        }),
    });
    (
        Box::new(move || (count.borrow_mut())()),
        Box::new(move |index| (key.borrow_mut())(index)),
        Box::new(move |index, k, cx| (build.borrow_mut())(index, k, cx)),
        region,
    )
}

/// Read `view` out as an element in `scope`: components, providers and slots
/// around it built (untracked), their resources kept with it.
pub fn element(mut view: View, parent: &Scope) -> Element {
    // Cloned only when a provider or slot changes it.
    let mut scope: Option<Scope> = None;
    let mut resources = Resources::new();
    let runtime = &parent.runtime;
    loop {
        let kind = std::mem::replace(&mut view.kind, Kind::Container(Layout::Column));
        match kind {
            Kind::Component(build) => {
                let mut cx = scope.as_ref().unwrap_or(parent).context();
                let inner = runtime.untracked(|| build(&mut cx));
                resources.extend(finish(cx));
                view = view.refine(inner);
            }
            Kind::Provider(provide, child) => {
                let scope = scope.get_or_insert_with(|| parent.clone());
                scope.environment.services = scope.environment.services.child();
                let mut cx = scope.context();
                runtime.untracked(|| provide(&mut cx));
                scope.environment = cx.environment.clone();
                resources.extend(finish(cx));
                view = view.refine(*child);
            }
            Kind::Slot(Slot { build, services }) => {
                let scope = scope.get_or_insert_with(|| parent.clone());
                scope.environment.services = services;
                let mut cx = scope.context();
                let inner = runtime.untracked(|| build(&mut cx));
                resources.extend(finish(cx));
                view = view.refine(inner);
            }
            Kind::Confined(child) => {
                // A box holding what it mounts: portals have no layer here.
                view.children.push(*child);
                return primitive(
                    view,
                    ElementKind::Box(Layout::Column),
                    parent,
                    scope,
                    resources,
                );
            }
            kind => {
                let kind = match kind {
                    Kind::Container(layout) => ElementKind::Box(layout),
                    Kind::Text(text) => ElementKind::Text(text),
                    Kind::BoundText(text) => ElementKind::BoundText(text),
                    Kind::Button => ElementKind::Button,
                    Kind::Scroll { offset, horizontal } => {
                        ElementKind::Scroll { offset, horizontal }
                    }
                    Kind::Checkbox {
                        label,
                        value,
                        mixed,
                    } => ElementKind::Checkbox {
                        label,
                        value,
                        mixed,
                    },
                    Kind::Slider {
                        label,
                        value,
                        range,
                    } => ElementKind::Slider {
                        label,
                        value,
                        min: *range.start(),
                        max: *range.end(),
                        step: view.rare.as_ref().and_then(|rare| rare.slider_step),
                    },
                    Kind::Progress { label, value } => ElementKind::Progress { label, value },
                    Kind::Editor {
                        label,
                        value,
                        multiline,
                    } => ElementKind::Editor {
                        label,
                        value,
                        multiline,
                        placeholder: view.rare.as_deref_mut().and_then(|r| r.placeholder.take()),
                        secure: view
                            .rare
                            .as_ref()
                            .and_then(|rare| rare.secure)
                            .unwrap_or(false),
                    },
                    Kind::Image { label, source } => ElementKind::Image { label, source },
                    Kind::Region(_) | Kind::VirtualList(_) => {
                        match view.rare.as_deref_mut().and_then(|rare| rare.dom.take()) {
                            Some(region) => ElementKind::Region(*region),
                            None => ElementKind::Unsupported("region"),
                        }
                    }
                    Kind::RichText(_) => ElementKind::Unsupported("rich text"),
                    Kind::MenuItem(label) => {
                        return menu_item(view, label, parent, scope, resources);
                    }
                    Kind::Submenu { label, open } => {
                        return submenu(view, label, open, parent, scope, resources);
                    }
                    Kind::Menu {
                        label,
                        open,
                        anchor,
                    } => {
                        let kind = OverlayKind::Menu;
                        let anchor = Some(*anchor);
                        return overlay(view, kind, label, open, anchor, parent, scope, resources);
                    }
                    Kind::Modal {
                        label,
                        open,
                        anchor,
                    } => {
                        let kind = match anchor {
                            Some(_) => OverlayKind::Popover,
                            None => OverlayKind::Dialog,
                        };
                        let anchor = anchor.map(|anchor| *anchor);
                        return overlay(view, kind, label, open, anchor, parent, scope, resources);
                    }
                    Kind::Canvas(_) => ElementKind::Unsupported("canvas"),
                    Kind::Svg { .. } => ElementKind::Unsupported("svg"),
                    #[cfg(target_os = "macos")]
                    Kind::NativeSurface { .. } => ElementKind::Unsupported("native surface"),
                    Kind::Component(_) | Kind::Provider(..) | Kind::Slot(_) | Kind::Confined(_) => {
                        unreachable!()
                    }
                };
                return primitive(view, kind, parent, scope, resources);
            }
        }
    }
}

/// `view` (a popover) as a tooltip: the host shows it as the pointer rests
/// on its anchor for `delay`, or keyboard focus reaches the anchor.
pub(crate) fn hover(mut view: View, delay: std::time::Duration, interactive: bool) -> View {
    view.rare_mut().portal.get_or_insert_default().dom_hover = Some((delay, interactive));
    view
}

/// A menu item: a button that closes its menu (and the menus it opened
/// from) before running its click, or a submenu's trigger, which opens it.
fn menu_item(
    mut view: View,
    label: String,
    parent: &Scope,
    scope: Option<Scope>,
    resources: Resources,
) -> Element {
    use super::OVERLAY;
    use crate::theme::tok;
    super::dim_when_disabled(&mut view);
    let mut defaults = super::widget_style(|w| &w.menu_item);
    if OVERLAY.menu_item_min_height.is_some()
        && (view.styles.height.is_some() || view.styles.height_percent.is_some())
    {
        defaults = defaults.min_h(0.);
    }
    defaults.merge(&view.styles);
    view.styles = defaults;
    if view.variants.hover.is_none() {
        view.variants.defaults[0] = Some(|| super::widget_style(|w| &w.menu_hover));
    }
    if view.variants.focus.is_none() {
        view.variants.defaults[2] = Some(super::menu_focus(&view.styles));
    }
    let menu = scope.as_ref().unwrap_or(parent).environment.menu.clone();
    let (trigger, check, shortcut) = match view.rare.as_deref_mut() {
        Some(rare) => (
            rare.menu_trigger.take(),
            rare.menu_check.clone(),
            rare.shortcut.clone(),
        ),
        None => (None, None, None),
    };
    let named = !view.children.is_empty();
    if !named {
        // The check (or a radio's dot) in a gutter of its own.
        if let Some((checked, radio)) = check.clone() {
            let mark = if radio { "•" } else { "✓" };
            view.children.push(
                super::text_signal(move || {
                    if checked() {
                        mark.into()
                    } else {
                        String::new()
                    }
                })
                .w(16.)
                .shrink_0()
                .accessible_hidden(),
            );
        }
        let drawn = super::menu_text(label.clone(), view.rare.as_deref_mut());
        if trigger.is_some() || shortcut.is_some() {
            view.children.push(drawn.flex_grow(1.));
        } else {
            view.children.push(drawn);
        }
        if let Some(keys) = shortcut {
            view.children.push(
                super::text(keys)
                    .ml(OVERLAY.shortcut_gap)
                    .text_color(tok::TEXT_TERTIARY)
                    .accessible_hidden(),
            );
        }
        if trigger.is_some() {
            view.children.push(
                super::text("›")
                    .ml(OVERLAY.shortcut_gap)
                    .text_color(tok::TEXT_TERTIARY)
                    .accessible_hidden(),
            );
        }
    }
    // In a menu the arrows move among its items (one Tab stop: the menu).
    if menu.is_some() && view.focusable.is_none() {
        view.focusable = Some(false);
    }
    let mut own = view.click.take();
    view.click = Some(match trigger.clone() {
        Some(open) => Box::new(move |modifiers| {
            open.set(true);
            if let Some(click) = own.as_mut() {
                click(modifiers);
            }
        }),
        None => Box::new(move |modifiers| {
            if let Some(menu) = &menu {
                menu.close_chain();
            }
            if let Some(click) = own.as_mut() {
                click(modifiers);
            }
        }),
    });
    let mut el = primitive(view, ElementKind::Button, parent, scope, resources);
    let role = match &check {
        Some((_, true)) => "menuitemradio",
        Some(_) => "menuitemcheckbox",
        None => "menuitem",
    };
    el.attrs.insert(0, ("role", role.into()));
    if named {
        el.attrs.insert(1, ("aria-label", label));
    }
    if let Some((checked, _)) = check {
        el.bound_attrs
            .push(("aria-checked", Box::new(move || Some(bool_str(checked())))));
    }
    if let Some(open) = trigger {
        el.attrs.push(("aria-haspopup", "menu".into()));
        el.bound_attrs.push((
            "aria-expanded",
            Box::new(move || Some(bool_str(open.get()))),
        ));
    }
    el
}

/// A submenu: its trigger (the app's, or a menu item of its label) in the
/// menu it is in, its panel beside it.
fn submenu(
    mut view: View,
    label: String,
    open: Signal<bool>,
    parent: &Scope,
    scope: Option<Scope>,
    resources: Resources,
) -> Element {
    super::dim_when_disabled(&mut view);
    let trigger = (view.rare.as_mut())
        .and_then(|r| r.portal.as_mut())
        .and_then(|p| p.trigger.take());
    let mut anchor = match trigger {
        Some(trigger) => trigger,
        None => match super::followed_label(view.rare.as_deref_mut()) {
            Some(followed) => {
                super::menu_item(label.clone()).accessible_label_when(move || followed.get())
            }
            None => super::menu_item(label.clone()),
        },
    };
    anchor.rare_mut().menu_trigger = Some(open.clone());
    if let Some(id) = view.rare.as_mut().and_then(|r| r.trigger_id.take()) {
        anchor.id = Some(id);
    }
    // Disabled, its trigger is (it never opens).
    if let Some(disabled) = view.disabled.take() {
        anchor.disabled = Some(disabled);
    }
    let kind = OverlayKind::Submenu;
    overlay(
        view,
        kind,
        label,
        open,
        Some(anchor),
        parent,
        scope,
        resources,
    )
}

/// An overlay's panel, with the card a desktop panel of its kind has.
#[allow(clippy::too_many_arguments)]
fn overlay(
    mut view: View,
    mut kind: OverlayKind,
    label: String,
    open: Signal<bool>,
    mut anchor: Option<View>,
    parent: &Scope,
    scope: Option<Scope>,
    resources: Resources,
) -> Element {
    use super::{OVERLAY, PlacementEdge, PlacementSide};
    use crate::theme::tok;
    let outer = scope.unwrap_or_else(|| parent.clone());
    let (options, dismiss, match_anchor_width, hidden) = match view.rare.as_deref_mut() {
        Some(rare) => (
            rare.portal.take(),
            rare.dismiss_backdrop.take(),
            rare.match_anchor_width,
            rare.panel_hidden,
        ),
        None => (None, None, false, false),
    };
    let layer = options.as_ref().is_some_and(|o| o.layer);
    let hover = options.as_ref().and_then(|o| o.dom_hover);
    if hover.is_some() {
        kind = OverlayKind::Tooltip;
    } else if layer && anchor.is_none() {
        kind = OverlayKind::Layer;
    }
    let anchored = anchor.is_some();
    let is_menu = matches!(kind, OverlayKind::Menu | OverlayKind::Submenu);
    // A layer's panel is only what it holds; a dialog's or menu's is a card
    // (as `mount_portal_panel`).
    if !layer {
        let (padding, radius) = match (is_menu, anchored) {
            (true, _) => (OVERLAY.menu_padding, OVERLAY.menu_radius),
            (false, true) => (
                (OVERLAY.popover_padding, OVERLAY.popover_padding),
                OVERLAY.popover_radius,
            ),
            (false, false) => ((16., 16.), crate::theme::Themed::Token(tok::RADIUS_MD.0)),
        };
        let card = Styles::new().w(if anchored { 240. } else { 360. });
        let card = match padding {
            (x, y) if x == y => card.p(x),
            (x, y) => card.px(x).py(y),
        };
        let card = match (OVERLAY.panel_shadows, anchored) {
            (Some(shadows), true) => card.shadows(shadows.to_vec()),
            _ => card,
        };
        let mut defaults = card
            .bg(if is_menu {
                tok::MENU_BG
            } else if anchored {
                tok::POPOVER_BG
            } else {
                tok::SURFACE_OVERLAY
            })
            .rounded(radius)
            .overflow_hidden();
        if is_menu {
            defaults = defaults.items_stretch();
        }
        defaults.merge(&view.styles);
        view.styles = defaults;
    }
    // Focus lands on the panel only with nothing in it to take it.
    if view.focusable.is_none() && !matches!(kind, OverlayKind::Tooltip | OverlayKind::Layer) {
        view.focusable = Some(false);
    }
    // A bare button opening a menu is a menu button: a press opens it.
    if kind == OverlayKind::Menu
        && let Some(anchor) = anchor.as_mut()
        && anchor.click.is_none()
        && matches!(anchor.kind, Kind::Button)
    {
        let open = open.clone();
        anchor.click = Some(Box::new(move |_: crate::input::Modifiers| {
            open.set(true);
        }));
    }
    let mut inner = outer.clone();
    if is_menu {
        inner.environment.menu = Some(crate::compose_menu::MenuContext::new(
            open.clone(),
            outer.environment.menu.clone(),
            None,
            kind == OverlayKind::Submenu,
        ));
    }
    let placement = match options.as_ref().and_then(|o| o.placement) {
        Some(placement) => placement,
        None if kind == OverlayKind::Submenu => super::Placement {
            side: PlacementSide::Right,
            align: PlacementEdge::Start,
            offset: 0.,
        },
        None => Default::default(),
    };
    let side = match placement.side {
        PlacementSide::Bottom => 0,
        PlacementSide::Top => 1,
        PlacementSide::Left => 2,
        PlacementSide::Right => 3,
    };
    let align = match placement.align {
        PlacementEdge::Start => 0,
        PlacementEdge::Center => 1,
        PlacementEdge::End => 2,
    };
    let (delay, interactive) = hover.unwrap_or_default();
    let backdrop = (kind == OverlayKind::Dialog)
        .then(|| color_value(outer.environment.theme.color(tok::BACKDROP)));
    let overlay = Overlay {
        kind,
        open,
        anchor,
        outer,
        side,
        align,
        offset: placement.offset,
        dismiss_backdrop: dismiss.unwrap_or(kind != OverlayKind::Submenu),
        match_anchor_width,
        autofocus: options.as_ref().and_then(|o| o.autofocus).unwrap_or(true),
        delay_ms: delay.as_millis().min(u32::MAX as u128) as u32,
        interactive,
        backdrop,
    };
    let mut el = primitive(
        view,
        ElementKind::Overlay(Box::new(overlay)),
        parent,
        Some(inner),
        resources,
    );
    let role = match kind {
        OverlayKind::Popover => Some("dialog"),
        OverlayKind::Menu | OverlayKind::Submenu => Some("menu"),
        OverlayKind::Tooltip => Some("tooltip"),
        OverlayKind::Dialog | OverlayKind::Layer => None,
    };
    if !matches!(kind, OverlayKind::Tooltip | OverlayKind::Layer) {
        el.attrs.insert(0, ("aria-label", label));
    }
    if let Some(role) = role {
        el.attrs.insert(0, ("role", role.into()));
    }
    if hidden {
        el.attrs.push(("aria-hidden", "true".into()));
    }
    el
}

fn primitive(
    mut view: View,
    kind: ElementKind,
    parent: &Scope,
    own_scope: Option<Scope>,
    resources: Resources,
) -> Element {
    let scope = own_scope.as_ref().unwrap_or(parent);
    let mut dropped = Vec::new();
    if !view.events.is_empty() {
        dropped.push("on_event");
    }
    let mut attrs = Vec::new();
    let mut bound_attrs: Vec<BoundAttr> = Vec::new();
    let mut conditional = Vec::new();
    if let Some(id) = view.id.take() {
        attrs.push(("data-id", id));
    }
    match view.focusable {
        Some(true) => attrs.push(("tabindex", "0".into())),
        Some(false) => attrs.push(("tabindex", "-1".into())),
        None => {}
    }
    if let Some(rare) = view.rare.take() {
        let Rare {
            accessible_label,
            accessibility,
            dom_conditional,
            selectable,
            ..
        } = *rare;
        conditional = dom_conditional;
        if let Some(label) = accessible_label {
            attrs.push(("aria-label", label));
        }
        match selectable {
            Some(true) => attrs.push(("data-selectable", "1".into())),
            Some(false) => attrs.push(("data-selectable", "0".into())),
            None => {}
        }
        if let Some(a) = accessibility {
            let a = *a;
            if let Some(role) = a.role
                && let Some(role) = aria_role(role)
            {
                attrs.push(("role", role.into()));
            }
            if let Some(level) = a.level {
                attrs.push(("aria-level", level.to_string()));
            }
            if let Some(live) = a.live {
                attrs.push((
                    "aria-live",
                    match live {
                        Live::Polite => "polite",
                        Live::Assertive => "assertive",
                    }
                    .into(),
                ));
            }
            if let Some(description) = a.description {
                attrs.push(("aria-description", description));
            }
            if a.hidden {
                attrs.push(("aria-hidden", "true".into()));
            }
            if a.modal {
                attrs.push(("aria-modal", "true".into()));
            }
            if let Some(mut f) = a.label {
                bound_attrs.push(("aria-label", Box::new(move || Some(f()))));
            }
            if let Some(mut f) = a.checked {
                bound_attrs.push(("aria-checked", Box::new(move || Some(bool_str(f())))));
            }
            if let Some(mut f) = a.selected {
                bound_attrs.push(("aria-selected", Box::new(move || Some(bool_str(f())))));
            }
            if let Some(mut f) = a.expanded {
                bound_attrs.push(("aria-expanded", Box::new(move || Some(bool_str(f())))));
            }
            if let Some(mut f) = a.value {
                bound_attrs.push(("aria-valuetext", Box::new(move || Some(f()))));
            }
            if let Some(mut f) = a.description_when {
                bound_attrs.push(("aria-description", Box::new(move || Some(f()))));
            }
            if a.focus_group.is_some() {
                dropped.push("focus_group");
            }
        }
    }
    let text_like = matches!(kind, ElementKind::Text(_) | ElementKind::BoundText(_));
    let base = kind_css(&kind);
    let styles = std::mem::take(&mut view.styles);
    let reactive = view.reactive_style.take();
    let themed = styles.tokens.is_some();
    let theme = scope.environment.theme.clone();
    // Unthemed, the styles are a class and a reactive style writes only
    // its patch (inline, over the class, as a patch lies over the styles).
    let (own, dynamic_css): (Styles, Option<Box<dyn FnMut() -> String>>) = if !themed {
        let dynamic = reactive.map(|mut reactive| {
            let theme = theme.clone();
            Box::new(move || {
                let mut patch = None;
                reactive(&mut patch);
                let mut css = String::new();
                if let Some(mut patch) = patch {
                    crate::theme::resolve(&mut patch, &theme);
                    styles_css(&patch, text_like, &mut css);
                }
                css
            }) as Box<dyn FnMut() -> String>
        });
        (styles, dynamic)
    } else {
        let theme = theme.clone();
        let mut reactive = reactive;
        let dynamic: Box<dyn FnMut() -> String> = Box::new(move || {
            let mut resolved = styles.clone();
            if let Some(reactive) = &mut reactive {
                let mut patch = None;
                reactive(&mut patch);
                if let Some(patch) = patch {
                    resolved.merge(&patch);
                }
            }
            crate::theme::resolve(&mut resolved, &theme);
            let mut css = String::new();
            styles_css(&resolved, text_like, &mut css);
            css
        });
        (Styles::new(), Some(dynamic))
    };
    let variant = |styles: Option<Box<Styles>>, default: Option<fn() -> Styles>| {
        let mut styles = match (styles, default) {
            (Some(styles), _) => styles,
            (None, Some(default)) => Box::new(default()),
            (None, None) => return None,
        };
        if styles.tokens.is_some() {
            crate::theme::resolve(&mut styles, &theme);
        }
        Some(styles)
    };
    let variants = std::mem::take(&mut view.variants);
    if variants.reactive.is_some() {
        dropped.push("reactive_interaction");
    }
    let [dh, da, df] = variants.defaults;
    let look = Look {
        base,
        text: text_like,
        own,
        hover: variant(variants.hover, dh),
        active: variant(variants.active, da),
        focus: variant(variants.focus, df),
        disabled: variant(variants.disabled, None),
    };
    Element {
        kind,
        look,
        dynamic_css,
        attrs,
        bound_attrs,
        click: view.click.take(),
        disabled: view.disabled.take(),
        children: std::mem::take(&mut view.children),
        conditional,
        on_mount: std::mem::take(&mut view.mounted),
        scope: own_scope,
        resources,
        contents: view.contents != super::Contents::Own,
        dropped,
    }
}

fn bool_str(value: bool) -> String {
    if value { "true" } else { "false" }.into()
}

/// The ARIA role for `role`, where the element it mounts as has none.
pub fn aria_role(role: Role) -> Option<&'static str> {
    Some(match role {
        Role::Window => "application",
        Role::Group => "group",
        Role::Label => return None,
        Role::Button => "button",
        Role::Link => "link",
        Role::CheckBox => "checkbox",
        Role::Slider => "slider",
        Role::TextInput => "textbox",
        Role::MultilineTextInput => "textbox",
        Role::Progress => "progressbar",
        Role::ScrollView => return None,
        Role::ScrollBar => "scrollbar",
        Role::Splitter => "separator",
        Role::ListItem => "listitem",
        Role::Dialog => "dialog",
        Role::Menu => "menu",
        Role::MenuItem => "menuitem",
        Role::Image => "img",
        Role::Document => "document",
        Role::Heading => "heading",
        Role::List => "list",
        Role::ListBox => "listbox",
        Role::ListBoxOption => "option",
        Role::Toolbar => "toolbar",
        Role::Status => "status",
        Role::TabList => "tablist",
        Role::Tab => "tab",
        Role::TabPanel => "tabpanel",
        Role::RadioGroup => "radiogroup",
        Role::RadioButton => "radio",
        Role::Switch => "switch",
        Role::SpinButton => "spinbutton",
        Role::ComboBox => "combobox",
        Role::Tree => "tree",
        Role::TreeItem => "treeitem",
        Role::Grid => "grid",
        Role::Row => "row",
        Role::Cell => "gridcell",
        Role::ColumnHeader => "columnheader",
        Role::Alert => "alert",
        Role::Article => "article",
        #[allow(unreachable_patterns)]
        _ => return None,
    })
}

/// The CSS every element of a kind starts from, before its styles: zgx's
/// layout defaults, which are mostly CSS flexbox's. Elements also carry the
/// stylesheet's base class (`zgx-dom`'s `BASE_CSS`).
fn kind_css(kind: &ElementKind) -> &'static str {
    match kind {
        ElementKind::Box(Layout::Row) => "flex-direction:row;",
        ElementKind::Box(Layout::Column) => "",
        ElementKind::Box(Layout::Overlay) => "display:grid;",
        ElementKind::Button => "flex-direction:row;",
        ElementKind::Scroll {
            horizontal: false, ..
        } => "overflow:hidden auto;width:320px;height:240px;",
        ElementKind::Scroll {
            horizontal: true, ..
        } => "flex-direction:row;overflow:auto hidden;width:320px;height:240px;",
        ElementKind::Editor { .. } => "width:240px;",
        _ => "",
    }
}

fn num(value: f32) -> String {
    if value.fract() == 0. && value.abs() < 1e9 {
        format!("{}", value as i64)
    } else {
        format!("{:.3}", value)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

fn color_decl(css: &mut String, property: &str, Color(r, g, b, a): Color) {
    if a == 255 {
        let _ = write!(css, "{property}:#{r:02x}{g:02x}{b:02x};");
    } else {
        let _ = write!(
            css,
            "{property}:rgba({r},{g},{b},{});",
            num(a as f32 / 255.)
        );
    }
}

fn color_value(Color(r, g, b, a): Color) -> String {
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("rgba({r},{g},{b},{})", num(a as f32 / 255.))
    }
}

fn length(value: crate::layout::Length) -> String {
    match value {
        crate::layout::Length::Auto => "auto".into(),
        crate::layout::Length::Px(px) => format!("{}px", num(px)),
        crate::layout::Length::Percent(fraction) => format!("{}%", num(fraction * 100.)),
    }
}

fn align(align: Align) -> &'static str {
    match align {
        Align::Baseline => "baseline",
        Align::Start => "flex-start",
        Align::Center => "center",
        Align::End => "flex-end",
        Align::Stretch => "stretch",
    }
}

fn justify(justify: Justify) -> &'static str {
    match justify {
        Justify::Start => "flex-start",
        Justify::Center => "center",
        Justify::End => "flex-end",
        Justify::SpaceBetween => "space-between",
        Justify::SpaceAround => "space-around",
        Justify::SpaceEvenly => "space-evenly",
        #[allow(unreachable_patterns)]
        _ => "stretch",
    }
}

fn edges(css: &mut String, property: &str, insets: crate::scene::Insets, sides: u8) {
    let sides = if sides == 0 { 15 } else { sides };
    if sides == 15 {
        let _ = write!(
            css,
            "{property}:{}px {}px {}px {}px;",
            num(insets.top),
            num(insets.right),
            num(insets.bottom),
            num(insets.left)
        );
        return;
    }
    for (bit, side, value) in [
        (1, "top", insets.top),
        (2, "right", insets.right),
        (4, "bottom", insets.bottom),
        (8, "left", insets.left),
    ] {
        if sides & bit != 0 {
            let _ = write!(css, "{property}-{side}:{}px;", num(value));
        }
    }
}

/// `styles` as CSS declarations, appended to `css`.
pub fn styles_css(styles: &Styles, text: bool, css: &mut String) {
    let s: &crate::style::StylesFields = styles;
    if let Some(layout) = s.overflow().layout {
        css.push_str(match layout {
            Layout::Row => "flex-direction:row;",
            Layout::Column => "flex-direction:column;",
            Layout::Overlay => "display:grid;",
        });
    }
    if let Some(w) = s.width.get() {
        let _ = write!(css, "width:{}px;", num(w));
    }
    if let Some(h) = s.height.get() {
        let _ = write!(css, "height:{}px;", num(h));
    }
    if let Some(w) = s.width_percent.get() {
        let _ = write!(css, "width:{}%;", num(w * 100.));
    }
    if let Some(h) = s.height_percent.get() {
        let _ = write!(css, "height:{}%;", num(h * 100.));
    }
    if let Some(padding) = s.padding {
        edges(css, "padding", padding, s.padding_sides);
    }
    if let Some(margin) = s.margin {
        edges(css, "margin", margin, s.margin_sides);
    }
    if let Some(gap) = s.gap.get() {
        let _ = write!(css, "gap:{}px;", num(gap));
    }
    if let Some(clip) = s.clip {
        css.push_str(if clip {
            "overflow:hidden;"
        } else {
            "overflow:visible;"
        });
    }
    if let Some(grow) = s.flex_grow.get() {
        let _ = write!(css, "flex-grow:{};", num(grow));
    }
    if let Some(shrink) = s.flex_shrink.get() {
        let _ = write!(css, "flex-shrink:{};", num(shrink));
    }
    if let Some(a) = s.align {
        let _ = write!(css, "align-items:{};", align(a));
        if s.overflow().layout == Some(Layout::Overlay) {
            let _ = write!(css, "justify-items:{};", align(a));
        }
    }
    if let Some(j) = s.justify {
        let _ = write!(css, "justify-content:{};", justify(j));
    }
    if let Some(absolute) = s.absolute {
        css.push_str(if absolute {
            "position:absolute;"
        } else {
            "position:relative;"
        });
    }
    if let Some(wrap) = s.text_wrap {
        css.push_str(if wrap {
            "white-space:pre-wrap;overflow-wrap:anywhere;"
        } else {
            "white-space:pre;"
        });
    }
    let border_width = s.border_width.get();
    match &s.paint_background {
        Some(Some(background)) => background_css(css, background),
        Some(None) => {}
        None => {
            if let Some(bg) = s.background {
                color_decl(css, "background-color", bg);
            }
        }
    }
    if s.paint_background.is_some()
        && let Some(bg) = s.background
        && !matches!(s.paint_background, Some(Some(_)))
    {
        color_decl(css, "background-color", bg);
    }
    if let Some(cursor) = s.cursor {
        let _ = write!(css, "cursor:{};", cursor_css(cursor));
    }
    if let Some(color) = s.text_color {
        color_decl(css, "color", color);
    }
    if let Some(size) = s.text_size.get() {
        let _ = write!(css, "font-size:{}px;", num(size));
    }
    if let Some(line_height) = s.line_height {
        match line_height.pixels() {
            Some(px) => {
                let _ = write!(css, "line-height:{}px;", num(px));
            }
            None => css.push_str("line-height:normal;"),
        }
    }
    if let Some(family) = &s.font_family {
        let _ = write!(css, "font-family:{};", family_css(family));
    }
    if let Some(align) = s.text_align {
        use crate::text_layout::TextAlign;
        css.push_str(match align {
            TextAlign::Start => "text-align:start;",
            TextAlign::Left => "text-align:left;",
            TextAlign::Center => "text-align:center;",
            TextAlign::Right => "text-align:right;",
            TextAlign::End => "text-align:end;",
            #[allow(unreachable_patterns)]
            _ => "",
        });
    }
    if let Some(weight) = s.font_weight {
        let _ = write!(css, "font-weight:{weight};");
    }
    if s.overflow().italic == Some(true) {
        css.push_str("font-style:italic;");
    }
    match s.corners {
        Some(Some(c)) => {
            let _ = write!(
                css,
                "border-radius:{}px {}px {}px {}px;",
                num(c.top_left),
                num(c.top_right),
                num(c.bottom_right),
                num(c.bottom_left)
            );
        }
        _ => {
            if let Some(radius) = s.radius.get() {
                let _ = write!(css, "border-radius:{}px;", num(radius));
            }
        }
    }
    let border_style = s
        .rare
        .as_ref()
        .and_then(|rare| rare.border_style)
        .map(|style| match style {
            crate::decoration::BorderStyle::Solid => "solid",
            crate::decoration::BorderStyle::Dashed { length, gap } if length <= gap * 0.5 => {
                "dotted"
            }
            _ => "dashed",
        })
        .unwrap_or("solid");
    match s.border_edges {
        Some(Some(insets)) => {
            let _ = write!(
                css,
                "border-style:{border_style};border-width:{}px {}px {}px {}px;",
                num(insets.top),
                num(insets.right),
                num(insets.bottom),
                num(insets.left)
            );
        }
        _ => {
            if let Some(width) = border_width {
                let _ = write!(css, "border:{}px {border_style};", num(width));
            }
        }
    }
    if let Some(color) = s.border_color {
        color_decl(css, "border-color", color);
    }
    if let Some(opacity) = s.opacity.get() {
        let _ = write!(css, "opacity:{};", num(opacity));
    }
    let mut transform = String::new();
    if let Some(t) = s.transform
        && (t.x != 0. || t.y != 0.)
    {
        let _ = write!(transform, "translate({}px,{}px) ", num(t.x), num(t.y));
    }
    let local = s.local_transform;
    if local.set != 0 {
        let v = local.values;
        if v.rotate != 0. {
            let _ = write!(transform, "rotate({}rad) ", num(v.rotate));
        }
        if v.scale != (1., 1.) {
            let _ = write!(transform, "scale({},{}) ", num(v.scale.0), num(v.scale.1));
        }
        if v.skew != (0., 0.) {
            let _ = write!(
                transform,
                "skew({}rad,{}rad) ",
                num(v.skew.0),
                num(v.skew.1)
            );
        }
        let _ = write!(
            css,
            "transform-origin:{}% {}%;",
            num(v.origin.0 * 100.),
            num(v.origin.1 * 100.)
        );
    }
    if !transform.is_empty() {
        let _ = write!(css, "transform:{};", transform.trim_end());
    }
    let o = s.overflow();
    let mut filter = String::new();
    if let Some(blur) = o.filter_blur.get() {
        let _ = write!(filter, "blur({}px) ", num(blur));
    }
    if let Some(v) = o.brightness.get() {
        let _ = write!(filter, "brightness({}) ", num(v));
    }
    if let Some(v) = o.contrast.get() {
        let _ = write!(filter, "contrast({}) ", num(v));
    }
    if let Some(v) = o.saturate.get() {
        let _ = write!(filter, "saturate({}) ", num(v));
    }
    if !filter.is_empty() {
        let _ = write!(css, "filter:{};", filter.trim_end());
    }
    if let Some(blur) = o.blur.get() {
        let _ = write!(css, "backdrop-filter:blur({}px);", num(blur));
    }
    if let Some(options) = &s.layout_options {
        layout_css(options, css);
    }
    if let Some(rare) = &s.rare {
        rare_css(rare, text, css);
    }
}

fn layout_css(o: &crate::layout::LayoutOptions, css: &mut String) {
    use crate::layout::{ContentAlign, Display, FlexWrap};
    if let Some(display) = o.display {
        css.push_str(match display {
            Display::Flex => "display:flex;",
            Display::Grid => "display:grid;",
            Display::None => "display:none;",
        });
    }
    if o.visible == Some(false) {
        css.push_str("visibility:hidden;");
    }
    match (o.clip_x, o.clip_y) {
        (None, None) => {}
        (x, y) => {
            let value = |clip: Option<bool>| match clip {
                Some(true) => "hidden",
                Some(false) => "visible",
                None => "",
            };
            if x.is_some() {
                let _ = write!(css, "overflow-x:{};", value(x));
            }
            if y.is_some() {
                let _ = write!(css, "overflow-y:{};", value(y));
            }
        }
    }
    if let Some(wrap) = o.wrap {
        css.push_str(match wrap {
            FlexWrap::NoWrap => "flex-wrap:nowrap;",
            FlexWrap::Wrap => "flex-wrap:wrap;",
            FlexWrap::Reverse => "flex-wrap:wrap-reverse;",
        });
    }
    if o.reverse == Some(true) {
        css.push_str("--zr:1;");
    }
    if let Some(basis) = o.basis {
        let _ = write!(css, "flex-basis:{};", length(basis));
    }
    if let Some(a) = o.align_self {
        let _ = write!(css, "align-self:{};", align(a));
    }
    if let Some(a) = o.align_content {
        css.push_str(match a {
            ContentAlign::Start => "align-content:flex-start;",
            ContentAlign::Center => "align-content:center;",
            ContentAlign::End => "align-content:flex-end;",
            ContentAlign::Stretch => "align-content:stretch;",
            ContentAlign::SpaceBetween => "align-content:space-between;",
            ContentAlign::SpaceAround => "align-content:space-around;",
            #[allow(unreachable_patterns)]
            _ => "align-content:space-evenly;",
        });
    }
    if let Some(gap) = o.gap_x {
        let _ = write!(css, "column-gap:{};", length(gap));
    }
    if let Some(gap) = o.gap_y {
        let _ = write!(css, "row-gap:{};", length(gap));
    }
    if let Some(ratio) = o.aspect_ratio
        && ratio.is_finite()
        && ratio > 0.
    {
        let _ = write!(css, "aspect-ratio:{};", num(ratio));
    }
    if let Some(columns) = o.columns {
        let _ = write!(
            css,
            "grid-template-columns:repeat({},minmax(0,1fr));",
            columns.max(1)
        );
    }
    if let Some(rows) = o.rows {
        let _ = write!(
            css,
            "grid-template-rows:repeat({},minmax(0,1fr));",
            rows.max(1)
        );
    }
    if o.column_full == Some(true) {
        css.push_str("grid-column:1/-1;");
    } else if o.column_start.is_some() || o.column_span.is_some() {
        let start = o.column_start.filter(|s| *s != 0);
        let span = o.column_span.unwrap_or(1).max(1);
        match start {
            Some(start) => {
                let _ = write!(css, "grid-column:{start}/span {span};");
            }
            None => {
                let _ = write!(css, "grid-column:span {span};");
            }
        }
    }
    if o.row_full == Some(true) {
        css.push_str("grid-row:1/-1;");
    } else if o.row_start.is_some() || o.row_span.is_some() {
        let start = o.row_start.filter(|s| *s != 0);
        let span = o.row_span.unwrap_or(1).max(1);
        match start {
            Some(start) => {
                let _ = write!(css, "grid-row:{start}/span {span};");
            }
            None => {
                let _ = write!(css, "grid-row:span {span};");
            }
        }
    }
    for (property, edges) in [
        ("", o.inset),
        ("margin-", o.margin),
        ("padding-", o.padding),
    ] {
        for (side, value) in [
            ("top", edges.top),
            ("right", edges.right),
            ("bottom", edges.bottom),
            ("left", edges.left),
        ] {
            if let Some(value) = value {
                let _ = write!(css, "{property}{side}:{};", length(value));
            }
        }
    }
    for (property, edges) in [
        ("inset-inline", o.inset_inline),
        ("margin-inline", o.margin_inline),
        ("padding-inline", o.padding_inline),
    ] {
        if let Some(start) = edges.start {
            let _ = write!(css, "{property}-start:{};", length(start));
        }
        if let Some(end) = edges.end {
            let _ = write!(css, "{property}-end:{};", length(end));
        }
    }
    for (property, value) in [
        ("min-width", o.min_width),
        ("max-width", o.max_width),
        ("min-height", o.min_height),
        ("max-height", o.max_height),
    ] {
        if let Some(value) = value {
            let _ = write!(css, "{property}:{};", length(value));
        }
    }
}

fn rare_css(r: &crate::style::RareStyles, text: bool, css: &mut String) {
    for (property, value) in [
        ("min-width", r.min_width),
        ("max-width", r.max_width),
        ("min-height", r.min_height),
        ("max-height", r.max_height),
    ] {
        if let Some(value) = value {
            let _ = write!(css, "{property}:{}px;", num(value));
        }
    }
    if let Some(direction) = r.direction {
        css.push_str(match direction {
            crate::scene::Direction::Ltr => "direction:ltr;",
            crate::scene::Direction::Rtl => "direction:rtl;",
        });
    }
    if let Some(Some((width, color))) = r.outline {
        let _ = write!(
            css,
            "outline:{}px solid {};",
            num(width),
            color_value(color)
        );
        if let Some(offset) = r.outline_offset {
            let _ = write!(css, "outline-offset:{}px;", num(offset));
        }
    }
    let mut shadows = Vec::new();
    let mut shadow = |s: &crate::scene::BoxShadow, inset: bool| {
        shadows.push(format!(
            "{}{}px {}px {}px {}px {}",
            if inset { "inset " } else { "" },
            num(s.offset.x),
            num(s.offset.y),
            num(s.blur_radius * 2.),
            num(s.spread),
            color_value(s.color)
        ));
    };
    if let Some(Some(s)) = &r.shadow {
        shadow(s, false);
    }
    if let Some(Some(list)) = &r.shadows {
        for s in list.iter() {
            shadow(s, false);
        }
    }
    if let Some(Some(list)) = &r.inset_shadows {
        for s in list.iter() {
            shadow(s, true);
        }
    }
    if !shadows.is_empty() {
        let _ = write!(css, "box-shadow:{};", shadows.join(","));
    } else if matches!(r.shadow, Some(None)) || matches!(r.shadows, Some(None)) {
        css.push_str("box-shadow:none;");
    }
    if let Some(spacing) = r.letter_spacing
        && spacing.cell().is_none()
    {
        let _ = write!(css, "letter-spacing:{}px;", num(spacing.pixels()));
    }
    if r.underline == Some(true) {
        css.push_str("text-decoration:underline;");
    }
    if r.text_overflow == Some(crate::text_layout::TextOverflow::Ellipsis) {
        css.push_str("overflow:hidden;text-overflow:ellipsis;");
        if text {
            css.push_str("min-width:0;");
        }
    }
    if let Some(Some(lines)) = r.line_clamp {
        let _ = write!(
            css,
            "display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:{lines};overflow:hidden;"
        );
    }
    if r.relative == Some(true) {
        css.push_str("position:relative;");
    }
    if let Some(blend) = r.blend {
        let _ = write!(css, "mix-blend-mode:{};", blend_css(blend));
    }
    if let Some(fade) = r.fade_edges {
        let _ = write!(
            css,
            "mask-image:linear-gradient(to bottom,transparent,#000 {}px,#000 calc(100% - {}px),transparent);",
            num(fade[0]),
            num(fade[1])
        );
    }
    if let Some(cold) = &r.cold {
        if let Some(z) = cold.z {
            let _ = write!(css, "z-index:{z};");
        }
        if cold.display_contents {
            css.push_str("display:contents;");
        }
        if let Some(top) = cold.sticky_top {
            let _ = write!(css, "position:sticky;top:{}px;", num(top));
        }
        if let Some(template) = &cold.grid_template {
            if let Some(columns) = &template.columns {
                let _ = write!(css, "grid-template-columns:{};", tracks_css(columns));
            }
            if let Some(rows) = &template.rows {
                let _ = write!(css, "grid-template-rows:{};", tracks_css(rows));
            }
        }
        if let Some(lang) = &cold.lang {
            let _ = lang;
        }
        if cold.text_balance == Some(true) {
            css.push_str("text-wrap:balance;");
        }
    }
}

fn blend_css(blend: crate::scene::BlendMode) -> String {
    let name = format!("{blend:?}");
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('-');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

fn cursor_css(cursor: crate::cursor::Cursor) -> String {
    let name = format!("{cursor:?}");
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('-');
        }
        out.push(c.to_ascii_lowercase());
    }
    match out.as_str() {
        "arrow" => "default".into(),
        "hand" | "pointing-hand" => "pointer".into(),
        "i-beam" | "ibeam" => "text".into(),
        _ => out,
    }
}

fn family_css(family: &crate::text_layout::FontFamily) -> String {
    use crate::text_layout::FontFamily;
    match family {
        FontFamily::SansSerif => "system-ui,-apple-system,'Segoe UI',sans-serif".into(),
        FontFamily::Serif => "serif".into(),
        FontFamily::Monospace => "ui-monospace,SFMono-Regular,Menlo,Consolas,monospace".into(),
        #[allow(unreachable_patterns)]
        other => format!(
            "'{}',system-ui,sans-serif",
            crate::theme::family_name(other).replace('\'', "")
        ),
    }
}

fn background_css(css: &mut String, background: &crate::decoration::Background) {
    use crate::decoration::Background;
    match background {
        Background::Brush(crate::canvas::Brush::Solid(color)) => {
            color_decl(css, "background-color", *color);
        }
        Background::Linear { angle, stops } => {
            let stops: Vec<String> = stops
                .iter()
                .map(|stop| format!("{} {}%", color_value(stop.color), num(stop.offset * 100.)))
                .collect();
            let _ = write!(
                css,
                "background-image:linear-gradient({}deg,{});",
                num(*angle),
                stops.join(",")
            );
        }
        _ => {}
    }
}

fn breadth_css(b: crate::grid_tracks::TrackBreadth) -> String {
    use crate::grid_tracks::TrackBreadth;
    match b {
        TrackBreadth::Points(px) => format!("{}px", num(px)),
        TrackBreadth::Percent(f) => format!("{}%", num(f * 100.)),
        TrackBreadth::Fr(fr) => format!("{}fr", num(fr)),
        TrackBreadth::Auto => "auto".into(),
        TrackBreadth::MinContent => "min-content".into(),
        TrackBreadth::MaxContent => "max-content".into(),
    }
}

fn size_css(size: crate::grid_tracks::TrackSize) -> String {
    use crate::grid_tracks::TrackSize;
    match size {
        TrackSize::Is(b) => breadth_css(b),
        TrackSize::MinMax(min, max) => format!("minmax({},{})", breadth_css(min), breadth_css(max)),
        TrackSize::FitContent(b) => format!("fit-content({})", breadth_css(b)),
    }
}

/// A track list as CSS writes it.
fn tracks_css(tracks: &crate::grid_tracks::GridTracks) -> String {
    use crate::grid_tracks::{Track, TrackRepeat};
    let parts: Vec<String> = tracks
        .0
        .iter()
        .map(|track| match track {
            Track::Size(size) => size_css(*size),
            Track::Repeat(times, sizes) => {
                let times = match times {
                    TrackRepeat::Times(n) => n.to_string(),
                    TrackRepeat::AutoFill => "auto-fill".into(),
                    TrackRepeat::AutoFit => "auto-fit".into(),
                };
                let sizes: Vec<String> = sizes.iter().map(|s| size_css(*s)).collect();
                format!("repeat({times},{})", sizes.join(" "))
            }
            Track::Names(names) => format!("[{}]", names.join(" ")),
        })
        .collect();
    parts.join(" ")
}
