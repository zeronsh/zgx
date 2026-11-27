//! Web views whose pages are Ladybird's (`WebEngine::Ladybird`, the
//! `ladybird` feature). Ladybird runs in a process of zgx's,
//! `zgx-ladybird-host` (crates/zgx-ladybird-sys), which links its LibWebView
//! and starts its helpers (WebContent, the Compositor, RequestServer, ...):
//! a crash there ends pages, not the app. The Compositor paints each page
//! into backing stores in shared memory; the host sends the app each store
//! once and then which one shows a new frame and what changed in it, and the
//! page's box (a GPU view) uploads only that into its texture. A page that
//! draws nothing sends nothing: idle pages cost no frames and no CPU.
//!
//! Where there's no host (not built, or it won't start), pages show their
//! placeholders.

mod client;
mod keys;
mod protocol;

pub(crate) use client::{WakerGuard, add_waker};

use client::{Delivery, Frame, Host};
use protocol::{FromHost, Message, ToHost};
use std::{
    any::Any,
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
    sync::{Arc, Mutex},
    time::Instant,
};
use zgx::{
    compose::{component, div, gpu_view},
    input::{EventContext, InputEvent, PointerButton},
    scene::{NodeId, Scene},
    style::Styled,
    web_view::{
        PageViews, WeakWebView, WebEngine, WebView, WebViewBackend, WebViewCommand, WebViewEvent,
    },
};
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

thread_local! {
    /// The UI thread's pages, by their web view's id.
    static PAGES: RefCell<HashMap<u64, Weak<Page>>> = RefCell::new(HashMap::new());
    /// The system's clipboard, which Ladybird's pages copy to and paste from
    /// (one for the app: a window's, the last to make pages' boxes).
    static CLIPBOARD: RefCell<Option<Rc<RefCell<Option<crate::clipboard::Clipboard>>>>> = const { RefCell::new(None) };
}

/// Run what the host's thread delivered (the UI thread's safe point).
pub(crate) fn deliver() {
    for delivery in client::take_deliveries() {
        match delivery {
            Delivery::Frame(view) => {
                if let Some(page) = page(view) {
                    page.gpu.invalidate();
                }
            }
            Delivery::Message(message) => take(message),
            Delivery::Snapshot {
                view,
                request,
                image,
            } => {
                let reply =
                    page(view).and_then(|page| page.snapshots.borrow_mut().remove(&request));
                if let Some(reply) = reply {
                    reply(image.and_then(|(width, height, pixels, premultiplied)| {
                        image_data(width, height, pixels, premultiplied)
                    }));
                }
            }
            Delivery::Gone => {
                let pages: Vec<Rc<Page>> =
                    PAGES.with(|pages| pages.borrow().values().filter_map(Weak::upgrade).collect());
                for page in pages {
                    if let Some(web) = page.web.upgrade() {
                        web.emit(WebViewEvent::ProcessTerminated);
                    }
                }
            }
        }
    }
}

/// A snapshot's pixels as an image.
fn image_data(
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    premultiplied: bool,
) -> zgx::web_view::SnapshotResult {
    let image = if premultiplied {
        zgx::image::ImageData::new_premultiplied(width, height, pixels)
    } else {
        zgx::image::ImageData::new(width, height, pixels)
    };
    image
        .map(Arc::new)
        .map_err(|_| "the snapshot had no pixels".to_owned())
}

fn page(view: u64) -> Option<Rc<Page>> {
    PAGES.with(|pages| pages.borrow().get(&view).and_then(Weak::upgrade))
}

/// A host message about a page's state, on the UI thread.
fn take(message: FromHost) {
    match &message {
        FromHost::ClipboardWrite { text, html } => {
            CLIPBOARD.with(|clipboard| {
                let clipboard = clipboard.borrow();
                let Some(clipboard) = clipboard.as_ref() else {
                    return;
                };
                if let Some(clipboard) = clipboard.borrow_mut().as_mut() {
                    let _ = if html.is_empty() {
                        clipboard.set_text(text.clone())
                    } else {
                        clipboard.set_rich_text(text.clone(), html.clone())
                    };
                }
            });
            return;
        }
        FromHost::ClipboardRead { request } => {
            let (text, html) = CLIPBOARD.with(|clipboard| {
                let clipboard = clipboard.borrow();
                let mut held = clipboard.as_ref().map(|clipboard| clipboard.borrow_mut());
                match held.as_mut().and_then(|held| held.as_mut()) {
                    Some(clipboard) => (
                        clipboard.get_text().unwrap_or_default(),
                        clipboard.get_html().unwrap_or_default(),
                    ),
                    None => (String::new(), String::new()),
                }
            });
            if let Some(host) = client::host() {
                host.send(
                    Message::new(ToHost::ClipboardData, 0)
                        .u64(*request)
                        .str(&text)
                        .str(&html),
                );
            }
            return;
        }
        _ => {}
    }
    let view = match &message {
        FromHost::Title { view, .. }
        | FromHost::Url { view, .. }
        | FromHost::Loading { view, .. }
        | FromHost::LoadFinished { view, .. }
        | FromHost::History { view, .. }
        | FromHost::Cursor { view, .. }
        | FromHost::NewWindow { view, .. }
        | FromHost::Crashed { view, .. }
        | FromHost::Editing { view, .. }
        | FromHost::LinkHovered { view, .. }
        | FromHost::ThemeColor { view, .. }
        | FromHost::Favicon { view, .. } => *view,
        FromHost::ClipboardWrite { .. } | FromHost::ClipboardRead { .. } => return,
        FromHost::Ready
        | FromHost::Store { .. }
        | FromHost::Present { .. }
        | FromHost::Snapshot { .. } => return,
    };
    let Some(page) = page(view) else { return };
    let Some(web) = page.web.upgrade() else {
        return;
    };
    match message {
        FromHost::Title { title, .. } => {
            web.update_state(|state| state.title = title.clone());
            web.emit(WebViewEvent::TitleChanged { title });
        }
        FromHost::Url { url, .. } => {
            web.update_state(|state| state.url = url.clone());
            web.emit(WebViewEvent::UrlChanged { url });
        }
        FromHost::Loading { loading, .. } => {
            web.update_state(|state| {
                state.loading = loading;
                state.progress = if loading { 0.1 } else { 1. };
                if loading {
                    state.error = None;
                }
            });
            if loading {
                let url = web.url();
                web.emit(WebViewEvent::NavigationStarted { url });
            }
        }
        FromHost::LoadFinished { url, .. } => {
            web.update_state(|state| {
                state.url = url.clone();
                state.loading = false;
                state.progress = 1.;
            });
            web.emit(WebViewEvent::NavigationCommitted { url: url.clone() });
            web.emit(WebViewEvent::NavigationFinished { url });
        }
        FromHost::History { back, forward, .. } => web.update_state(|state| {
            state.can_go_back = back;
            state.can_go_forward = forward;
        }),
        FromHost::Cursor { cursor, .. } => page.set_cursor(cursor_of(cursor)),
        FromHost::NewWindow { url, .. } => web.emit(WebViewEvent::NewWindowRequested { url }),
        FromHost::Crashed { reason, .. } => {
            web.update_state(|state| {
                state.loading = false;
                state.error = Some("The page stopped responding.".into());
                state.termination = Some(match reason {
                    0 => zgx::web_view::Termination::Crashed,
                    1 => zgx::web_view::Termination::OutOfMemory,
                    2 => zgx::web_view::Termination::Killed,
                    _ => zgx::web_view::Termination::Unknown,
                });
            });
            web.emit(WebViewEvent::ProcessTerminated);
        }
        FromHost::Editing { editing, .. } => page.editing.set(editing),
        FromHost::LinkHovered { url, .. } => {
            web.update_state(|state| state.hovered_link = (!url.is_empty()).then_some(url));
        }
        FromHost::Favicon { image, .. } => {
            if !web.options().favicons {
                return;
            }
            let favicon = image.and_then(|(width, height, pitch, format, premultiplied, bytes)| {
                let pixels = client::rgba_rows(&bytes, width, height, pitch, format);
                image_data(
                    width,
                    height,
                    pixels,
                    premultiplied && matches!(format, 2 | 4),
                )
                .ok()
            });
            web.update_state(|state| state.favicon = favicon);
            web.emit(WebViewEvent::FaviconChanged);
        }
        FromHost::ThemeColor { color, .. } => web.update_state(|state| {
            state.theme_color = color.map(|(r, g, b, a)| zgx::scene::Color(r, g, b, a));
        }),
        FromHost::Ready
        | FromHost::Store { .. }
        | FromHost::Present { .. }
        | FromHost::Snapshot { .. }
        | FromHost::ClipboardWrite { .. }
        | FromHost::ClipboardRead { .. } => {}
    }
}

/// zgx's cursor for Ladybird's `Gfx::StandardCursor`.
fn cursor_of(standard: u8) -> zgx::cursor::Cursor {
    use zgx::cursor::Cursor;
    match standard {
        3 => Cursor::Crosshair,
        4 => Cursor::Text,
        5 => Cursor::EwResize,
        6 => Cursor::NsResize,
        7 => Cursor::NwseResize,
        8 => Cursor::NeswResize,
        9 => Cursor::ColResize,
        10 => Cursor::RowResize,
        11 => Cursor::Pointer,
        12 => Cursor::Help,
        13 => Cursor::Grab,
        14 => Cursor::Grabbing,
        15 => Cursor::Copy,
        16 => Cursor::Move,
        17 => Cursor::Wait,
        18 => Cursor::NotAllowed,
        20 => Cursor::ZoomIn,
        _ => Cursor::Default,
    }
}

/// What gets a full-page snapshot's answer.
type SnapshotReply = Box<dyn FnOnce(zgx::web_view::SnapshotResult)>;

/// A press: when, where (window points), its button's bit and its count.
type Press = (Instant, f32, f32, u8, u8);

/// A Ladybird page on the UI thread: its box's state and its frame.
struct Page {
    id: u64,
    web: WeakWebView,
    host: Arc<Host>,
    gpu: zgx::gpu_view::GpuView,
    node: Cell<Option<NodeId>>,
    scene: RefCell<Weak<RefCell<Scene>>>,
    /// Device pixels a point, as last drawn.
    scale: Rc<Cell<f32>>,
    /// Buttons held, as Ladybird's bits.
    buttons: Cell<u8>,
    modifiers: Cell<u8>,
    /// The last press, for double and triple clicks.
    last_press: Cell<Option<Press>>,
    /// The character the last key typed, which zgx's `Text` repeats.
    typed: RefCell<Option<String>>,
    editing: Cell<bool>,
    /// What gets each full-page snapshot's answer, by request.
    snapshots: RefCell<HashMap<u64, SnapshotReply>>,
    next_snapshot: Cell<u64>,
}
impl Page {
    fn send(&self, message: Message) {
        self.host.send(message);
    }
    fn message(&self, kind: ToHost) -> Message {
        Message::new(kind, self.id)
    }
    fn set_cursor(&self, cursor: zgx::cursor::Cursor) {
        if let (Some(node), Some(scene)) = (self.node.get(), self.scene.borrow().upgrade())
            && let Ok(mut scene) = scene.try_borrow_mut()
            && scene.contains(node)
        {
            scene.set_cursor(node, Some(cursor));
        }
    }
    /// A point of the window in the page's device pixels.
    fn local(&self, x: f32, y: f32) -> Option<(f64, f64)> {
        let (node, scene) = (self.node.get()?, self.scene.borrow().upgrade()?);
        let scene = scene.try_borrow().ok()?;
        if !scene.contains(node) {
            return None;
        }
        let bounds = scene.bounds(node);
        let shown = scene.window_rect(node, bounds);
        let scale = self.scale.get() as f64;
        let sx = bounds.width / shown.width.max(0.001);
        let sy = bounds.height / shown.height.max(0.001);
        Some((
            ((x - shown.x) * sx) as f64 * scale,
            ((y - shown.y) * sy) as f64 * scale,
        ))
    }
    /// Its box's input.
    fn input(&self, event: &mut EventContext) {
        match &event.event {
            InputEvent::PointerDown { x, y, button } => {
                let bit = button_bit(button);
                self.buttons.set(self.buttons.get() | bit);
                let now = Instant::now();
                let clicks = match self.last_press.get() {
                    Some((at, px, py, b, n))
                        if b == bit
                            && now.duration_since(at).as_millis() < 500
                            && (px - x).abs() < 4.
                            && (py - y).abs() < 4. =>
                    {
                        n.saturating_add(1)
                    }
                    _ => 1,
                };
                self.last_press.set(Some((now, *x, *y, bit, clicks)));
                self.pointer(MOUSE_DOWN, *x, *y, bit, clicks);
            }
            InputEvent::PointerUp { x, y, button } => {
                let bit = button_bit(button);
                self.buttons.set(self.buttons.get() & !bit);
                let clicks = self.last_press.get().map_or(1, |p| p.4);
                self.pointer(MOUSE_UP, *x, *y, bit, clicks);
            }
            InputEvent::PointerMove { x, y } => self.pointer(MOUSE_MOVE, *x, *y, 0, 0),
            InputEvent::PointerLeave => {
                self.send(
                    self.message(ToHost::Mouse)
                        .u8(MOUSE_LEAVE)
                        .f64(0.)
                        .f64(0.)
                        .u8(0)
                        .u8(self.buttons.get())
                        .u8(self.modifiers.get())
                        .u8(0),
                );
            }
            InputEvent::Scroll {
                x,
                y,
                delta_x,
                delta_y,
            } => {
                let Some((px, py)) = self.local(*x, *y) else {
                    return;
                };
                let scale = self.scale.get() as f64;
                self.send(
                    self.message(ToHost::Wheel)
                        .f64(px)
                        .f64(py)
                        .f64(*delta_x as f64 * scale)
                        .f64(*delta_y as f64 * scale)
                        .u8(1)
                        .u8(self.modifiers.get()),
                );
                event.stop_propagation();
            }
            InputEvent::KeyDown {
                key,
                modifiers,
                repeat,
            } => {
                let bits = keys::modifiers(*modifiers);
                self.modifiers.set(bits);
                let (code, character) = keys::code(key);
                // A chord types nothing (Ctrl+A selects all).
                let types = character != 0 && !modifiers.control && !modifiers.meta;
                *self.typed.borrow_mut() =
                    types.then(|| char::from_u32(character).unwrap_or_default().to_string());
                self.send(
                    self.message(ToHost::Key)
                        .u8(KEY_DOWN)
                        .u32(code)
                        .u8(bits)
                        .u32(if types { character } else { 0 })
                        .u8(u8::from(*repeat)),
                );
                event.stop_propagation();
            }
            InputEvent::KeyUp { key, modifiers } => {
                let bits = keys::modifiers(*modifiers);
                self.modifiers.set(bits);
                let (code, _) = keys::code(key);
                self.send(
                    self.message(ToHost::Key)
                        .u8(KEY_UP)
                        .u32(code)
                        .u8(bits)
                        .u32(0)
                        .u8(0),
                );
                event.stop_propagation();
            }
            InputEvent::Text(text) => {
                // What a key typed went with it; text without one (a dead
                // key's, a compose sequence's) is committed as an input
                // method's.
                if self.typed.borrow_mut().take().as_deref() != Some(text.as_str()) {
                    self.send(self.message(ToHost::ImeCommit).str(text));
                }
                event.stop_propagation();
            }
            InputEvent::ImePreedit { text, .. } => {
                if text.is_empty() {
                    self.send(self.message(ToHost::ImeUnmark));
                } else {
                    self.send(self.message(ToHost::ImeMark).str(text));
                }
                event.stop_propagation();
            }
            InputEvent::ImeCommit(text) => {
                self.send(self.message(ToHost::ImeCommit).str(text));
                event.stop_propagation();
            }
            InputEvent::Focus => {
                self.send(self.message(ToHost::Focus).u8(1));
                if let Some(web) = self.web.upgrade() {
                    web.update_state(|state| state.focused = true);
                }
            }
            InputEvent::Blur => {
                self.send(self.message(ToHost::Focus).u8(0));
                if let Some(web) = self.web.upgrade() {
                    web.update_state(|state| state.focused = false);
                }
            }
            _ => {}
        }
    }
    fn pointer(&self, kind: u8, x: f32, y: f32, button: u8, clicks: u8) {
        let Some((x, y)) = self.local(x, y) else {
            return;
        };
        self.send(
            self.message(ToHost::Mouse)
                .u8(kind)
                .f64(x)
                .f64(y)
                .u8(button)
                .u8(self.buttons.get())
                .u8(self.modifiers.get())
                .u8(clicks),
        );
    }
}

// Ladybird's `Web::MouseEvent::Type` and `Web::KeyEvent::Type`.
const MOUSE_DOWN: u8 = 0;
const MOUSE_UP: u8 = 1;
const MOUSE_MOVE: u8 = 2;
const MOUSE_LEAVE: u8 = 3;
const KEY_DOWN: u8 = 0;
const KEY_UP: u8 = 1;

/// Ladybird's bit for a button (`Web::UIEvents::MouseButton`).
fn button_bit(button: &PointerButton) -> u8 {
    match button {
        PointerButton::Primary => 1,
        PointerButton::Secondary => 2,
        PointerButton::Middle => 4,
        PointerButton::Other(_) => 0,
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        PAGES.with(|pages| {
            pages.borrow_mut().remove(&self.id);
        });
        self.host.forget(self.id);
    }
}

/// The controller's side: what it asks of the page.
struct LadybirdWebView(Rc<Page>);
impl WebViewBackend for LadybirdWebView {
    fn run(&self, command: WebViewCommand) {
        let page = &self.0;
        match command {
            WebViewCommand::LoadUrl(url) => page.send(page.message(ToHost::LoadUrl).str(&url)),
            WebViewCommand::LoadHtml { html, .. } => {
                page.send(page.message(ToHost::LoadHtml).str(&html))
            }
            WebViewCommand::Reload => page.send(page.message(ToHost::Reload)),
            WebViewCommand::Stop => page.send(page.message(ToHost::Stop)),
            WebViewCommand::GoBack => page.send(page.message(ToHost::Traverse).i32(-1)),
            WebViewCommand::GoForward => page.send(page.message(ToHost::Traverse).i32(1)),
            WebViewCommand::GoToHistory(n) => page.send(page.message(ToHost::Traverse).i32(n)),
            WebViewCommand::Evaluate { script, reply } => {
                page.send(page.message(ToHost::RunScript).str(&script));
                if let Some(reply) = reply {
                    reply(Err(
                        "Ladybird pages don't return a script's result yet".into()
                    ));
                }
            }
            WebViewCommand::Focus(focus) => {
                page.send(page.message(ToHost::Focus).u8(u8::from(focus)))
            }
            WebViewCommand::OpenDevtools => {}
            WebViewCommand::SetZoom(zoom) => page.send(page.message(ToHost::Zoom).f64(zoom)),
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn edit(&self, command: zgx::web_view::EditCommand) -> Result<(), String> {
        use zgx::web_view::EditCommand;
        let code = match command {
            EditCommand::Copy => 0,
            EditCommand::Cut => 1,
            EditCommand::Paste => 2,
            EditCommand::SelectAll => 3,
            EditCommand::Undo => 4,
            EditCommand::Redo => 5,
            _ => return Err("Ladybird has no such editing command".into()),
        };
        self.0.send(self.0.message(ToHost::Edit).u8(code));
        Ok(())
    }
    fn reload_ignoring_cache(&self) -> Result<(), String> {
        self.0.send(self.0.message(ToHost::ReloadIgnoringCache));
        Ok(())
    }
    fn snapshot(
        &self,
        area: zgx::web_view::SnapshotArea,
        reply: Box<dyn FnOnce(zgx::web_view::SnapshotResult)>,
    ) {
        let page = &self.0;
        match area {
            // What the page shows is the front store zgx has mapped already.
            zgx::web_view::SnapshotArea::Visible => {
                let front = page.host.frame(page.id).lock().unwrap().front.clone();
                reply(match front {
                    Some((_, store)) => image_data(
                        store.width,
                        store.height,
                        client::rgba(&store),
                        matches!(store.format, 2 | 4),
                    ),
                    None => Err("the page hasn't drawn yet".into()),
                });
            }
            // The whole document, which Ladybird's WebContent paints for it.
            zgx::web_view::SnapshotArea::FullPage => {
                let request = page.next_snapshot.get() + 1;
                page.next_snapshot.set(request);
                page.snapshots.borrow_mut().insert(request, reply);
                page.send(page.message(ToHost::Snapshot).u64(request));
            }
        }
    }
}

/// The page for `web`, made (and its view made in the host) on first ask.
fn backend(web: &WebView, host: Arc<Host>) -> Rc<Page> {
    if let Some(backend) = web.backend()
        && let Some(page) = backend.as_any().downcast_ref::<LadybirdWebView>()
    {
        return page.0.clone();
    }
    let frame = host.frame(web.id());
    let scale = Rc::new(Cell::new(1.));
    let page = Rc::new(Page {
        id: web.id(),
        web: web.downgrade(),
        host: host.clone(),
        gpu: zgx_gpu::viewport::view(Painter {
            id: web.id(),
            host: host.clone(),
            frame,
            scale: scale.clone(),
            asked: None,
            gpu: None,
        }),
        node: Cell::new(None),
        scene: RefCell::new(Weak::new()),
        scale,
        buttons: Cell::new(0),
        modifiers: Cell::new(0),
        last_press: Cell::new(None),
        typed: RefCell::new(None),
        editing: Cell::new(false),
        snapshots: RefCell::new(HashMap::new()),
        next_snapshot: Cell::new(0),
    });
    PAGES.with(|pages| pages.borrow_mut().insert(web.id(), Rc::downgrade(&page)));
    // Made at a size of its own until its box draws (`Painter`), so what the
    // controller queued (its first URL) goes to it now.
    host.send(
        Message::new(ToHost::CreateView, web.id())
            .u32(800)
            .u32(600)
            .f64(1.),
    );
    web.attach(Rc::new(LadybirdWebView(page.clone())));
    page
}

/// The service making pages' boxes: Ladybird's pages here (or their
/// placeholders where there's no host), the others by `others` (WebKitGTK's
/// on Linux), or as native views where `others` makes none.
pub(crate) fn page_views(
    others: Option<PageViews>,
    clipboard: Rc<RefCell<Option<crate::clipboard::Clipboard>>>,
) -> PageViews {
    CLIPBOARD.with(|held| *held.borrow_mut() = Some(clipboard));
    PageViews::new(move |web| {
        if web.options().engine != WebEngine::Ladybird {
            return others.as_ref().and_then(|others| others.make(web));
        }
        let Some(host) = client::host() else {
            return Some(zgx::web_view::placeholder_view(web));
        };
        let page = backend(web, host);
        Some(component(move |_| {
            let (input, mounted) = (page.clone(), page.clone());
            div()
                .focusable(true)
                .overflow_hidden()
                .on_event(move |event| input.input(event))
                .accessible_label("Web page")
                .on_mount(move |handle| {
                    let page = mounted;
                    page.node.set(Some(handle.node()));
                    *page.scene.borrow_mut() = handle
                        .scene()
                        .as_ref()
                        .map_or_else(Weak::new, Rc::downgrade);
                    page.send(page.message(ToHost::Visible).u8(1));
                    handle.retain(Shown(page.clone()));
                })
                .child(
                    gpu_view("", &page.gpu)
                        .absolute()
                        .left(0.)
                        .top(0.)
                        .w_full()
                        .h_full(),
                )
        }))
    })
}

/// While a page's box is in the scene: hidden once it goes.
struct Shown(Rc<Page>);
impl Drop for Shown {
    fn drop(&mut self) {
        self.0.node.set(None);
        self.0.send(self.0.message(ToHost::Visible).u8(0));
    }
}

/// The page's box's painter: asks the host for the box's size and scale,
/// and uploads what changed of the frame shown into its texture.
struct Painter {
    id: u64,
    host: Arc<Host>,
    frame: Arc<Mutex<Frame>>,
    scale: Rc<Cell<f32>>,
    /// The size and scale the page was last asked for.
    asked: Option<((u32, u32), f32)>,
    gpu: Option<Blit>,
}
struct Blit {
    pipelines: [wgpu::RenderPipeline; 2],
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The texture, its bind group, size and Ladybird format.
    texture: Option<(wgpu::Texture, wgpu::BindGroup, (u32, u32), u8)>,
}
impl Viewport for Painter {
    fn render(&mut self, frame: &mut ViewportFrame<'_>) {
        self.scale.set(frame.scale);
        let asked = (frame.size, frame.scale);
        if self.asked != Some(asked) {
            self.asked = Some(asked);
            let (width, height) = (frame.size.0.max(1), frame.size.1.max(1));
            self.host.send(
                Message::new(ToHost::Resize, self.id)
                    .u32(width)
                    .u32(height)
                    .f64(frame.scale as f64),
            );
        }
        let blit = match &mut self.gpu {
            Some(blit) if blit.format == frame.format => blit,
            _ => self.gpu.insert(Blit::new(frame.device, frame.format)),
        };
        let mut done = None;
        {
            let mut shared = self.frame.lock().unwrap();
            shared.told = false;
            if let Some((id, store)) = shared.front.clone() {
                let size = (store.width, store.height);
                let fresh = blit
                    .texture
                    .as_ref()
                    .is_none_or(|(_, _, at, format)| *at != size || *format != store.format);
                if fresh {
                    blit.make_texture(frame.device, size, store.format);
                }
                let damage = if fresh {
                    Some((0, 0, size.0, size.1))
                } else {
                    shared.damage
                };
                shared.damage = None;
                if let (Some((x, y, w, h)), Some((texture, ..))) = (damage, &blit.texture)
                    && w > 0
                    && h > 0
                {
                    let offset = (y * store.pitch + x * 4) as usize;
                    frame.queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d { x, y, z: 0 },
                            aspect: wgpu::TextureAspect::All,
                        },
                        &store.bytes()[offset..],
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(store.pitch),
                            rows_per_image: None,
                        },
                        wgpu::Extent3d {
                            width: w,
                            height: h,
                            depth_or_array_layers: 1,
                        },
                    );
                }
                // `write_texture` copied the bytes: the store may be painted
                // into again once it's no longer the front.
                done = Some(id);
            }
        }
        if let Some(id) = done {
            self.host.send(Message::new(ToHost::Done, self.id).i32(id));
        }
        let mut pass = frame
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ladybird page"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame.target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        if let Some((_, bind, _, format)) = &blit.texture {
            // BGRx and RGBx stores (1, 3) carry no alpha: drawn opaque.
            let opaque = matches!(format, 1 | 3);
            pass.set_pipeline(&blit.pipelines[usize::from(opaque)]);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }
    fn device_changed(&mut self) {
        // A new texture, uploaded whole.
        self.gpu = None;
    }
}
impl Blit {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ladybird page"),
            source: wgpu::ShaderSource::Wgsl(BLIT.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ladybird page"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ladybird page"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ladybird page"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ladybird page"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipelines: [pipeline("fragment"), pipeline("opaque")],
            format,
            layout,
            sampler,
            texture: None,
        }
    }
    fn make_texture(&mut self, device: &wgpu::Device, size: (u32, u32), format: u8) {
        // Ladybird's formats: BGRx 1, BGRA 2, RGBx 3, RGBA 4.
        let texture_format = if matches!(format, 3 | 4) {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            wgpu::TextureFormat::Bgra8Unorm
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ladybird page"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ladybird page"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.texture = Some((texture, bind, size, format));
    }
}
/// The page's texture over the whole box (one triangle covering it).
const BLIT: &str = r#"
@group(0) @binding(0) var page: texture_2d<f32>;
@group(0) @binding(1) var linear: sampler;
struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vertex(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return Out(vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}
@fragment fn fragment(in: Out) -> @location(0) vec4<f32> {
    return textureSample(page, linear, in.uv);
}
@fragment fn opaque(in: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(page, linear, in.uv).rgb, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    //! Against a real host: these run where `ZGX_LADYBIRD_DIR` names an
    //! install of one (zgx-ladybird-sys's), and say they were skipped where
    //! not. Each test has its own views on the one host.
    use super::*;
    use std::time::Duration;

    /// One host test at a time: the window's `deliver` takes every
    /// delivery, other tests' too.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn host_or_skip(test: &str) -> Option<(Arc<Host>, std::sync::MutexGuard<'static, ()>)> {
        if std::env::var_os("ZGX_LADYBIRD_DIR").is_none() {
            eprintln!("{test}: skipped, no ZGX_LADYBIRD_DIR");
            return None;
        }
        let serial = SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Some((
            client::host().expect("ZGX_LADYBIRD_DIR names a host that starts"),
            serial,
        ))
    }

    /// What the painter does with a frame: take it and say it's read.
    fn ack(host: &Host, view: u64) {
        let frame = host.frame(view);
        let id = {
            let mut frame = frame.lock().unwrap();
            frame.told = false;
            frame.damage = None;
            frame.front.as_ref().map(|(id, _)| *id)
        };
        if let Some(id) = id {
            host.send(Message::new(ToHost::Done, view).i32(id));
        }
    }

    /// The deliveries for `view` until `done` says so (or a minute passes),
    /// its frames taken as the painter takes them.
    fn wait_for(view: u64, mut done: impl FnMut(&Delivery) -> bool) -> bool {
        let host = client::host().unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            let mut others = Vec::new();
            let mut found = false;
            for delivery in client::take_deliveries() {
                let ours = match &delivery {
                    Delivery::Frame(v) | Delivery::Snapshot { view: v, .. } => *v == view,
                    // The app's own messages (the clipboard's) are any test's.
                    Delivery::Message(message) => message_view(message).is_none_or(|v| v == view),
                    Delivery::Gone => panic!("the host went away"),
                };
                if ours {
                    found |= done(&delivery);
                    if matches!(delivery, Delivery::Frame(_)) {
                        ack(&host, view);
                    }
                } else {
                    others.push(delivery);
                }
            }
            // Other tests' deliveries go back for them.
            if !others.is_empty() {
                client::put_back(others);
            }
            if found {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn message_view(message: &FromHost) -> Option<u64> {
        match message {
            FromHost::Title { view, .. }
            | FromHost::Url { view, .. }
            | FromHost::Loading { view, .. }
            | FromHost::LoadFinished { view, .. }
            | FromHost::History { view, .. }
            | FromHost::Cursor { view, .. }
            | FromHost::NewWindow { view, .. }
            | FromHost::Crashed { view, .. }
            | FromHost::Editing { view, .. }
            | FromHost::LinkHovered { view, .. }
            | FromHost::ThemeColor { view, .. }
            | FromHost::Favicon { view, .. }
            | FromHost::Store { view, .. }
            | FromHost::Present { view, .. }
            | FromHost::Snapshot { view, .. } => Some(*view),
            FromHost::Ready | FromHost::ClipboardWrite { .. } | FromHost::ClipboardRead { .. } => {
                None
            }
        }
    }

    fn title_is(title: &str) -> impl FnMut(&Delivery) -> bool + '_ {
        move |delivery| matches!(delivery, Delivery::Message(FromHost::Title { title: t, .. }) if t == title)
    }

    /// The front frame's pixel at `(x, y)`, as BGRA, and its size.
    fn pixel(frame: &Mutex<Frame>, x: u32, y: u32) -> Option<([u8; 4], (u32, u32))> {
        let frame = frame.lock().unwrap();
        let (_, store) = frame.front.as_ref()?;
        let at = (y * store.pitch + x * 4) as usize;
        let bytes: [u8; 4] = store.bytes()[at..at + 4].try_into().ok()?;
        Some((bytes, (store.width, store.height)))
    }

    #[test]
    fn a_page_draws_into_shared_memory_then_idles() {
        let Some((host, _serial)) = host_or_skip("a_page_draws_into_shared_memory_then_idles")
        else {
            return;
        };
        let view = 1_000_001;
        let frame = host.frame(view);
        host.send(
            Message::new(ToHost::CreateView, view)
                .u32(320)
                .u32(200)
                .f64(1.),
        );
        host.send(
            Message::new(ToHost::LoadHtml, view)
                .str("<title>Blue</title><body style='margin:0;background:rgb(0,0,255)'>"),
        );
        let (mut first_damage, mut titled) = (None, false);
        let drawn = wait_for(view, |d| {
            if matches!(d, Delivery::Frame(_)) && first_damage.is_none() {
                first_damage = Some(frame.lock().unwrap().damage);
            }
            titled |=
                matches!(d, Delivery::Message(FromHost::Title { title, .. }) if title == "Blue");
            titled && pixel(&frame, 160, 100).is_some_and(|(bgra, _)| bgra == [255, 0, 0, 255])
        });
        assert!(drawn, "the page's title, and blue drawn by Ladybird");
        assert_eq!(pixel(&frame, 160, 100).unwrap().1, (320, 200));
        assert_eq!(
            first_damage,
            Some(Some((0, 0, 320, 200))),
            "the first frame is all new"
        );
        assert_eq!(
            frame.lock().unwrap().front.as_ref().map(|(_, s)| s.format),
            Some(2),
            "BGRA"
        );
        // A still page sends nothing.
        assert!(
            !wait_for_a_while(view, Duration::from_secs(2)),
            "an idle page delivers nothing"
        );
    }

    /// Whether anything for `view` arrives within `time`.
    fn wait_for_a_while(view: u64, time: Duration) -> bool {
        let deadline = Instant::now() + time;
        while Instant::now() < deadline {
            let deliveries = client::take_deliveries();
            let (ours, others): (Vec<_>, Vec<_>) = deliveries.into_iter().partition(|d| match d {
                Delivery::Frame(v) | Delivery::Snapshot { view: v, .. } => *v == view,
                Delivery::Message(m) => message_view(m) == Some(view),
                Delivery::Gone => true,
            });
            client::put_back(others);
            if !ours.is_empty() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn clicks_and_keys_reach_the_page() {
        let Some((host, _serial)) = host_or_skip("clicks_and_keys_reach_the_page") else {
            return;
        };
        let view = 1_000_002;
        host.send(
            Message::new(ToHost::CreateView, view)
                .u32(320)
                .u32(200)
                .f64(1.),
        );
        host.send(Message::new(ToHost::Focus, view).u8(1));
        host.send(Message::new(ToHost::LoadHtml, view).str(
            "<title>ready</title><body style='margin:0'>\
             <script>document.addEventListener('mousedown', e => { if (e.target.id != 'f') document.title = 'down ' + e.button })</script>\
             <input id=f style='position:absolute;left:0;top:100px;width:300px' oninput='document.title=this.value'>",
        ));
        assert!(wait_for(view, title_is("ready")));
        let mouse = |kind: u8, x: f64, y: f64, button: u8, buttons: u8| {
            host.send(
                Message::new(ToHost::Mouse, view)
                    .u8(kind)
                    .f64(x)
                    .f64(y)
                    .u8(button)
                    .u8(buttons)
                    .u8(0)
                    .u8(1),
            );
        };
        mouse(MOUSE_DOWN, 50., 20., 1, 1);
        mouse(MOUSE_UP, 50., 20., 1, 0);
        assert!(wait_for(view, title_is("down 0")), "a primary press");
        // Into the field, then type.
        mouse(MOUSE_DOWN, 50., 110., 1, 1);
        mouse(MOUSE_UP, 50., 110., 1, 0);
        for c in ['h', 'i'] {
            let (code, character) = keys::code(&zgx::input::Key::Character(c.to_string()));
            host.send(
                Message::new(ToHost::Key, view)
                    .u8(KEY_DOWN)
                    .u32(code)
                    .u8(0)
                    .u32(character)
                    .u8(0),
            );
            host.send(
                Message::new(ToHost::Key, view)
                    .u8(KEY_UP)
                    .u32(code)
                    .u8(0)
                    .u32(0)
                    .u8(0),
            );
        }
        assert!(
            wait_for(view, title_is("hi")),
            "typed text reaches the field"
        );
        host.send(Message::new(ToHost::ImeCommit, view).str("é"));
        assert!(wait_for(view, title_is("hié")), "an input method's text");
    }

    #[test]
    fn the_hovered_link_and_theme_colour_reach_the_app() {
        let Some((host, _serial)) = host_or_skip("the_hovered_link_and_theme_colour_reach_the_app")
        else {
            return;
        };
        let view = 1_000_004;
        host.send(
            Message::new(ToHost::CreateView, view)
                .u32(320)
                .u32(200)
                .f64(1.),
        );
        host.send(Message::new(ToHost::LoadHtml, view).str(
            "<title>t</title><meta name=theme-color content='#336699'>\
             <body style='margin:0'><a href='https://example.com/x' style='display:block;width:100px;height:50px'>x</a>",
        ));
        let (mut themed, mut titled) = (false, false);
        let ready = wait_for(view, |d| {
            titled |= matches!(d, Delivery::Message(FromHost::Title { title, .. }) if title == "t");
            themed |= matches!(
                d,
                Delivery::Message(FromHost::ThemeColor {
                    color: Some((0x33, 0x66, 0x99, 255)),
                    ..
                })
            );
            titled && themed
        });
        assert!(ready, "the page's theme colour arrives");
        let mouse_move = |x: f64, y: f64| {
            host.send(
                Message::new(ToHost::Mouse, view)
                    .u8(MOUSE_MOVE)
                    .f64(x)
                    .f64(y)
                    .u8(0)
                    .u8(0)
                    .u8(0)
                    .u8(0),
            );
        };
        mouse_move(20., 20.);
        assert!(
            wait_for(view, |d| matches!(
                d,
                Delivery::Message(FromHost::LinkHovered { url, .. }) if url == "https://example.com/x"
            )),
            "the link under the pointer"
        );
        mouse_move(200., 150.);
        assert!(
            wait_for(view, |d| matches!(
                d,
                Delivery::Message(FromHost::LinkHovered { url, .. }) if url.is_empty()
            )),
            "off the link, none"
        );
    }

    #[test]
    fn snapshots_take_what_shows_or_the_whole_page() {
        let Some((host, _serial)) = host_or_skip("snapshots_take_what_shows_or_the_whole_page")
        else {
            return;
        };
        let runtime = zgx::reactive::Runtime::new();
        let web = WebView::new(
            &runtime,
            zgx::web_view::WebViewOptions {
                engine: WebEngine::Ladybird,
                ..Default::default()
            },
        );
        let page = backend(&web, host.clone());
        host.send(
            Message::new(ToHost::Resize, page.id)
                .u32(320)
                .u32(200)
                .f64(1.),
        );
        web.load_html(
            "<title>tall</title><body style='margin:0'>\
             <div style='height:300px;background:rgb(255,0,0)'></div>\
             <div style='height:300px;background:rgb(0,0,255)'></div>",
            None,
        );
        let frame = host.frame(page.id);
        assert!(
            wait_for(page.id, |_| {
                pixel(&frame, 160, 100)
                    .is_some_and(|(bgra, size)| bgra == [0, 0, 255, 255] && size == (320, 200))
            }),
            "the page draws at its size"
        );
        let answer = |area| {
            let got = Rc::new(RefCell::new(None));
            let into = got.clone();
            web.snapshot(area, move |result| *into.borrow_mut() = Some(result));
            let deadline = Instant::now() + Duration::from_secs(60);
            while got.borrow().is_none() && Instant::now() < deadline {
                deliver();
                std::thread::sleep(Duration::from_millis(10));
            }
            got.borrow_mut()
                .take()
                .expect("an answer")
                .expect("a snapshot")
        };
        let rgba = |image: &zgx::image::ImageData, x: u32, y: u32| -> [u8; 4] {
            let at = ((y * image.width() + x) * 4) as usize;
            image.pixels()[at..at + 4].try_into().unwrap()
        };
        let visible = answer(zgx::web_view::SnapshotArea::Visible);
        assert_eq!((visible.width(), visible.height()), (320, 200));
        assert_eq!(
            rgba(&visible, 160, 100),
            [255, 0, 0, 255],
            "what shows: the red top"
        );
        let whole = answer(zgx::web_view::SnapshotArea::FullPage);
        assert_eq!(whole.width(), 320);
        assert!(
            whole.height() >= 600,
            "the whole document: {}",
            whole.height()
        );
        assert_eq!(rgba(&whole, 160, 100), [255, 0, 0, 255]);
        assert_eq!(
            rgba(&whole, 160, 500),
            [0, 0, 255, 255],
            "and the blue it scrolls to"
        );
    }

    #[test]
    fn edit_commands_copy_to_and_paste_from_the_apps_clipboard() {
        let Some((host, _serial)) =
            host_or_skip("edit_commands_copy_to_and_paste_from_the_apps_clipboard")
        else {
            return;
        };
        let view = 1_000_005;
        host.send(
            Message::new(ToHost::CreateView, view)
                .u32(320)
                .u32(200)
                .f64(1.),
        );
        host.send(Message::new(ToHost::Focus, view).u8(1));
        host.send(Message::new(ToHost::LoadHtml, view).str(
            "<title>ready</title><body style='margin:0'>\
             <input id=f value='copied text' style='position:absolute;left:0;top:0;width:300px;height:40px' \
             onfocus=\"document.title='focused'\" oninput='document.title=this.value'>",
        ));
        assert!(wait_for(view, title_is("ready")));
        // Into the field: its text is what select all and copy take.
        for kind in [MOUSE_DOWN, MOUSE_UP] {
            host.send(
                Message::new(ToHost::Mouse, view)
                    .u8(kind)
                    .f64(50.)
                    .f64(20.)
                    .u8(1)
                    .u8(u8::from(kind == MOUSE_DOWN))
                    .u8(0)
                    .u8(1),
            );
        }
        // Input goes through the Compositor, commands straight to the page:
        // the field has its focus before select all.
        assert!(
            wait_for(view, title_is("focused")),
            "the click focuses the field"
        );
        host.send(Message::new(ToHost::Edit, view).u8(3));
        host.send(Message::new(ToHost::Edit, view).u8(0));
        assert!(
            wait_for(view, |d| matches!(
                d,
                Delivery::Message(FromHost::ClipboardWrite { text, .. }) if text == "copied text"
            )),
            "copy puts the selection on the app's clipboard"
        );
        // Paste asks the app, whose clipboard text replaces the selection.
        host.send(Message::new(ToHost::Edit, view).u8(2));
        let mut request = None;
        assert!(
            wait_for(view, |d| match d {
                Delivery::Message(FromHost::ClipboardRead { request: r }) => {
                    request = Some(*r);
                    true
                }
                _ => false,
            }),
            "paste reads the app's clipboard"
        );
        host.send(
            Message::new(ToHost::ClipboardData, 0)
                .u64(request.unwrap())
                .str("pasted")
                .str(""),
        );
        assert!(
            wait_for(view, title_is("pasted")),
            "the app's text is pasted into the field"
        );
    }

    #[test]
    fn a_pages_icon_reaches_the_controller() {
        let Some((host, _serial)) = host_or_skip("a_pages_icon_reaches_the_controller") else {
            return;
        };
        let runtime = zgx::reactive::Runtime::new();
        let web = WebView::new(
            &runtime,
            zgx::web_view::WebViewOptions {
                engine: WebEngine::Ladybird,
                favicons: true,
                ..Default::default()
            },
        );
        let _page = backend(&web, host);
        // A red 16 x 16 PNG.
        web.load_html(
            "<title>icon</title><link rel=icon href='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAGUlEQVR42mP4z8DwnxLMMGrAqAGjBgwXAwAwxP4QisZM5QAAAABJRU5ErkJggg=='>",
            None,
        );
        let deadline = Instant::now() + Duration::from_secs(60);
        while web.state().with_untracked(|state| state.favicon.is_none())
            && Instant::now() < deadline
        {
            deliver();
            std::thread::sleep(Duration::from_millis(10));
        }
        let favicon = web
            .state()
            .with_untracked(|state| state.favicon.clone())
            .expect("the page's icon");
        assert_eq!((favicon.width(), favicon.height()), (16, 16));
        assert_eq!(&favicon.pixels()[..4], &[255, 0, 0, 255], "red");
    }

    #[test]
    fn a_resize_makes_frames_of_the_new_size() {
        let Some((host, _serial)) = host_or_skip("a_resize_makes_frames_of_the_new_size") else {
            return;
        };
        let view = 1_000_003;
        let frame = host.frame(view);
        host.send(
            Message::new(ToHost::CreateView, view)
                .u32(320)
                .u32(200)
                .f64(1.),
        );
        host.send(
            Message::new(ToHost::LoadHtml, view)
                .str("<title>r</title><body style='margin:0;background:rgb(0,255,0)'>"),
        );
        assert!(wait_for(view, title_is("r")));
        host.send(Message::new(ToHost::Resize, view).u32(200).u32(150).f64(2.));
        let resized = wait_for(view, |_| {
            pixel(&frame, 10, 10).is_some_and(|(_, size)| size == (200, 150))
        });
        assert!(resized, "a frame of the new size");
        let (bgra, _) = pixel(&frame, 100, 75).unwrap();
        assert_eq!(bgra, [0, 255, 0, 255]);
    }

    /// A window's frame: the page's box in a `Ui`, drawn by the GPU renderer
    /// and read back, with the host's deliveries taken as the window takes
    /// them.
    #[test]
    fn a_ladybird_page_shows_in_its_box() {
        let Some((_host, _serial)) = host_or_skip("a_ladybird_page_shows_in_its_box") else {
            return;
        };
        let Ok(mut gpu) = zgx_gpu::GpuRenderer::new(300, 200) else {
            eprintln!("a_ladybird_page_shows_in_its_box: skipped, no GPU adapter");
            return;
        };
        let mut ui = zgx::widgets::Ui::new(300., 200.);
        let web = WebView::new(
            &ui.runtime,
            zgx::web_view::WebViewOptions {
                engine: WebEngine::Ladybird,
                ..Default::default()
            },
        );
        web.load_html(
            "<title>Shown</title><body style='margin:0;background:rgb(255,128,0)'>",
            None,
        );
        let width = ui.signal(200_f32);
        let w = width.clone();
        ui.mount(zgx::compose::provide(
            page_views(None, Rc::new(RefCell::new(None))),
            div()
                .size(300., 200.)
                .bg(zgx::scene::Color(0, 0, 0, 255))
                .child(
                    zgx::web_view::web_view(&web)
                        .absolute()
                        .ml(50.)
                        .mt(50.)
                        .h(100.)
                        .reactive_style(move || zgx::style::Styles::new().w(w.get())),
                ),
        ));
        let frame = |ui: &mut zgx::widgets::Ui, gpu: &mut zgx_gpu::GpuRenderer| {
            deliver();
            ui.prepare_frame();
            let report = ui.scene.borrow_mut().flush();
            gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
            gpu.readback().unwrap()
        };
        let pixel = |pixels: &[u8], x: usize, y: usize| -> [u8; 4] {
            pixels[(y * 300 + x) * 4..(y * 300 + x) * 4 + 4]
                .try_into()
                .unwrap()
        };
        let orange = |p: [u8; 4]| p[0] > 240 && (110..150).contains(&p[1]) && p[2] < 20;
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut pixels = frame(&mut ui, &mut gpu);
        while !orange(pixel(&pixels, 150, 100)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            pixels = frame(&mut ui, &mut gpu);
        }
        assert!(
            orange(pixel(&pixels, 150, 100)),
            "the page in its box: {:?}",
            pixel(&pixels, 150, 100)
        );
        assert_eq!(pixel(&pixels, 20, 20), [0, 0, 0, 255], "outside the box");
        assert_eq!(
            web.title(),
            "Shown",
            "the page's title reached the controller"
        );
        // The box grows: the page is asked for the new size and fills it.
        width.set(240.);
        let deadline = Instant::now() + Duration::from_secs(60);
        pixels = frame(&mut ui, &mut gpu);
        while !orange(pixel(&pixels, 285, 100)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            pixels = frame(&mut ui, &mut gpu);
        }
        assert!(
            orange(pixel(&pixels, 285, 100)),
            "the page fills its wider box"
        );
    }
}
