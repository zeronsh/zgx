//! Popups past the window on Wayland: each an `xdg_popup` of the window's
//! `xdg_surface` (the vendored winit's `WindowExtWayland::xdg_surface`), which
//! the compositor places and keeps on its outputs.
//!
//! zgx's own objects live on an event queue of its own on winit's display,
//! dispatched where the event loop turns (`dispatch`): winit reads the socket
//! for both, so the queue costs no wakeup of its own. A popup is a
//! `wl_surface` with an `xdg_surface` and its `xdg_popup` role, made as it
//! shows and destroyed as it hides, drawn by a renderer of its own on the
//! window's device. One that moves or changes size (an animated popover) is
//! moved and resized in place (`xdg_popup.reposition`), never shown anew:
//! compositors fade popups in and out (Hyprland's `fadePopups`), so a new
//! surface each frame flashed it see-through. (A compositor before
//! `xdg_wm_base` v3 has no reposition: there it shows anew.) Its
//! positioner anchors it where the scene placed it (the
//! window geometry's coordinates, which start above the content by any title
//! bar winit draws), against its trigger where it has one
//! (`Scene::popup_anchor`): the compositor flips it to the trigger's other
//! side at the output's edge, as GTK's menus, or slides it onto the output.
//! The compositor's word on where it went is final, and input goes by where
//! the scene drew it, so what is under the pointer is what shows there. A
//! popup taller (or wider) than the output has room for is cut to it by the
//! compositor, which says so in its configure: the scene lays its panel out
//! no bigger (`Ui::set_popup_limit`), a menu scrolling its items, and it
//! takes that size. One the compositor flipped to its trigger's
//! other side is placed there by the scene (`Ui::set_popup_flipped`), so a
//! popover's arrow faces its trigger.
//!
//! A popup's frames are paced by its surface's frame callbacks, as the
//! window's are (winit's `pre_present_notify`): one drawn while the
//! compositor has yet to take the last waits for it (`Popups::deadline`
//! wakes the loop should its callback be missed). Its swap chain presents
//! in FIFO, so a frame presented sooner blocked the event loop in the next
//! acquire until the compositor took one, a refresh or more (7 to 20 ms a
//! frame, as a slider dragged in a popup draws on each pointer move).
//!
//! The popup takes no grab: keys stay the window's, which drives menus as
//! before, and a press on another app takes the window's focus, which
//! closes them. Pointer input over a popup comes from zgx's own
//! `wl_pointer`, in the window's coordinates, to its host, which hit-tests
//! the scene there: over a popup's clear margin, what is under it in the
//! window is hovered. A popup's window geometry is its panel alone
//! (`panel_of`), its shadow outside it, as GTK's popups have theirs: the
//! compositor places the panel where the scene did and keeps it on the
//! output, the shadow free to reach past the output's edge (with the whole
//! surface for its geometry, a card whose shadow reached past the output's
//! foot was slid up from where the scene drew it, unsaid, and the
//! pointer's moves over it were read that far off). Only what its panel
//! lays out takes the pointer (its input region, `input_rect`), and none
//! of a popup letting the pointer through (a tooltip). A compositor may
//! still give a popup the pointer as it maps where its geometry lies, so
//! the window's own leave onto one of its popups is not the pointer
//! leaving (`pointer_over_popup`), nor a popup's leave back onto the window
//! (`leaving`).
//! todo(wayland): the grab (`xdg_popup.grab`), with keys through a
//! `wl_keyboard` of zgx's own.
use super::PopupInput;
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
    backend::ObjectId,
    delegate_noop,
    protocol::{
        wl_callback::{self, WlCallback},
        wl_compositor::WlCompositor,
        wl_pointer::{self, WlPointer},
        wl_region::WlRegion,
        wl_registry::{self, WlRegistry},
        wl_seat::{self, Capability, WlSeat},
        wl_surface::WlSurface,
    },
};
use wayland_protocols::{
    wp::viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
    xdg::shell::client::{
        xdg_popup::{self, XdgPopup},
        xdg_positioner::{Anchor, ConstraintAdjustment, Gravity, XdgPositioner},
        xdg_surface::{self, XdgSurface},
        xdg_wm_base::{self, XdgWmBase},
    },
};
use winit::{
    dpi::LogicalPosition,
    event::{MouseButton, MouseScrollDelta, TouchPhase},
    raw_window_handle::{HasDisplayHandle, RawDisplayHandle, RawWindowHandle, WaylandWindowHandle},
    window::Window,
};
use zgx::scene::{NodeId, Rect, Scene};
use zgx_gpu::{GpuContext, GpuError, GpuRenderer};

type Sink = Rc<dyn Fn(PopupInput)>;

/// How long a popup's frame waits for the compositor to take its last
/// (its surface's frame callback) before it is drawn anyway: a callback
/// missed, or withheld from a surface not shown, never stalls it.
const FRAME_WAIT: Duration = Duration::from_millis(25);

/// Where a popup's pointer input goes: its box's origin in its parent
/// window (the scene's), and the parent's host.
struct Target {
    origin: (f32, f32),
    scale: f64,
    /// Its parent window's zoom (`zoom_of`): the scene's points are its
    /// surface's divided by it.
    zoom: f32,
    send: Sink,
}

/// What zgx's queue hears.
#[derive(Default)]
struct State {
    compositor: Option<WlCompositor>,
    wm_base: Option<XdgWmBase>,
    viewporter: Option<WpViewporter>,
    seats: Vec<(WlSeat, Option<WlPointer>)>,
    /// Each popup's surface's target.
    targets: HashMap<ObjectId, Target>,
    /// The popup the pointer is over, and where (its parent's points).
    over: Option<(ObjectId, (f32, f32))>,
    /// The pointer left a popup, told its host as this dispatch ends unless
    /// it went onto the window meanwhile, which winit tells (a leave told
    /// after winit's enter would take the pointer off the window).
    leaving: Option<Sink>,
    /// `xdg_surface`s configured before they first showed, by the serial
    /// `show` acks.
    configured: HashMap<ObjectId, u32>,
    /// `xdg_surface`s shown: configured again, acked at once.
    live: std::collections::HashSet<ObjectId>,
    /// Popups the compositor dismissed (`popup_done`).
    done: Vec<ObjectId>,
    /// Each `xdg_popup`'s place and size as last configured (its parent's
    /// window geometry's coordinates).
    placed: HashMap<ObjectId, (i32, i32, i32, i32)>,
    /// The last `xdg_popup.reposition` token asked, and each popup's last
    /// one answered (`repositioned`).
    token: u32,
    repositioned: HashMap<ObjectId, u32>,
    /// Popup surfaces whose last frame the compositor has yet to take
    /// (its `wl_surface.frame` callback), and when it was presented.
    framing: HashMap<ObjectId, Instant>,
}

/// zgx's queue on winit's display.
struct Client {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
    display: RawDisplayHandle,
}

thread_local! {
    /// Never dropped: winit closes the display before the thread's locals
    /// go, and a queue dropped after its display would touch freed memory.
    static CLIENT: RefCell<Option<std::mem::ManuallyDrop<Client>>> = const { RefCell::new(None) };
    /// Tried and found wanting (no `xdg_wm_base`, or not Wayland).
    static FAILED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// zgx's queue on `window`'s display, made as first wanted.
fn with_client<T>(window: &Window, then: impl FnOnce(&mut Client) -> Option<T>) -> Option<T> {
    if FAILED.with(std::cell::Cell::get) {
        return None;
    }
    CLIENT.with(|client| {
        let mut client = client.borrow_mut();
        if client.is_none() {
            *client = connect(window).map(std::mem::ManuallyDrop::new);
            if client.is_none() {
                FAILED.with(|failed| failed.set(true));
            }
        }
        then(client.as_deref_mut()?)
    })
}

/// `window`'s `xdg_surface`, and where its window geometry sits on it: from
/// the vendored winit (`WindowExtWayland`, see vendor/winit/ZGX_PATCH.md),
/// detected at build time (`zgx_winit_patched`, from its `links` metadata).
/// A winit from crates.io gives neither: there are no native popups, and
/// popups open in the window (`available` is false).
#[cfg(zgx_winit_patched)]
fn xdg_surface(window: &Window) -> Option<(std::ptr::NonNull<std::ffi::c_void>, (i32, i32))> {
    use winit::platform::wayland::WindowExtWayland;
    let origin = window.xdg_window_geometry_origin().unwrap_or((0, 0));
    Some((window.xdg_surface()?, origin))
}
#[cfg(not(zgx_winit_patched))]
fn xdg_surface(_: &Window) -> Option<(std::ptr::NonNull<std::ffi::c_void>, (i32, i32))> {
    None
}

fn connect(window: &Window) -> Option<Client> {
    xdg_surface(window)?;
    let display = window.display_handle().ok()?.as_raw();
    let RawDisplayHandle::Wayland(handle) = display else {
        return None;
    };
    // SAFETY: winit's display, connected while its event loop is; the loop
    // runs once per process (winit refuses a second). The backend is never
    // dropped (`CLIENT`), so never touches the display once winit has closed
    // it, and is used only from the loop's turns (`dispatch`) and from
    // windows' hosts, which all go before the loop does
    // (`WindowManager::exiting`).
    let backend = unsafe {
        wayland_backend::sys::client::Backend::from_foreign_display(handle.display.as_ptr().cast())
    };
    let connection = Connection::from_backend(backend);
    let mut queue = connection.new_event_queue::<State>();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    let mut state = State::default();
    queue.roundtrip(&mut state).ok()?;
    // The seats' capabilities.
    queue.roundtrip(&mut state).ok()?;
    if state.compositor.is_none() || state.wm_base.is_none() {
        return None;
    }
    Some(Client {
        connection,
        queue,
        state,
        display,
    })
}

/// Whether popups show past `window` (Wayland's): its compositor has
/// `xdg_wm_base`, and winit gives its `xdg_surface`.
pub(super) fn available(window: &Window) -> bool {
    with_client(window, |_| Some(())).is_some()
}

/// Dispatch what zgx's queue heard (where the event loop turns: winit read
/// the socket for both).
pub(crate) fn dispatch() {
    CLIENT.with(|client| {
        if let Ok(mut client) = client.try_borrow_mut()
            && let Some(client) = client.as_deref_mut()
        {
            let _ = client.queue.dispatch_pending(&mut client.state);
            if let Some(send) = client.state.leaving.take() {
                (send)(PopupInput::Left);
            }
            let _ = client.connection.flush();
        }
    });
}

/// Whether the pointer is over one of the window's popups, as zgx's queue
/// has heard so far (dispatched first): the window's own leave is then the
/// pointer going onto it, not away.
pub(crate) fn pointer_over_popup() -> bool {
    dispatch();
    CLIENT.with(|client| {
        client
            .try_borrow()
            .ok()
            .and_then(|client| client.as_deref().map(|client| client.state.over.is_some()))
            .unwrap_or(false)
    })
}

/// A popup shown.
struct Shown {
    // Dropped first: its surface goes before the wl_surface it draws to.
    renderer: GpuRenderer,
    popup: XdgPopup,
    xdg: XdgSurface,
    viewport: Option<WpViewport>,
    surface: WlSurface,
    rect: Rect,
    /// Its positioner's rules as last asked: its panel's size (whole
    /// points), its place against its trigger.
    rules: Rules,
    /// Its panel (`panel_of`, the scene's): its window geometry, what a
    /// popup of its own opens against.
    panel: Rect,
    /// Its surface's size and its window geometry on it (the panel), as
    /// last set (`surface_and_geometry`).
    surface_size: (i32, i32),
    geometry: (i32, i32, i32, i32),
    /// Where it asked to be, and its trigger, if any (its parent's window
    /// geometry's coordinates).
    asked: Rect,
    trigger: Option<Rect>,
    revision: Option<u64>,
    /// Its frame's last contained panic (`contain::run`).
    fault: Option<crate::contain::Fault>,
    /// The popup it is a popup of (a submenu's menu), if not the window's.
    parent: Option<NodeId>,
    /// Its input region as last set (`input_rect`), once set.
    input: Option<Option<Rect>>,
}
impl Shown {
    fn destroy(self, client: &mut Client) {
        let Shown {
            renderer,
            popup,
            xdg,
            viewport,
            surface,
            ..
        } = self;
        drop(renderer);
        client.state.targets.remove(&surface.id());
        client.state.framing.remove(&surface.id());
        if client
            .state
            .over
            .as_ref()
            .is_some_and(|(over, _)| *over == surface.id())
        {
            client.state.over = None;
        }
        client.state.live.remove(&xdg.id());
        client.state.placed.remove(&popup.id());
        client.state.repositioned.remove(&popup.id());
        popup.destroy();
        xdg.destroy();
        if let Some(viewport) = viewport {
            viewport.destroy();
        }
        surface.destroy();
        let _ = client.connection.flush();
    }
    /// Whether it can be moved and resized in place (`xdg_popup` v3).
    fn movable(&self) -> bool {
        self.popup.version() >= 3
    }
    /// Move and resize it in place to `rect` (the scene's), by `rules`
    /// (`xdg_popup.reposition`): the same surface, so no compositor fades
    /// it out and a new one in, and the pointer stays over it. Its window
    /// geometry and viewport take the new size, and its new place comes in
    /// a configure, acked as it comes (it is live): its next frame, drawn at
    /// that size, commits them all at once.
    fn reposition(
        &mut self,
        client: &mut Client,
        (rect, panel): (Rect, Rect),
        zoom: f32,
        (rules, asked, trigger): (Rules, Rect, Option<Rect>),
    ) {
        let Some(wm_base) = client.state.wm_base.clone() else {
            return;
        };
        let qh = client.queue.handle();
        let positioner = rules.positioner(&wm_base, &qh);
        client.state.token = client.state.token.wrapping_add(1);
        let token = client.state.token;
        self.popup.reposition(&positioner, token);
        positioner.destroy();
        // Its surface round its panel, and the panel on it (its window
        // geometry), in the surfaces' coordinates.
        let zoomed = |r: Rect| super::window_rect(r, zoom);
        let (surface_size, geometry) =
            surface_and_geometry(zoomed(rect), zoomed(panel), rules.size);
        if geometry != self.geometry {
            let (x, y, width, height) = geometry;
            self.xdg.set_window_geometry(x, y, width, height);
            self.geometry = geometry;
        }
        if surface_size != self.surface_size {
            if let Some(viewport) = &self.viewport {
                viewport.set_destination(surface_size.0, surface_size.1);
            }
            self.surface_size = surface_size;
        }
        let id = self.popup.id();
        for _ in 0..2 {
            if client.state.repositioned.get(&id) == Some(&token) {
                break;
            }
            if client.queue.roundtrip(&mut client.state).is_err() {
                break;
            }
        }
        self.rect = rect;
        self.panel = panel;
        self.rules = rules;
        self.asked = asked;
        self.trigger = trigger;
        // Drawn at its new size.
        self.revision = None;
    }
}

/// A popup's `xdg_positioner` rules (its parent's window geometry's
/// coordinates): its size, anchor rectangle, anchor, gravity, offset and
/// adjustments.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rules {
    size: (i32, i32),
    anchor_rect: (i32, i32, i32, i32),
    anchor: Anchor,
    gravity: Gravity,
    offset: (i32, i32),
    adjust: ConstraintAdjustment,
}
impl Rules {
    /// Those for a popup whose panel is at `rect` (its window geometry),
    /// opened against `trigger` (the surfaces'
    /// coordinates: the scene's at its window's zoom), of a surface whose
    /// window geometry starts at `origin`; and where it asks to be and its
    /// trigger, in that geometry's coordinates.
    fn of(rect: Rect, trigger: Option<Rect>, origin: (i32, i32)) -> (Self, Rect, Option<Rect>) {
        let geometry = |r: Rect| {
            Rect::new(
                r.x - origin.0 as f32,
                r.y - origin.1 as f32,
                r.width,
                r.height,
            )
        };
        let (asked, trigger) = (geometry(rect), trigger.map(geometry));
        let (anchor_rect, anchor, gravity, offset, adjust) = position(asked, trigger);
        let rules = Rules {
            size: (
                rect.width.ceil().max(1.) as i32,
                rect.height.ceil().max(1.) as i32,
            ),
            anchor_rect: (
                anchor_rect.x as i32,
                anchor_rect.y as i32,
                anchor_rect.width as i32,
                anchor_rect.height as i32,
            ),
            anchor,
            gravity,
            offset,
            adjust,
        };
        (rules, asked, trigger)
    }
    fn positioner(&self, wm_base: &XdgWmBase, qh: &QueueHandle<State>) -> XdgPositioner {
        let positioner = wm_base.create_positioner(qh, ());
        positioner.set_size(self.size.0, self.size.1);
        let (x, y, width, height) = self.anchor_rect;
        positioner.set_anchor_rect(x, y, width, height);
        positioner.set_anchor(self.anchor);
        positioner.set_gravity(self.gravity);
        positioner.set_offset(self.offset.0, self.offset.1);
        positioner.set_constraint_adjustment(self.adjust);
        positioner
    }
}

/// Where the window geometry of the surface a popup opens from starts (the
/// surfaces' coordinates): `holder`'s box (the scene's) where it opens from
/// a popup, else `parent`'s, below any title bar winit draws.
fn origin_of(parent: &Window, holder: Option<Rect>, zoom: f32) -> Option<(i32, i32)> {
    match holder {
        Some(rect) => {
            let rect = super::window_rect(rect, zoom);
            Some((rect.x.round() as i32, rect.y.round() as i32))
        }
        None => xdg_surface(parent).map(|(_, origin)| origin),
    }
}

/// A window's popups on Wayland.
pub(super) struct Popups {
    shown: Vec<(NodeId, Shown)>,
    /// Popups the compositor cut to its room, and the size it gave them
    /// (`Ui::set_popup_limit`), until they close.
    limited: Vec<(NodeId, (i32, i32))>,
    /// Popups the compositor flipped (`Ui::set_popup_flipped`), until they
    /// close: once each, so a popup with room on neither side settles.
    flipped: Vec<NodeId>,
    send: Sink,
    /// When a popup's frame, waiting for the compositor to take its last,
    /// is drawn anyway (`FRAME_WAIT`).
    wake: Option<Instant>,
}
impl Popups {
    pub fn new(send: Sink) -> Self {
        Self {
            shown: Vec::new(),
            limited: Vec::new(),
            flipped: Vec::new(),
            send,
            wake: None,
        }
    }
    pub fn showing(&self) -> bool {
        !self.shown.is_empty()
    }
    /// When the loop must turn for a popup's frame left waiting on its last
    /// (`update`): drawn as its frame callback comes, which wakes the loop,
    /// or at this at the latest.
    pub fn deadline(&self) -> Option<Instant> {
        self.wake
    }
    /// After the scene's flush: show each visible popup root in an
    /// `xdg_popup` of `parent`, draw it again where it changed (always when
    /// `full`), and hide the rest.
    /// `takes` says whether a node takes the pointer (listening to it, or
    /// disabled: `Some(true)`) or lets it through (`View::pointer_events`:
    /// `Some(false)`), or leaves that to what holds it (`None`).
    pub fn update(
        &mut self,
        parent: &Window,
        scene: &Scene,
        context: &GpuContext,
        scale: f32,
        full: bool,
        takes: &dyn Fn(NodeId) -> Option<bool>,
    ) -> Result<(), GpuError> {
        self.wake = None;
        if self.shown.is_empty() && scene.popups().is_empty() {
            return Ok(());
        }
        // Its window's zoom: the scene's points are the window's (and its
        // popups' surfaces') divided by it.
        let zoom = super::zoom_of(parent.id()) as f32;
        let wanted: Vec<(NodeId, Rect)> = scene
            .popups()
            .iter()
            .filter_map(|root| scene.popup_bounds(*root).map(|bounds| (*root, bounds)))
            .collect();
        let shown = std::mem::take(&mut self.shown);
        let limited = &mut self.limited;
        let flipped = &mut self.flipped;
        let send = self.send.clone();
        let wake = &mut self.wake;
        with_client(parent, |client| {
            // Frame callbacks read off the socket since the loop dispatched
            // (by the window's present, say).
            let _ = client.queue.dispatch_pending(&mut client.state);
            let qh = client.queue.handle();
            // Closed: no longer cut to the room it had.
            limited.retain(|(root, _)| {
                let open = wanted.iter().any(|(wanted, _)| wanted == root);
                if !open {
                    (send)(PopupInput::Limit {
                        root: *root,
                        size: None,
                    });
                }
                open
            });
            flipped.retain(|root| {
                let open = wanted.iter().any(|(wanted, _)| wanted == root);
                if !open {
                    (send)(PopupInput::Flip {
                        root: *root,
                        flipped: (false, false),
                    });
                }
                open
            });
            // Dismissed by the compositor: its host closes it.
            for done in client.state.done.drain(..) {
                if shown.iter().any(|(_, popup)| popup.popup.id() == done) {
                    (send)(PopupInput::Dismiss);
                }
            }
            // What goes: popups no longer wanted, or moved or resized where
            // the compositor cannot do that in place (`xdg_popup` before v3:
            // to show anew), and the popups of those (a parent goes after
            // its children, as the protocol has it).
            let mut kept = shown;
            let mut going: Vec<NodeId> = kept
                .iter()
                .filter(|(root, popup)| {
                    !wanted.iter().any(|(wanted, bounds)| {
                        wanted == root
                            && (popup.movable()
                                || rects_match(*bounds, popup.rect)
                                    && rects_match(panel_of(scene, *root, *bounds), popup.panel))
                    })
                })
                .map(|(root, _)| *root)
                .collect();
            loop {
                let children: Vec<NodeId> = kept
                    .iter()
                    .filter(|(root, popup)| {
                        !going.contains(root) && popup.parent.is_some_and(|p| going.contains(&p))
                    })
                    .map(|(root, _)| *root)
                    .collect();
                if children.is_empty() {
                    break;
                }
                going.extend(children);
            }
            // The newest first: children before their parents.
            for index in (0..kept.len()).rev() {
                if going.contains(&kept[index].0) {
                    let (root, popup) = kept.remove(index);
                    zgx_gpu::present_log::log(|| format!("popup hide {root:?}"));
                    popup.destroy(client);
                }
            }
            for (root, bounds) in &wanted {
                let index = match kept.iter().position(|(shown, _)| shown == root) {
                    // Moved or resized, or its trigger or the popup it opens
                    // from moved: moved and resized in place.
                    Some(index) if kept[index].1.movable() => {
                        let trigger = scene.popup_anchor(*root).map(|anchor| scene.bounds(anchor));
                        // A submenu opens against its menu's panel.
                        let holder = kept[index].1.parent.and_then(|holder| {
                            kept.iter()
                                .find(|(shown, _)| *shown == holder)
                                .map(|(_, popup)| popup.panel)
                        });
                        let zoomed = |r: Rect| super::window_rect(r, zoom);
                        if let Some(origin) = origin_of(parent, holder, zoom) {
                            let panel = panel_of(scene, *root, *bounds);
                            let wants = Rules::of(zoomed(panel), trigger.map(zoomed), origin);
                            let popup = &mut kept[index].1;
                            if wants.0 != popup.rules
                                || !rects_match(*bounds, popup.rect)
                                || !rects_match(panel, popup.panel)
                            {
                                zgx_gpu::present_log::log(|| {
                                    format!("popup reposition {root:?} to {bounds:?}")
                                });
                                popup.reposition(client, (*bounds, panel), zoom, wants);
                            }
                        }
                        index
                    }
                    Some(index) => index,
                    None => {
                        let anchor = scene.popup_anchor(*root);
                        let trigger = anchor.map(|anchor| scene.bounds(anchor));
                        // A submenu is a popup of the menu holding its item,
                        // placed against that menu's surface, wherever the
                        // compositor put it.
                        let holder = anchor.and_then(|anchor| {
                            kept.iter().find(|(shown, _)| {
                                scene.ancestors(anchor).any(|node| node == *shown)
                            })
                        });
                        let of =
                            holder.map(|(holder, popup)| (*holder, popup.xdg.clone(), popup.panel));
                        let panel = panel_of(scene, *root, *bounds);
                        let Some(popup) = show(
                            client,
                            parent,
                            of,
                            *root,
                            (*bounds, panel),
                            trigger,
                            scale,
                            zoom,
                            context,
                        ) else {
                            continue;
                        };
                        kept.push((*root, popup));
                        kept.len() - 1
                    }
                };
                let popup = &mut kept[index].1;
                // Cut by the compositor to the room on its output: the scene
                // lays it out no bigger, and it takes that size.
                let placed = client.state.placed.get(&popup.popup.id()).copied();
                if let Some((_, _, width, height)) = placed
                    && width > 0
                    && height > 0
                    && (width < popup.rules.size.0 || height < popup.rules.size.1)
                    && !limited.contains(&(*root, (width, height)))
                {
                    limited.retain(|(limited, _)| limited != root);
                    limited.push((*root, (width, height)));
                    zgx_gpu::present_log::log(|| format!("popup cut {root:?} to {width}x{height}"));
                    // In the scene's points: the surface's over the zoom.
                    (send)(PopupInput::Limit {
                        root: *root,
                        size: Some((width as f32 / zoom, height as f32 / zoom)),
                    });
                }
                // Flipped by the compositor to its trigger's other side: the
                // scene places it there, and it moves there.
                if let Some((x, y, width, height)) = placed
                    && let Some(trigger) = popup.trigger
                    && !flipped.contains(root)
                {
                    let got = Rect::new(x as f32, y as f32, width as f32, height as f32);
                    let turned = sides_turned(popup.asked, got, trigger);
                    if turned != (false, false) {
                        flipped.push(*root);
                        zgx_gpu::present_log::log(|| format!("popup flipped {root:?} {turned:?}"));
                        (send)(PopupInput::Flip {
                            root: *root,
                            flipped: turned,
                        });
                    }
                }
                let revision = scene.layer_revision(*root);
                let due = full || popup.revision != Some(revision);
                // The compositor yet to take its last frame: this one waits
                // for it (its frame callback wakes the loop), drawn then.
                let surface = popup.surface.id();
                let waiting = waits(client.state.framing.get(&surface).copied(), Instant::now())
                    .filter(|_| due);
                let draw = due && waiting.is_none();
                // Where it takes the pointer: set before its frame, which
                // commits it (or committed alone, with no frame to draw).
                // Found again only as it shows or draws (what it lays out
                // changed): a walk of all it holds, not for every frame.
                let mut input_changed = false;
                if draw || popup.input.is_none() {
                    let input = input_rect(scene, *root, *bounds, takes);
                    if popup.input != Some(input) {
                        set_input(client, &popup.surface, input, zoom);
                        popup.input = Some(input);
                        input_changed = true;
                    }
                }
                if input_changed && !draw {
                    popup.surface.commit();
                }
                if let Some(until) = waiting {
                    popup.revision = None;
                    *wake = Some(wake.map_or(until, |wake| wake.min(until)));
                } else if draw {
                    popup.revision = Some(revision);
                    popup.renderer.set_popup(*root, *bounds);
                    let renderer = &mut popup.renderer;
                    let (wl_surface, framing) = (&popup.surface, &mut client.state.framing);
                    let started = Instant::now();
                    let status = crate::contain::run_frame(
                        &mut popup.fault,
                        revision,
                        started,
                        "a popup's frame",
                        crate::contain::queue,
                        || {
                            renderer.repaint_all();
                            renderer.render(scene, &[])?;
                            // Its frame callback, on the commit presenting it.
                            renderer.present_with_notify(|| {
                                wl_surface.frame(&qh, surface.clone());
                                framing.insert(surface.clone(), Instant::now());
                            })
                        },
                    );
                    for issue in popup.renderer.take_issues() {
                        crate::contain::queue_issue(issue);
                    }
                    zgx_gpu::present_log::log(|| {
                        format!(
                            "popup draw {root:?} {status:?} in {} us",
                            started.elapsed().as_micros()
                        )
                    });
                }
                client.state.targets.insert(
                    popup.surface.id(),
                    Target {
                        origin: (popup.rect.x, popup.rect.y),
                        scale: scale as f64,
                        zoom,
                        send: send.clone(),
                    },
                );
            }
            let _ = client.connection.flush();
            self.shown = kept;
            Some(())
        });
        Ok(())
    }
    /// Its popups' renderers, for a memory report.
    pub fn renderers(&self) -> impl Iterator<Item = &zgx_gpu::GpuRenderer> {
        self.shown.iter().map(|(_, popup)| &popup.renderer)
    }
    pub fn hide_all(&mut self, parent: &Window) {
        self.wake = None;
        let shown = std::mem::take(&mut self.shown);
        with_client(parent, |client| {
            // The newest first: children before their parents.
            for (_, popup) in shown.into_iter().rev() {
                popup.destroy(client);
            }
            Some(())
        });
    }
}

/// Whether a popup that asked to be at `asked` beside `trigger` was put at
/// `got` on the trigger's other side, across and down.
fn sides_turned(asked: Rect, got: Rect, trigger: Rect) -> (bool, bool) {
    let (right, bottom) = (trigger.x + trigger.width, trigger.y + trigger.height);
    let before = |start: f32, size: f32, edge: f32| start + size <= edge + 0.5;
    let after = |start: f32, edge: f32| start >= edge - 0.5;
    let down = (after(asked.y, bottom) && before(got.y, got.height, trigger.y))
        || (before(asked.y, asked.height, trigger.y) && after(got.y, bottom));
    let across = (after(asked.x, right) && before(got.x, got.width, trigger.x))
        || (before(asked.x, asked.width, trigger.x) && after(got.x, right));
    (across, down)
}

/// Where popup `root`, shown at `bounds`, takes the pointer, from
/// `bounds`' top-left (the scene's points): round the boxes laid out in it
/// that take the pointer as the window's hit test has it (`takes`: a node
/// with listeners of its own takes it, `Some(true)`, one letting it
/// through gives it to what is under, `Some(false)`, nearest first, else
/// what holds it decides), and never the ink painted past them (a card's
/// shadow, which reaches far past the card). `None` where nothing in it
/// takes it (a tooltip, its card listening to nothing, in its layer that
/// lets the pointer through). A box clipping its content bounds what it
/// holds.
fn input_rect(
    scene: &Scene,
    root: NodeId,
    bounds: Rect,
    takes: &dyn Fn(NodeId) -> Option<bool>,
) -> Option<Rect> {
    // What holds it decides for it, where it says nothing itself.
    let held = scene
        .ancestors(root)
        .filter(|node| *node != scene.root())
        .find_map(takes)
        .unwrap_or(false);
    // Each level's answer down the walk (it goes depth first).
    let mut answers: Vec<bool> = Vec::new();
    let mut taken: Option<Rect> = None;
    scene.walk(root, |item| {
        answers.truncate(item.depth);
        let above = answers.last().copied().unwrap_or(held);
        let answer = takes(item.id).unwrap_or(above);
        answers.push(answer);
        let b = item.bounds;
        if answer && b.width > 0. && b.height > 0. {
            taken = Some(taken.map_or(b, |taken| taken.union(b)));
        }
        scene.clips(item.id) != (true, true)
    });
    let taken = taken?.intersection(bounds)?;
    Some(Rect::new(
        taken.x - bounds.x,
        taken.y - bounds.y,
        taken.width,
        taken.height,
    ))
}

/// Set `surface`'s input region to `input` (the scene's points from its
/// top-left, at the window's `zoom`), or to nothing.
fn set_input(client: &Client, surface: &WlSurface, input: Option<Rect>, zoom: f32) {
    let Some(compositor) = &client.state.compositor else {
        return;
    };
    let region = compositor.create_region(&client.queue.handle(), ());
    if let Some(input) = input {
        // Rounded out to whole surface points.
        let r = super::window_rect(input, zoom);
        let (x, y) = (r.x.floor(), r.y.floor());
        let (right, bottom) = ((r.x + r.width).ceil(), (r.y + r.height).ceil());
        region.add(x as i32, y as i32, (right - x) as i32, (bottom - y) as i32);
    }
    surface.set_input_region(Some(&region));
    region.destroy();
}

/// Popup `root`'s panel, shown at `bounds` (the scene's): the box its root
/// lays out, never the ink it paints past that (a card's shadow, which
/// reaches far past the card). Its window geometry, as GTK's popups leave
/// their shadows out of theirs: the compositor places that against the
/// trigger and keeps it on the output, the shadow free to reach past the
/// output's edge, and takes it for where the popup is (Hyprland gives a
/// popup the pointer by its geometry as it maps). With the whole surface
/// for its geometry, a card whose shadow reached past the output's foot was
/// slid up off where the scene drew it, unsaid, and the pointer's moves
/// over it were read that far off: off its trigger, which closed it.
fn panel_of(scene: &Scene, root: NodeId, bounds: Rect) -> Rect {
    scene
        .bounds(root)
        .intersection(bounds)
        .filter(|panel| panel.width >= 1. && panel.height >= 1.)
        .unwrap_or(bounds)
}

/// A popup's surface size and its window geometry on it (`panel_of`), the
/// surfaces' coordinates, for its surface at `rect` and its panel at
/// `panel` (both the surfaces' coordinates) asking `size` for the panel.
fn surface_and_geometry(
    rect: Rect,
    panel: Rect,
    size: (i32, i32),
) -> ((i32, i32), (i32, i32, i32, i32)) {
    let surface = (
        rect.width.ceil().max(1.) as i32,
        rect.height.ceil().max(1.) as i32,
    );
    let x = ((panel.x - rect.x).round() as i32).clamp(0, surface.0 - 1);
    let y = ((panel.y - rect.y).round() as i32).clamp(0, surface.1 - 1);
    let width = size.0.clamp(1, surface.0 - x);
    let height = size.1.clamp(1, surface.1 - y);
    (surface, (x, y, width, height))
}

/// Until when a popup's next frame waits, its last presented at
/// `presented` and not yet taken by the compositor (its frame callback
/// outstanding): `FRAME_WAIT` on at most. `None`: it draws now.
fn waits(presented: Option<Instant>, now: Instant) -> Option<Instant> {
    presented
        .map(|presented| presented + FRAME_WAIT)
        .filter(|until| *until > now)
}

fn rects_match(a: Rect, b: Rect) -> bool {
    (a.x - b.x).abs() < 0.5
        && (a.y - b.y).abs() < 0.5
        && (a.width - b.width).abs() < 0.5
        && (a.height - b.height).abs() < 0.5
}

/// Where an `xdg_positioner` puts a popup the scene placed at `rect` (in the
/// window geometry's coordinates, as `trigger`): against its trigger, on the
/// side the scene put it, the compositor free to flip it to the other side
/// at the output's edge; with no trigger, or over it (`open_over`), at its
/// place, slid onto the output. Anchor rectangle, anchor, gravity, offset
/// and adjustments.
fn position(
    rect: Rect,
    trigger: Option<Rect>,
) -> (Rect, Anchor, Gravity, (i32, i32), ConstraintAdjustment) {
    // Slid onto the output, and cut to it where it is bigger than the room
    // there (the compositor flips, then slides, then cuts).
    let slide = ConstraintAdjustment::SlideX
        | ConstraintAdjustment::SlideY
        | ConstraintAdjustment::ResizeX
        | ConstraintAdjustment::ResizeY;
    let point = (
        Rect::new(rect.x.round(), rect.y.round(), 1., 1.),
        Anchor::TopLeft,
        Gravity::BottomRight,
        (0, 0),
        slide,
    );
    let Some(t) = trigger.filter(|t| t.width >= 1. && t.height >= 1.) else {
        return point;
    };
    let t = Rect::new(t.x.round(), t.y.round(), t.width.round(), t.height.round());
    let (right, bottom) = (t.x + t.width, t.y + t.height);
    let offset = |x: f32, y: f32| (x.round() as i32, y.round() as i32);
    let (flip_x, flip_y) = (
        ConstraintAdjustment::FlipX | slide,
        ConstraintAdjustment::FlipY | slide,
    );
    // The gap from the trigger widens the anchor rectangle on both sides,
    // rather than going in the offset, which a flip keeps as it is
    // (wlroots): flipped, it opens as far from the other side.
    let tall = |gap: f32| Rect::new(t.x, t.y - gap, t.width, t.height + 2. * gap);
    let wide = |gap: f32| Rect::new(t.x - gap, t.y, t.width + 2. * gap, t.height);
    if rect.y >= bottom - 0.5 {
        let anchor = tall((rect.y - bottom).round());
        let at = offset(rect.x - t.x, 0.);
        (anchor, Anchor::BottomLeft, Gravity::BottomRight, at, flip_y)
    } else if rect.y + rect.height <= t.y + 0.5 {
        let anchor = tall((t.y - rect.y - rect.height).round());
        let at = offset(rect.x - t.x, 0.);
        (anchor, Anchor::TopLeft, Gravity::TopRight, at, flip_y)
    } else if rect.x >= right - 0.5 {
        let anchor = wide((rect.x - right).round());
        let at = offset(0., rect.y - t.y);
        (anchor, Anchor::TopRight, Gravity::BottomRight, at, flip_x)
    } else if rect.x + rect.width <= t.x + 0.5 {
        let anchor = wide((t.x - rect.x - rect.width).round());
        let at = offset(0., rect.y - t.y);
        (anchor, Anchor::TopLeft, Gravity::BottomLeft, at, flip_x)
    } else {
        point
    }
}

/// Show popup `root` at `rect` (`parent`'s points), opened against
/// `trigger` if it has one, in an `xdg_popup` of `parent`'s, or of popup `of`
/// (its root, `xdg_surface` and box) where it opens from one, configured and
/// ready to draw.
#[allow(clippy::too_many_arguments)]
fn show(
    client: &mut Client,
    parent: &Window,
    of: Option<(NodeId, XdgSurface, Rect)>,
    root: NodeId,
    (rect, panel): (Rect, Rect),
    trigger: Option<Rect>,
    scale: f32,
    zoom: f32,
    context: &GpuContext,
) -> Option<Shown> {
    // The protocol's coordinates are the surfaces', the scene's at the
    // window's zoom: what is asked, and what the compositor reports back.
    let zoomed = |r: Rect| super::window_rect(r, zoom);
    // `scale` is the window's content scale: its output's times its zoom.
    let display = scale / zoom;
    let (scene_rect, scene_panel) = (rect, panel);
    let (rect, panel, trigger) = (zoomed(rect), zoomed(panel), trigger.map(zoomed));
    let qh = client.queue.handle();
    let (compositor, wm_base) = (
        client.state.compositor.clone()?,
        client.state.wm_base.clone()?,
    );
    // Positions relative to the parent's window geometry: a popup's is its
    // surface, which shows its box from the box's top-left.
    let (parent_xdg, (gx, gy)) = match &of {
        Some((_, xdg, rect)) => {
            let rect = zoomed(*rect);
            (xdg.clone(), (rect.x.round() as i32, rect.y.round() as i32))
        }
        None => {
            let (surface, origin) = xdg_surface(parent)?;
            // SAFETY: winit's live xdg_surface on this display, which
            // `parent` holds for this call; the proxy made of it goes when
            // this returns (`get_popup` takes it as an argument only).
            let xdg =
                unsafe { ObjectId::from_ptr(XdgSurface::interface(), surface.as_ptr().cast()) }
                    .ok()
                    .and_then(|id| XdgSurface::from_id(&client.connection, id).ok())?;
            (xdg, origin)
        }
    };
    // Asked by its panel (its window geometry), the surface round it.
    let (rules, asked, trigger) = Rules::of(panel, trigger, (gx, gy));
    let ((width, height), geometry) = surface_and_geometry(rect, panel, rules.size);
    let surface = compositor.create_surface(&qh, ());
    let viewport = client
        .state
        .viewporter
        .as_ref()
        .map(|viewporter| viewporter.get_viewport(&surface, &qh, ()));
    let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
    let positioner = rules.positioner(&wm_base, &qh);
    let popup = xdg.get_popup(Some(&parent_xdg), &positioner, &qh, ());
    positioner.destroy();
    xdg.set_window_geometry(geometry.0, geometry.1, geometry.2, geometry.3);
    // The surface's size in the output's pixels.
    let physical = (
        (rect.width * display).round().max(1.) as u32,
        (rect.height * display).round().max(1.) as u32,
    );
    match &viewport {
        Some(viewport) => viewport.set_destination(width, height),
        None => surface.set_buffer_scale(display.round().max(1.) as i32),
    }
    surface.commit();
    // Configured before its first frame, as the protocol asks.
    let xdg_id = xdg.id();
    for _ in 0..4 {
        if client.state.configured.contains_key(&xdg_id) {
            break;
        }
        client.queue.roundtrip(&mut client.state).ok()?;
    }
    let serial = client.state.configured.remove(&xdg_id)?;
    xdg.ack_configure(serial);
    client.state.live.insert(xdg_id);
    let window = RawWindowHandle::Wayland(WaylandWindowHandle::new(std::ptr::NonNull::new(
        surface.id().as_ptr().cast(),
    )?));
    zgx_gpu::present_log::log(|| {
        format!(
            "popup show {root:?} at {rect:?} buffer {}x{} scale {scale}",
            physical.0, physical.1
        )
    });
    // SAFETY: the display outlives the popup: a `Shown` goes with its
    // window's host, before the event loop closes the display
    // (`WindowManager::exiting`). Its `wl_surface` is destroyed only after
    // the renderer (`Shown::destroy`; dropped whole, the renderer is its
    // first field and the surface proxy's drop destroys nothing).
    let renderer = unsafe {
        GpuRenderer::for_raw_surface(
            client.display,
            window,
            physical.0,
            physical.1,
            // Pixels per scene point.
            scale,
            context,
        )
    };
    let renderer = match renderer {
        Ok(renderer) => renderer,
        Err(_) => {
            popup.destroy();
            xdg.destroy();
            if let Some(viewport) = viewport {
                viewport.destroy();
            }
            surface.destroy();
            return None;
        }
    };
    Some(Shown {
        renderer,
        popup,
        xdg,
        viewport,
        surface,
        rect: scene_rect,
        rules,
        panel: scene_panel,
        surface_size: (width, height),
        geometry,
        asked,
        trigger,
        revision: None,
        fault: None,
        parent: of.map(|(holder, ..)| holder),
        input: None,
    })
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "xdg_wm_base" => state.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
                "wp_viewporter" => state.viewporter = Some(registry.bind(name, 1, qh, ())),
                "wl_seat" => state
                    .seats
                    .push((registry.bind(name, version.min(5), qh, ()), None)),
                _ => {}
            }
        }
    }
}
impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        else {
            return;
        };
        let Some((_, pointer)) = state.seats.iter_mut().find(|(each, _)| each == seat) else {
            return;
        };
        match (capabilities.contains(Capability::Pointer), pointer.take()) {
            (true, None) => *pointer = Some(seat.get_pointer(qh, ())),
            (true, kept) => *pointer = kept,
            (false, Some(old)) if old.version() >= 3 => old.release(),
            (false, _) => {}
        }
    }
}
impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let at = |target: &Target, x: f64, y: f64| {
            super::scene_point(target.origin, (x as f32, y as f32), target.zoom)
        };
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                let Some(target) = state.targets.get(&surface.id()) else {
                    // Onto the window (zgx's pointer hears its surfaces
                    // too): winit tells it, so no leave from a popup goes
                    // after.
                    state.leaving = None;
                    state.over = None;
                    return;
                };
                // From one popup straight onto another: no leave between.
                state.leaving = None;
                let point = at(target, surface_x, surface_y);
                (target.send)(PopupInput::Moved {
                    x: point.0,
                    y: point.1,
                });
                state.over = Some((surface.id(), point));
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                let Some((id, point)) = &mut state.over else {
                    return;
                };
                let Some(target) = state.targets.get(id) else {
                    return;
                };
                *point = at(target, surface_x, surface_y);
                (target.send)(PopupInput::Moved {
                    x: point.0,
                    y: point.1,
                });
            }
            wl_pointer::Event::Leave { surface, .. } => {
                // Told as the dispatch ends (`dispatch`): an enter onto the
                // window or another popup may follow.
                if let Some(target) = state.targets.get(&surface.id()) {
                    state.leaving = Some(target.send.clone());
                }
                state.over = None;
            }
            wl_pointer::Event::Button {
                button,
                state: pressed,
                ..
            } => {
                let Some((id, (x, y))) = &state.over else {
                    return;
                };
                let Some(target) = state.targets.get(id) else {
                    return;
                };
                // Linux's input codes (BTN_LEFT and on).
                let button = match button {
                    0x110 => MouseButton::Left,
                    0x111 => MouseButton::Right,
                    0x112 => MouseButton::Middle,
                    0x116 => MouseButton::Back,
                    0x115 => MouseButton::Forward,
                    other => MouseButton::Other(other as u16),
                };
                (target.send)(PopupInput::Button {
                    x: *x,
                    y: *y,
                    button,
                    pressed: matches!(pressed, WEnum::Value(wl_pointer::ButtonState::Pressed)),
                });
            }
            wl_pointer::Event::Axis {
                axis: WEnum::Value(axis),
                value,
                ..
            } => {
                let Some((id, (x, y))) = &state.over else {
                    return;
                };
                let Some(target) = state.targets.get(id) else {
                    return;
                };
                // Wayland's sign is the inverse of winit's.
                let (dx, dy) = match axis {
                    wl_pointer::Axis::HorizontalScroll => (-value, 0.),
                    _ => (0., -value),
                };
                (target.send)(PopupInput::Wheel {
                    x: *x,
                    y: *y,
                    delta: MouseScrollDelta::PixelDelta(
                        LogicalPosition::new(dx, dy).to_physical(target.scale),
                    ),
                    phase: TouchPhase::Moved,
                });
            }
            _ => {}
        }
    }
}
impl Dispatch<XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        wm_base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}
impl Dispatch<XdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        xdg: &XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            if state.live.contains(&xdg.id()) {
                xdg.ack_configure(serial);
            } else {
                state.configured.insert(xdg.id(), serial);
            }
        }
    }
}
impl Dispatch<XdgPopup, ()> for State {
    fn event(
        state: &mut Self,
        popup: &XdgPopup,
        event: xdg_popup::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_popup::Event::Configure {
                x,
                y,
                width,
                height,
            } => {
                zgx_gpu::present_log::log(|| {
                    format!("popup configured at {x},{y} {width}x{height}")
                });
                state.placed.insert(popup.id(), (x, y, width, height));
            }
            xdg_popup::Event::Repositioned { token } => {
                state.repositioned.insert(popup.id(), token);
            }
            xdg_popup::Event::PopupDone => state.done.push(popup.id()),
            _ => {}
        }
    }
}
impl Dispatch<WlCallback, ObjectId> for State {
    fn event(
        state: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        surface: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The compositor took the frame: the next may be presented.
        if let wl_callback::Event::Done { .. } = event {
            state.framing.remove(surface);
        }
    }
}
delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ignore WlRegion);
delegate_noop!(State: ignore XdgPositioner);
delegate_noop!(State: ignore WpViewporter);
delegate_noop!(State: ignore WpViewport);

#[cfg(test)]
mod tests {
    use super::*;

    /// A chip with a tooltip card that paints a wide shadow, hovered so
    /// the card shows as a popup root: the card's box, where the popup
    /// root shows (its shadow's ink), and where it takes the pointer.
    fn hovered_card(interactive: bool) -> (Rect, Rect, Option<Rect>) {
        use zgx::{
            compose::{Tooltip, prelude::*},
            input::InputEvent,
            scene::{BoxShadow, Transform},
            widgets::Ui,
        };
        let mut ui = Ui::new(400., 300.);
        ui.set_popup_screens(vec![Rect::new(-1e5, -1e5, 2e5, 2e5)]);
        let card = column().id("card").w(160.).h(60.).shadow(BoxShadow {
            color: rgba(0x00000066),
            offset: Transform { x: 0., y: 20. },
            blur_radius: 56.,
            spread: 0.,
        });
        let view = ui.mount(
            column().p(150.).child(
                row()
                    .id("chip")
                    .size(24., 24.)
                    .tooltip(Tooltip::from(card).interactive(interactive)),
            ),
        );
        ui.prepare_frame();
        // (No task runner: it shows at once.)
        ui.dispatch(InputEvent::PointerMove { x: 162., y: 162. });
        ui.prepare_frame();
        let scene = ui.scene.borrow();
        let root = *scene.popups().first().expect("the card shows as a popup");
        let shown = scene.popup_bounds(root).expect("it paints");
        let card = scene.bounds(view.find("card").expect("the card"));
        let takes = |node| {
            let options = ui.input.options(node);
            if options.is_some_and(|options| options.pass_through) {
                Some(false)
            } else if ui.input.has_listeners(node)
                || options.is_some_and(|options| options.disabled)
            {
                Some(true)
            } else {
                None
            }
        };
        // Its window geometry is the card's box, the shadow outside it.
        assert_eq!(panel_of(&scene, root, shown), card);
        (card, shown, input_rect(&scene, root, shown, &takes))
    }

    #[test]
    fn a_popups_window_geometry_is_its_panel_its_shadow_outside() {
        // A 248x91 card in a 416x259 surface, 84 in from its left and 64
        // down (a shadow offset 20 down).
        let surface = Rect::new(395., 283., 416., 259.);
        let card = Rect::new(479., 347., 248., 91.);
        assert_eq!(
            surface_and_geometry(surface, card, (248, 91)),
            ((416, 259), (84, 64, 248, 91))
        );
        // Asked against its trigger by its panel: above it, 8 apart, it
        // opens above it, flipped at the output's edge; by its whole
        // surface (the shadow overlapping the trigger) it was a point
        // anchor, only slid.
        let trigger = Rect::new(703., 446., 24., 24.);
        let by_panel = position(card, Some(trigger));
        assert_eq!(
            (by_panel.1, by_panel.2),
            (Anchor::TopLeft, Gravity::TopRight)
        );
        assert!(by_panel.4.contains(ConstraintAdjustment::FlipY));
        assert!(
            !position(surface, Some(trigger))
                .4
                .contains(ConstraintAdjustment::FlipY)
        );
    }

    #[test]
    fn a_tooltip_takes_no_pointer_and_a_cards_shadow_none() {
        // A tooltip lets the pointer through: its surface takes none, so
        // it never takes the pointer off its trigger as it shows.
        let (card, shown, input) = hovered_card(false);
        assert!(
            shown.width > card.width + 50. && shown.height > card.height + 50.,
            "the shadow reaches past the card: {shown:?} for {card:?}"
        );
        assert_eq!(input, None);
        // A hover card takes the pointer over the card, not its shadow.
        let (card, shown, input) = hovered_card(true);
        assert_eq!(
            input,
            Some(Rect::new(
                card.x - shown.x,
                card.y - shown.y,
                card.width,
                card.height
            ))
        );
    }

    #[test]
    fn a_popup_opens_against_its_trigger_on_the_side_the_scene_chose() {
        let trigger = Rect::new(16., 396., 120., 28.);
        // Below, 4 points under it: the gap is the anchor rectangle's, on
        // both sides, so a flip opens it as far above.
        let grown = Rect::new(16., 392., 120., 36.);
        let below = position(Rect::new(16., 428., 200., 202.), Some(trigger));
        assert_eq!(
            (below.0, below.1, below.2, below.3),
            (grown, Anchor::BottomLeft, Gravity::BottomRight, (0, 0))
        );
        assert!(below.4.contains(ConstraintAdjustment::FlipY));
        // Too tall for the output, cut to it.
        assert!(below.4.contains(ConstraintAdjustment::ResizeY));
        let above = position(Rect::new(16., 190., 200., 202.), Some(trigger));
        assert_eq!(
            (above.0, above.1, above.2, above.3),
            (grown, Anchor::TopLeft, Gravity::TopRight, (0, 0))
        );
        // A submenu beside its item, 2 points off it, its top 4 above the
        // item's: free to flip left.
        let item = Rect::new(20., 100., 180., 24.);
        let beside = position(Rect::new(202., 96., 160., 80.), Some(item));
        assert_eq!(
            (beside.0, beside.1, beside.2, beside.3),
            (
                Rect::new(18., 100., 184., 24.),
                Anchor::TopRight,
                Gravity::BottomRight,
                (0, -4)
            )
        );
        assert!(beside.4.contains(ConstraintAdjustment::FlipX));
        // Over its trigger (`open_over`), or with none: at its place.
        let over = position(Rect::new(10., 390., 200., 202.), Some(trigger));
        assert_eq!((over.0, over.3), (Rect::new(10., 390., 1., 1.), (0, 0)));
        assert_eq!(
            position(Rect::new(5., 6., 7., 8.), None).0,
            Rect::new(5., 6., 1., 1.)
        );
    }

    #[test]
    fn a_popup_growing_against_its_trigger_asks_only_a_new_size() {
        // In the window geometry, which starts 30 above the content (a title
        // bar winit draws).
        let trigger = Rect::new(16., 426., 120., 28.);
        let (small, asked, seen) =
            Rules::of(Rect::new(16., 458., 200., 120.), Some(trigger), (0, -30));
        assert_eq!(asked, Rect::new(16., 488., 200., 120.));
        assert_eq!(seen, Some(Rect::new(16., 456., 120., 28.)));
        assert_eq!(small.size, (200, 120));
        assert_eq!(small.anchor_rect, (16, 452, 120, 36));
        // Opened out, below the trigger as before: the same rules but its
        // size, whole points rounded up. Repositioned in place.
        let (large, ..) = Rules::of(Rect::new(16., 458., 320.4, 240.), Some(trigger), (0, -30));
        assert_eq!(large.size, (321, 240));
        assert_eq!(
            Rules {
                size: small.size,
                ..large
            },
            small
        );
        // The same box asks the same: nothing to do.
        assert_eq!(
            Rules::of(Rect::new(16., 458., 200., 120.), Some(trigger), (0, -30)).0,
            small
        );
    }

    #[test]
    fn a_popup_frame_waits_for_the_compositor_to_take_its_last_a_while_at_most() {
        let now = Instant::now();
        // Its last taken (the callback came), or none yet: drawn at once.
        assert_eq!(waits(None, now), None);
        // Presented 3 ms ago, not yet taken: it waits, until FRAME_WAIT on.
        let presented = now - Duration::from_millis(3);
        assert_eq!(waits(Some(presented), now), Some(presented + FRAME_WAIT));
        // A callback that never came stalls it no longer than that.
        assert_eq!(waits(Some(now - FRAME_WAIT), now), None);
        assert_eq!(waits(Some(now - 2 * FRAME_WAIT), now), None);
    }

    #[test]
    fn a_popup_put_on_its_triggers_other_side_is_seen_flipped() {
        let trigger = Rect::new(16., 396., 120., 28.);
        let below = Rect::new(16., 436., 200., 120.);
        // Put above instead, 12 points off: flipped down the screen.
        let above = Rect::new(16., 264., 200., 120.);
        assert_eq!(sides_turned(below, above, trigger), (false, true));
        assert_eq!(sides_turned(above, below, trigger), (false, true));
        // Slid along the trigger, or where it asked: not flipped.
        let slid = Rect::new(-40., 436., 200., 120.);
        assert_eq!(sides_turned(below, slid, trigger), (false, false));
        assert_eq!(sides_turned(below, below, trigger), (false, false));
        // Beside it, put on its left.
        let right = Rect::new(140., 396., 160., 80.);
        let left = Rect::new(-148., 396., 160., 80.);
        assert_eq!(sides_turned(right, left, trigger), (true, false));
    }
}
