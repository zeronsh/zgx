//! Native views on iOS: the system's glass between zgx's planes.
//!
//! A window's views, bottom to top: winit's view with the base plane's Metal
//! layer, a view holding the glass, and a view holding the overlay plane's.
//! Neither added view handles a touch: it reaches winit's view and zgx's own
//! hit testing, directly or, over interactive glass (which must see touches to
//! answer them), up the responder chain. Neither is seen by assistive
//! technology: the one tree of elements is zgx's. All of it changes in one
//! Core Animation transaction, the planes' drawables presented inside it.
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
    rc::Retained,
    runtime::AnyClass,
    runtime::{AnyObject, NSObject, NSObjectProtocol},
    sel,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use std::{collections::HashMap, ptr::NonNull, rc::Rc};
use winit::raw_window_handle::{
    HasWindowHandle, RawDisplayHandle, RawWindowHandle, UiKitDisplayHandle, UiKitWindowHandle,
};
use zgx::{
    glass::{GlassContent, GlassStyle, GlassVariant},
    native_control::{TabsState, ToggleState},
    native_view::{NativeContent, NativePlacement, NativeViews, PlaneSplit},
    scene::{NodeId, Rect, Scene},
    widgets::Ui,
};

/// `UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight`.
const FILL: usize = 2 | 16;

fn rect(r: Rect) -> CGRect {
    CGRect {
        origin: CGPoint {
            x: r.x as f64,
            y: r.y as f64,
        },
        size: CGSize {
            width: r.width as f64,
            height: r.height as f64,
        },
    }
}

/// A view of ours: over the window's, untouchable and unseen by assistive
/// technology.
/// A `CGPathRef`, only passed on.
#[repr(C)]
struct CgPath([u8; 0]);
// SAFETY: as Core Graphics declares it, an opaque struct behind a pointer.
unsafe impl objc2::RefEncode for CgPath {
    const ENCODING_REF: objc2::Encoding =
        objc2::Encoding::Pointer(&objc2::Encoding::Struct("CGPath", &[]));
}

/// Cut `clip` (showing `visible`, in the window's points) to the rounded box
/// around its node, if any: its own corners, or a rounded parent's. The innermost
/// is the one a row's control or a page in a card meets.
fn set_rounded_mask(clip: &AnyObject, round: Option<&zgx::scene::RoundRect>, visible: Rect) {
    // SAFETY: UIKit's and Core Animation's constructors and setters, on the main
    // thread; the layer keeps its mask.
    unsafe {
        let layer: Retained<AnyObject> = msg_send![clip, layer];
        let Some(round) = round else {
            let none: *const AnyObject = std::ptr::null();
            let _: () = msg_send![&*layer, setMask: none];
            return;
        };
        let radius = round.radii.iter().copied().fold(0f32, f32::max);
        let inside = Rect::new(
            round.rect.x - visible.x,
            round.rect.y - visible.y,
            round.rect.width,
            round.rect.height,
        );
        let path: Retained<AnyObject> = msg_send![class!(UIBezierPath),
            bezierPathWithRoundedRect: rect(inside),
            cornerRadius: radius as f64];
        let shape: Retained<AnyObject> = msg_send![class!(CAShapeLayer), new];
        let cg: *const CgPath = msg_send![&*path, CGPath];
        let _: () = msg_send![&*shape, setPath: cg];
        let _: () = msg_send![&*layer, setMask: &*shape];
    }
}

fn plain_view(frame: CGRect) -> Retained<AnyObject> {
    // SAFETY: UIView's initializer, on the main thread.
    unsafe { msg_send![msg_send![class!(UIView), alloc], initWithFrame: frame] }
}

/// A view touches pass through, unless on a view inside it: a host covers the
/// window, and would take every touch from those under it.
fn host_class() -> &'static AnyClass {
    static CLASS: std::sync::OnceLock<&'static AnyClass> = std::sync::OnceLock::new();
    extern "C-unwind" fn hit_test(
        this: &AnyObject,
        _: objc2::runtime::Sel,
        at: CGPoint,
        event: *mut AnyObject,
    ) -> *mut AnyObject {
        // SAFETY: UIView's own method, UIKit asking on the main thread.
        let hit: *mut AnyObject =
            unsafe { msg_send![super(this, class!(UIView)), hitTest: at, withEvent: event] };
        if std::ptr::eq(hit.cast_const(), this) {
            std::ptr::null_mut()
        } else {
            hit
        }
    }
    CLASS.get_or_init(|| {
        let mut builder =
            objc2::runtime::ClassBuilder::new(c"ZgxHostView", class!(UIView)).expect("one class");
        // SAFETY: the function has the selector's signature.
        unsafe {
            builder.add_method(
                objc2::sel!(hitTest:withEvent:),
                hit_test as extern "C-unwind" fn(_, _, _, _) -> _,
            );
        }
        builder.register()
    })
}

fn passive_view(frame: CGRect) -> Retained<AnyObject> {
    // SAFETY: UIView's initializer and setters, on the main thread (every
    // caller holds `Surfaces`, which is made there).
    unsafe {
        let view: Retained<AnyObject> =
            msg_send![msg_send![host_class(), alloc], initWithFrame: frame];
        passive(&view);
        view
    }
}

/// # Safety
/// `view` is a UIView, on the main thread.
unsafe fn passive(view: &AnyObject) {
    // SAFETY: UIView's setters, as the caller promises.
    unsafe {
        let _: () = msg_send![view, setUserInteractionEnabled: false];
        let _: () = msg_send![view, setAccessibilityElementsHidden: true];
        let _: () = msg_send![view, setIsAccessibilityElement: false];
    }
}

/// Opens a transaction its drop commits; implicit animations off, as zgx's
/// frames are the animation.
pub(crate) struct Transaction;
impl Transaction {
    pub fn begin() -> Self {
        // SAFETY: CATransaction's class methods, on the main thread.
        unsafe {
            let _: () = msg_send![class!(CATransaction), begin];
            let _: () = msg_send![class!(CATransaction), setDisableActions: true];
        }
        Self
    }
}
impl Drop for Transaction {
    fn drop(&mut self) {
        // SAFETY: closes `begin`'s transaction.
        unsafe {
            let _: () = msg_send![class!(CATransaction), commit];
        }
    }
}

/// What a mounted system view is.
#[derive(Clone)]
enum Content {
    Glass(GlassContent),
    /// An app's own view (`NativeContent::Platform`): it takes its touches.
    Platform,
    /// A `WKWebView` (`NativeContent::Web`), kept while it stands here.
    Web(#[allow(dead_code)] Rc<dyn zgx::web_view::WebViewBackend>),
    /// What says where the scroll edge effect reaches
    /// (`NativeContent::EdgeEffect`): a view above the content, as a bar is.
    Edge(zgx::native_control::EdgeEffect),
    /// A `UISwitch` (`NativeContent::Toggle`).
    Toggle(Rc<ToggleState>, #[allow(dead_code)] Retained<ToggleTarget>),
    /// A `UITabBar` (`NativeContent::Tabs`), and what it tells through.
    Tabs(Rc<TabsState>, #[allow(dead_code)] Retained<TabsDelegate>),
}

impl Content {
    /// A control a person touches, cut by what clips its node.
    fn is_control(&self) -> bool {
        matches!(
            self,
            Self::Platform | Self::Web(..) | Self::Toggle(..) | Self::Tabs(..)
        )
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<TabsState>]
    struct TabsDelegate;
    unsafe impl NSObjectProtocol for TabsDelegate {}
    impl TabsDelegate {
        #[unsafe(method(tabBar:didSelectItem:))]
        fn did_select(&self, _bar: &AnyObject, item: &AnyObject) {
            // SAFETY: UIBarItem's getter; the tag is the tab's index.
            let tag: isize = unsafe { msg_send![item, tag] };
            (self.ivars().changed)(tag as usize);
        }
    }
);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = std::cell::RefCell<Rc<ToggleState>>]
    struct ToggleTarget;
    unsafe impl NSObjectProtocol for ToggleTarget {}
    impl ToggleTarget {
        #[unsafe(method(toggled:))]
        fn toggled(&self, control: &AnyObject) {
            // SAFETY: UISwitch's getter.
            let on: bool = unsafe { msg_send![control, isOn] };
            (self.ivars().borrow().changed)(on);
        }
    }
);

struct Mounted {
    view: Retained<AnyObject>,
    /// What clips a control, holding it (`Content::is_control`).
    clip: Option<Retained<AnyObject>>,
    /// The `UISwitch` shows this.
    shown_on: bool,
    content: Content,
    style: Option<GlassStyle>,
    parent: Option<NodeId>,
    /// The host it is in (`NativePlacement::slot`).
    slot: u8,
    /// Whether its clip cuts it, and what it leaves.
    cut: Option<(bool, Rect)>,
    frame: Option<Rect>,
    alpha: f32,
    hidden: bool,
}

/// `scroll`'s effect on `edge`.
///
/// # Safety
/// `scroll` is a `UIScrollView`, on the main thread.
unsafe fn scroll_edge_effect(
    scroll: &AnyObject,
    edge: zgx::native_control::Edge,
) -> Retained<AnyObject> {
    use zgx::native_control::Edge;
    // SAFETY: UIScrollView's getters, as the caller promises.
    unsafe {
        match edge {
            Edge::Top => msg_send![scroll, topEdgeEffect],
            Edge::Bottom => msg_send![scroll, bottomEdgeEffect],
            Edge::Left => msg_send![scroll, leftEdgeEffect],
            Edge::Right => msg_send![scroll, rightEdgeEffect],
        }
    }
}

/// The app, held still inside a scroll view that doesn't scroll: the system's
/// edge effect works on a scroll view's own contents (its blur on any behind it,
/// its veil only on those), so this is how it gets over zgx's Metal. Made only
/// when an app asks for the effect: public API all, and nothing for those that
/// don't.
struct EdgeHost {
    scroll: Retained<AnyObject>,
    above: Retained<AnyObject>,
    /// Which slots lie under the effect.
    under: u8,
}
/// How far the held scroll view is scrolled: past either end, so every edge's
/// effect shows.
const EDGE_OFFSET: f64 = 400.;
impl EdgeHost {
    fn new(root: &Retained<AnyObject>) -> Self {
        // SAFETY: UIKit's constructors and setters, on the main thread. The view
        // controller's view gives way to a plain one holding the scroll view and
        // the view above it.
        unsafe {
            let window: Retained<AnyObject> = msg_send![&**root, window];
            let controller: Retained<AnyObject> = msg_send![&*window, rootViewController];
            let bounds: CGRect = msg_send![&**root, bounds];
            let holder = plain_view(bounds);
            let _: () = msg_send![&*holder, setAutoresizingMask: FILL];
            // What the effect's backdrop lies over, as a view controller's view has.
            let background: Retained<AnyObject> = msg_send![class!(UIColor), systemBackgroundColor];
            let _: () = msg_send![&*holder, setBackgroundColor: &*background];
            let _: () = msg_send![&*controller, setView: &*holder];
            let scroll: Retained<AnyObject> =
                msg_send![msg_send![class!(UIScrollView), alloc], initWithFrame: bounds];
            let _: () = msg_send![&*scroll, setAutoresizingMask: FILL];
            let _: () = msg_send![&*scroll, setScrollEnabled: false];
            let _: () = msg_send![&*scroll, setScrollsToTop: false];
            let _: () = msg_send![&*scroll, setShowsVerticalScrollIndicator: false];
            let _: () = msg_send![&*scroll, setShowsHorizontalScrollIndicator: false];
            // UIScrollViewContentInsetAdjustmentNever.
            let _: () = msg_send![&*scroll, setContentInsetAdjustmentBehavior: 2isize];
            let size = CGSize {
                width: 20000.,
                height: 20000.,
            };
            let _: () = msg_send![&*scroll, setContentSize: size];
            let at = CGPoint {
                x: EDGE_OFFSET,
                y: EDGE_OFFSET,
            };
            let _: () = msg_send![&*scroll, setContentOffset: at];
            use zgx::native_control::Edge;
            for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                let _: () = msg_send![&*scroll_edge_effect(&scroll, edge), setHidden: true];
            }
            let _: () = msg_send![&*holder, addSubview: &*scroll];
            // A view between: winit sets the app's view's frame itself.
            let held = plain_view(CGRect {
                origin: at,
                size: bounds.size,
            });
            let _: () = msg_send![&*held, setAutoresizingMask: FILL];
            let _: () = msg_send![&*scroll, addSubview: &*held];
            let _: () = msg_send![&*held, addSubview: &**root];
            let above = passive_view(bounds);
            let _: () = msg_send![&*above, setAutoresizingMask: FILL];
            let _: () = msg_send![&*above, setUserInteractionEnabled: true];
            let _: () = msg_send![&*holder, addSubview: &*above];
            Self {
                scroll,
                above,
                under: LEVELS,
            }
        }
    }
    /// A view that says where `effect` reaches: it must hold a view to count.
    fn element(&self, effect: zgx::native_control::EdgeEffect) -> Retained<AnyObject> {
        use zgx::native_control::Edge;
        let rect_edge = match effect.edge {
            Edge::Top => 1usize,
            Edge::Left => 2,
            Edge::Bottom => 4,
            Edge::Right => 8,
        };
        // SAFETY: UIKit's constructors and setters, on the main thread.
        unsafe {
            let scroll_effect = scroll_edge_effect(&self.scroll, effect.edge);
            let _: () = msg_send![&*scroll_effect, setHidden: false];
            let style: Retained<AnyObject> = if effect.hard {
                msg_send![class!(UIScrollEdgeEffectStyle), hardStyle]
            } else {
                msg_send![class!(UIScrollEdgeEffectStyle), softStyle]
            };
            let _: () = msg_send![&*scroll_effect, setStyle: &*style];
            let view = plain_view(rect(Rect::new(0., 0., 1., 1.)));
            let _: () = msg_send![&*view, setUserInteractionEnabled: false];
            let _: () = msg_send![&*view, setAccessibilityElementsHidden: true];
            let interaction: Retained<AnyObject> =
                msg_send![class!(UIScrollEdgeElementContainerInteraction), new];
            let _: () = msg_send![&*interaction, setScrollView: &*self.scroll];
            let _: () = msg_send![&*interaction, setEdge: rect_edge];
            let _: () = msg_send![&*view, addInteraction: &*interaction];
            let label: Retained<AnyObject> = msg_send![class!(UILabel), new];
            let text = crate::ios_system::ns_string(" ");
            let _: () = msg_send![&*label, setText: &*text];
            let clear: Retained<AnyObject> = msg_send![class!(UIColor), clearColor];
            let _: () = msg_send![&*label, setTextColor: &*clear];
            let _: () = msg_send![&*label, setAutoresizingMask: FILL];
            let _: () = msg_send![&*view, addSubview: &*label];
            view
        }
    }
}

/// How many overlays stack between native views: a page, what is pushed over
/// it, a sheet or a tab bar over those. Paint above them all joins the last.
pub(crate) const LEVELS: u8 = 4;

pub(crate) struct Surfaces {
    /// Each pane in a group, with its group (`read_groups`).
    groups: HashMap<NodeId, NodeId>,
    /// Native views by slot, each under the overlay of the next level:
    /// `hosts[0]`, `planes[0]`, `hosts[1]`, `planes[1]`…
    hosts: Vec<Retained<AnyObject>>,
    /// The overlays' views, by level from 1, and where each shows.
    planes: Vec<(Retained<AnyObject>, Option<Option<Rect>>)>,
    /// The window's view.
    root: Retained<AnyObject>,
    /// What holds the app in a scroll view of the system's, once one is asked for.
    edge_host: Option<EdgeHost>,
    mounted: HashMap<NodeId, Mounted>,
    order: Vec<NodeId>,
    zoom: f32,
    interactive: bool,
    /// `UISwitch`es no row shows now, to show the next row that needs one.
    reusable: Vec<(
        Retained<AnyObject>,
        Retained<AnyObject>,
        Retained<ToggleTarget>,
    )>,
}

/// More than a phone's screen of rows.
const REUSABLE_TOGGLES: usize = 32;

impl Surfaces {
    pub fn new(window: &winit::window::Window) -> Result<Self, String> {
        MainThreadMarker::new().ok_or("native views need the main thread")?;
        let handle = window.window_handle().map_err(|e| e.to_string())?;
        let RawWindowHandle::UiKit(handle) = handle.as_raw() else {
            return Err("the window has no UIView".into());
        };
        // SAFETY: winit's view, alive with its window, which outlives this.
        let root: Retained<AnyObject> =
            unsafe { Retained::retain(handle.ui_view.as_ptr().cast()) }.ok_or("no view")?;
        // SAFETY: UIView's getters and setters, on the main thread checked above.
        let (hosts, planes) = unsafe {
            let bounds: CGRect = msg_send![&*root, bounds];
            let (mut hosts, mut planes) = (Vec::new(), Vec::new());
            for _ in 0..LEVELS {
                let host = passive_view(bounds);
                let _: () = msg_send![&*host, setAutoresizingMask: FILL];
                let _: () = msg_send![&*root, addSubview: &*host];
                hosts.push(host);
                let plane = passive_view(bounds);
                let _: () = msg_send![&*plane, setHidden: true];
                let _: () = msg_send![&*root, addSubview: &*plane];
                planes.push((plane, None));
            }
            (hosts, planes)
        };
        Ok(Self {
            groups: HashMap::new(),
            hosts,
            planes,
            root,
            edge_host: None,
            mounted: HashMap::new(),
            order: Vec::new(),
            zoom: 1.,
            interactive: false,
            reusable: Vec::new(),
        })
    }
    /// A renderer presenting into the overlay's view.
    pub fn overlay_renderer(
        &self,
        context: &zgx_gpu::GpuContext,
        scale: f32,
    ) -> Result<zgx_gpu::GpuRenderer, zgx_gpu::GpuError> {
        self.level_renderer(1, context, scale)
    }
    /// A renderer presenting into the view of the overlay at `level`.
    pub fn level_renderer(
        &self,
        level: u8,
        context: &zgx_gpu::GpuContext,
        scale: f32,
    ) -> Result<zgx_gpu::GpuRenderer, zgx_gpu::GpuError> {
        let plane = &self.planes[usize::from(level) - 1].0;
        // SAFETY: a UIView's setter, on the main thread.
        unsafe {
            let _: () = msg_send![&**plane, setContentScaleFactor: scale as f64];
        }
        let view = NonNull::from(&**plane).cast();
        // SAFETY: the view lives as long as these surfaces, and they drop
        // the renderer first (`NativeHost::suspend`, field order there).
        unsafe {
            zgx_gpu::GpuRenderer::for_raw_surface(
                RawDisplayHandle::UiKit(UiKitDisplayHandle::new()),
                RawWindowHandle::UiKit(UiKitWindowHandle::new(view)),
                1,
                1,
                scale,
                context,
            )
        }
    }
    /// The view saying where the effect reaches, and the effect on, in the
    /// scroll view the app now lies in (made for the first one).
    fn make_edge(&mut self, effect: zgx::native_control::EdgeEffect) -> Retained<AnyObject> {
        let edge_host = self
            .edge_host
            .get_or_insert_with(|| EdgeHost::new(&self.root));
        edge_host.element(effect)
    }
    /// Hosts and planes in the app's view lie under the effect; those above
    /// its slot in the view above the scroll view.
    fn place_planes(&mut self, edge_slot: Option<u8>) {
        let Some(edge_host) = &mut self.edge_host else {
            return;
        };
        let under = edge_slot.unwrap_or(LEVELS);
        if edge_host.under == under {
            return;
        }
        edge_host.under = under;
        // SAFETY: UIView's method, on the main thread. A view keeps its layer's
        // contents as it moves.
        unsafe {
            for slot in 0..LEVELS {
                let (host, plane) = (
                    &self.hosts[usize::from(slot)],
                    &self.planes[usize::from(slot)].0,
                );
                let inside = slot <= under;
                let parent = if inside { &self.root } else { &edge_host.above };
                let _: () = msg_send![&**parent, addSubview: &**host];
                // The plane over this slot's views: inside only below the effect.
                let parent = if slot < under {
                    &self.root
                } else {
                    &edge_host.above
                };
                let _: () = msg_send![&**parent, addSubview: &**plane];
            }
        }
    }
    /// Whether native geometry or a pane's look changes this frame.
    pub fn differs(&self, split: &PlaneSplit, previous: &PlaneSplit) -> bool {
        split.natives() != previous.natives()
            || self.mounted.values().any(|mounted| match &mounted.content {
                Content::Glass(GlassContent::Pane(style)) => mounted.style != Some(*style.borrow()),
                _ => false,
            })
    }
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom;
    }
    /// Show the overlay's view at `rect`, or hide it.
    pub fn show_overlay(&mut self, shown: Option<Rect>) {
        self.show_level(1, shown);
    }
    /// Show the view of the overlay at `level` at `rect`, or hide it.
    pub fn show_level(&mut self, level: u8, shown: Option<Rect>) {
        let (view, was) = &mut self.planes[usize::from(level) - 1];
        if *was == Some(shown) {
            return;
        }
        *was = Some(shown);
        // SAFETY: UIView's setters, on the main thread.
        unsafe {
            let _: () = msg_send![&**view, setHidden: shown.is_none()];
            if let Some(shown) = shown {
                let _: () = msg_send![&**view, setFrame: rect(shown)];
            }
        }
    }
    pub fn apply(&mut self, natives: &NativeViews, split: &PlaneSplit) {
        let views = natives.views();
        // Gone: their views too.
        let reusable = &mut self.reusable;
        self.mounted.retain(|node, mounted| {
            let kept = views.iter().any(|(other, _)| other == node);
            if !kept {
                // SAFETY: UIView's method, on the main thread.
                unsafe {
                    let top = mounted.clip.as_ref().unwrap_or(&mounted.view);
                    let _: () = msg_send![&**top, removeFromSuperview];
                }
                // Scrolling away and back makes and drops a row's control each
                // time: kept, as a table view keeps its cells.
                if let (Content::Toggle(_, target), Some(clip)) = (&mounted.content, &mounted.clip)
                    && reusable.len() < REUSABLE_TOGGLES
                {
                    reusable.push((mounted.view.clone(), clip.clone(), target.clone()));
                }
            }
            kept
        });
        for (node, content) in &views {
            let (view, content) = match content {
                NativeContent::Glass(content) if !self.mounted.contains_key(node) => {
                    (make_view(content), Content::Glass(content.clone()))
                }
                NativeContent::Platform(view) if !self.mounted.contains_key(node) => {
                    let Some(view) = view.downcast_ref::<Retained<AnyObject>>() else {
                        continue;
                    };
                    (view.clone(), Content::Platform)
                }
                NativeContent::Web(web) if !web.is_closed() && !self.mounted.contains_key(node) => {
                    let Some(backend) = super::ios_web::backend(web) else {
                        continue;
                    };
                    let Some(view) = backend
                        .as_any()
                        .downcast_ref::<super::ios_web::IosWebView>()
                        .map(super::ios_web::IosWebView::view)
                    else {
                        continue;
                    };
                    (view, Content::Web(backend))
                }
                NativeContent::EdgeEffect(effect) if !self.mounted.contains_key(node) => {
                    (self.make_edge(*effect), Content::Edge(*effect))
                }
                NativeContent::Toggle(state) if !self.mounted.contains_key(node) => {
                    if let Some((view, clip, target)) = self.reusable.pop() {
                        *target.ivars().borrow_mut() = state.clone();
                        // SAFETY: UISwitch's and UIView's setters, on the main thread.
                        unsafe {
                            let _: () = msg_send![&*view, setOn: state.on.get(), animated: false];
                            let _: () = msg_send![&*clip, setHidden: false];
                        }
                        self.mounted.insert(
                            *node,
                            Mounted {
                                view,
                                clip: Some(clip),
                                shown_on: state.on.get(),
                                content: Content::Toggle(state.clone(), target),
                                style: None,
                                parent: None,
                                slot: 0,
                                cut: None,
                                frame: None,
                                alpha: 1.,
                                hidden: false,
                            },
                        );
                        continue;
                    }
                    let (view, target) = make_toggle(state);
                    (view, Content::Toggle(state.clone(), target))
                }
                NativeContent::Tabs(state) if !self.mounted.contains_key(node) => {
                    let (view, delegate) = make_tabs(state);
                    (view, Content::Tabs(state.clone(), delegate))
                }
                _ => continue,
            };
            let clip = content.is_control().then(|| {
                let clip = plain_view(rect(Rect::new(0., 0., 1., 1.)));
                // SAFETY: UIView's method, on the main thread.
                unsafe {
                    let _: () = msg_send![&*clip, addSubview: &*view];
                }
                clip
            });
            self.mounted.entry(*node).or_insert_with(|| Mounted {
                view,
                clip,
                shown_on: matches!(&content, Content::Toggle(state, _) if state.on.get()),
                content,
                style: None,
                parent: None,
                slot: 0,
                cut: None,
                frame: None,
                alpha: 1.,
                hidden: false,
            });
        }
        // What must lie under the effect and what above it, by its slot.
        let edge_slot = split
            .natives()
            .iter()
            .filter(|placement| {
                self.mounted
                    .get(&placement.node)
                    .is_some_and(|mounted| matches!(mounted.content, Content::Edge(_)))
            })
            .map(|placement| placement.slot)
            .max();
        self.place_planes(edge_slot);
        let mut order = Vec::with_capacity(split.natives().len());
        for placement in split.natives() {
            order.push(placement.node);
            let parent = self.groups.get(&placement.node).copied();
            self.place(placement, parent);
        }
        // A node scrolled quite away has no placement: its view can't stay
        // where it last was.
        for (node, mounted) in &mut self.mounted {
            if !mounted.hidden && !order.contains(node) {
                mounted.hidden = true;
                let top = mounted.clip.as_ref().unwrap_or(&mounted.view);
                // SAFETY: UIView's setter, on the main thread.
                unsafe {
                    let _: () = msg_send![&**top, setHidden: true];
                }
            }
        }
        // The host passes touches to its panes only while one answers them.
        let interactive = self.mounted.values().any(|mounted| {
            mounted.style.is_some_and(|style| style.interactive)
                || matches!(mounted.content, Content::Glass(GlassContent::Group(_)))
                || mounted.content.is_control()
        });
        if interactive != self.interactive {
            self.interactive = interactive;
            // SAFETY: UIView's setter, on the main thread.
            unsafe {
                for host in &self.hosts {
                    let _: () = msg_send![&**host, setUserInteractionEnabled: interactive];
                }
            }
        }
        // Later paint is higher: reorder only when the order changes.
        if order != self.order {
            for node in &order {
                if let Some(mounted) = self.mounted.get(node) {
                    let host = match mounted.parent {
                        Some(group) => self.mounted.get(&group).map(|g| content_view(&g.view)),
                        None => Some(self.hosts[usize::from(mounted.slot)].clone()),
                    };
                    if let Some(host) = host {
                        // SAFETY: UIView's method, on the main thread.
                        unsafe {
                            let top = mounted.clip.as_ref().unwrap_or(&mounted.view);
                            let _: () = msg_send![&*host, bringSubviewToFront: &**top];
                        }
                    }
                }
            }
            self.order = order;
        }
    }
    /// Which group each pane is in: the nearest around it. Read here, while
    /// the host holds the scene (it is borrowed through `apply`).
    pub fn read_groups(&mut self, scene: &Scene, natives: &NativeViews) {
        self.groups.clear();
        let views = natives.views();
        // The box a group's node fills, whose descendants merge.
        let boxes: Vec<(NodeId, NodeId)> = views
            .iter()
            .filter(|(_, content)| matches!(content, NativeContent::Glass(GlassContent::Group(_))))
            .filter_map(|(node, _)| Some((*node, scene.parent(*node)?)))
            .collect();
        if boxes.is_empty() {
            return;
        }
        for (node, _) in &views {
            if boxes.iter().any(|(group, _)| group == node) {
                continue;
            }
            let mut at = scene.parent(*node);
            while let Some(id) = at {
                if let Some((group, _)) = boxes.iter().find(|(_, boxed)| *boxed == id) {
                    self.groups.insert(*node, *group);
                    break;
                }
                at = scene.parent(id);
            }
        }
    }
    fn place(&mut self, placement: &NativePlacement, parent: Option<NodeId>) {
        let origin = parent
            .and_then(|group| self.mounted.get(&group))
            .and_then(|group| group.frame)
            .map_or((0., 0.), |frame| (frame.x, frame.y));
        let host = match parent {
            Some(group) => self.mounted.get(&group).map(|g| content_view(&g.view)),
            None => Some(self.hosts[usize::from(placement.slot)].clone()),
        };
        let edge_host = self
            .edge_host
            .as_ref()
            .map(|edge_host| edge_host.above.clone());
        let Some(mounted) = self.mounted.get_mut(&placement.node) else {
            return;
        };
        let host = if matches!(mounted.content, Content::Edge(_)) {
            edge_host
        } else {
            host
        };
        if mounted.parent != parent || mounted.slot != placement.slot || mounted.frame.is_none() {
            mounted.parent = parent;
            mounted.slot = placement.slot;
            if let Some(host) = host {
                let top = mounted.clip.as_ref().unwrap_or(&mounted.view);
                // SAFETY: UIView's method, on the main thread.
                unsafe {
                    let _: () = msg_send![&*host, addSubview: &**top];
                }
            }
        }
        let visible = placement.visible();
        let hidden = visible.is_none();
        if hidden != mounted.hidden {
            mounted.hidden = hidden;
            let top = mounted.clip.as_ref().unwrap_or(&mounted.view);
            // SAFETY: UIView's setter, on the main thread.
            unsafe {
                let _: () = msg_send![&**top, setHidden: hidden];
            }
        }
        if let (Some(clip), Some(visible)) = (&mounted.clip, visible) {
            // What cuts it, not its own box: its shadow shows past that.
            let visible = placement
                .clip
                .and_then(|cut| cut.intersection(visible.union(placement.frame)))
                .unwrap_or(visible);
            // A control shows what clips leave of it, as an image would.
            // Cut only where something cuts it: pressed glass grows past its box.
            let round = placement.rounded.first();
            let cut = placement.clip.is_some() || round.is_some();
            let moved_cut = mounted.cut != Some((cut, visible));
            if moved_cut {
                mounted.cut = Some((cut, visible));
                // SAFETY: UIView's setter, on the main thread.
                unsafe {
                    let _: () = msg_send![&**clip, setClipsToBounds: cut];
                }
                set_rounded_mask(clip, round, visible);
            }
            if mounted.frame != Some(placement.frame) || moved_cut {
                mounted.frame = Some(placement.frame);
                let inside = Rect::new(
                    placement.frame.x - visible.x,
                    placement.frame.y - visible.y,
                    placement.frame.width,
                    placement.frame.height,
                );
                // SAFETY: UIView's setters, on the main thread.
                unsafe {
                    let _: () = msg_send![&**clip, setFrame: rect(visible)];
                    let _: () = msg_send![&*mounted.view, setFrame: rect(inside)];
                }
            }
        // Glass is whole or hidden: the system's ignores masks and clips (tried:
        // layer masks and mask views), and a wrapper dims its box.
        } else if mounted.clip.is_none() && mounted.frame != Some(placement.frame) {
            mounted.frame = Some(placement.frame);
            let frame = Rect::new(
                placement.frame.x - origin.0,
                placement.frame.y - origin.1,
                placement.frame.width,
                placement.frame.height,
            );
            // SAFETY: UIView's setter, on the main thread.
            unsafe {
                let _: () = msg_send![&*mounted.view, setFrame: rect(frame)];
            }
        }
        if let Content::Tabs(state, _) = &mounted.content {
            let selected = state.selected.get();
            // SAFETY: UITabBar's getters and setter, on the main thread.
            unsafe {
                let items: Option<Retained<AnyObject>> = msg_send![&*mounted.view, items];
                let chosen: Option<Retained<AnyObject>> = msg_send![&*mounted.view, selectedItem];
                let tag = chosen.as_deref().map_or(-1, |item| {
                    let tag: isize = msg_send![item, tag];
                    tag
                });
                if tag != selected as isize
                    && let Some(items) = items
                {
                    let item: Retained<AnyObject> = msg_send![&*items, objectAtIndex: selected];
                    let _: () = msg_send![&*mounted.view, setSelectedItem: &*item];
                }
            }
        }
        if let Content::Toggle(state, _) = &mounted.content
            && mounted.shown_on != state.on.get()
        {
            mounted.shown_on = state.on.get();
            // SAFETY: UISwitch's method, on the main thread.
            unsafe {
                let _: () = msg_send![&*mounted.view, setOn: mounted.shown_on, animated: true];
            }
        }
        if mounted.alpha != placement.opacity {
            mounted.alpha = placement.opacity;
            // SAFETY: UIView's setter, on the main thread.
            unsafe {
                let _: () = msg_send![&*mounted.view, setAlpha: placement.opacity as f64];
            }
        }
        if let Content::Glass(GlassContent::Pane(style)) = &mounted.content {
            let style = *style.borrow();
            if mounted.style != Some(style) {
                let first = mounted.style.is_none();
                mounted.style = Some(style);
                // Made with this style (`make_view`).
                if !first {
                    restyle(&mounted.view, style);
                }
                // SAFETY: UIView's setter, on the main thread.
                unsafe {
                    let _: () =
                        msg_send![&*mounted.view, setUserInteractionEnabled: style.interactive];
                }
            }
        }
    }
    /// Keyboard focus never goes to glass.
    pub fn sync_focus(&mut self, _: &Ui, _: &NativeViews) {}
}

/// A `UITabBar` of `state`'s tabs, and what it tells through.
fn make_tabs(state: &Rc<TabsState>) -> (Retained<AnyObject>, Retained<TabsDelegate>) {
    let mtm = MainThreadMarker::new().expect("views are made on the main thread");
    let delegate = TabsDelegate::alloc(mtm).set_ivars(state.clone());
    // SAFETY: NSObject's `init`, on a fresh allocation with its ivars set;
    // UIKit's constructors and setters, on the main thread.
    unsafe {
        let delegate: Retained<TabsDelegate> = msg_send![super(delegate), init];
        let view: Retained<AnyObject> = msg_send![class!(UITabBar), new];
        let items: Vec<Retained<AnyObject>> = state
            .items
            .iter()
            .enumerate()
            .map(|(index, (title, symbol))| {
                let image: Option<Retained<AnyObject>> =
                    msg_send![class!(UIImage), systemImageNamed: &*ns_string(symbol)];
                let image_ptr = image
                    .as_deref()
                    .map_or(std::ptr::null(), |image| image as *const AnyObject);
                msg_send![msg_send![class!(UITabBarItem), alloc],
                    initWithTitle: &*ns_string(title),
                    image: image_ptr,
                    tag: index as isize]
            })
            .collect();
        let pointers: Vec<*const AnyObject> = items.iter().map(Retained::as_ptr).collect();
        let list: Retained<AnyObject> =
            msg_send![class!(NSArray), arrayWithObjects: pointers.as_ptr(), count: pointers.len()];
        let _: () = msg_send![&*view, setItems: &*list, animated: false];
        if let Some(item) = items.get(state.selected.get()) {
            let _: () = msg_send![&*view, setSelectedItem: &**item];
        }
        let _: () = msg_send![&*view, setDelegate: &*delegate];
        (view, delegate)
    }
}

fn ns_string(text: &str) -> Retained<AnyObject> {
    let text = std::ffi::CString::new(text.replace('\0', "")).unwrap_or_default();
    // SAFETY: NSString's constructor, over a NUL-terminated string.
    unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
}

/// A `UISwitch` that tells `state`, and what it tells through.
fn make_toggle(state: &Rc<ToggleState>) -> (Retained<AnyObject>, Retained<ToggleTarget>) {
    let mtm = MainThreadMarker::new().expect("views are made on the main thread");
    let target = ToggleTarget::alloc(mtm).set_ivars(std::cell::RefCell::new(state.clone()));
    // SAFETY: NSObject's `init`, on a fresh allocation with its ivars set;
    // UIKit's constructor and method. UIControlEventValueChanged is 1 << 12.
    unsafe {
        let target: Retained<ToggleTarget> = msg_send![super(target), init];
        let view: Retained<AnyObject> = msg_send![class!(UISwitch), new];
        let _: () = msg_send![&*view, setOn: state.on.get(), animated: false];
        let _: () = msg_send![&*view,
            addTarget: &*target,
            action: sel!(toggled:),
            forControlEvents: 1usize << 12];
        (view, target)
    }
}

/// A group's or pane's system view, made with its effect as it is to be:
/// UIKit gives an effect's press response to a view made with it, and not to
/// one that is handed it before its first layout.
fn make_view(content: &GlassContent) -> Retained<AnyObject> {
    // SAFETY: UIKit's constructors and setters, on the main thread;
    // UIGlassContainerEffect and UIGlassEffect are iOS 26's.
    unsafe {
        let effect: Retained<AnyObject> = match content {
            GlassContent::Group(spacing) => {
                let effect: Retained<AnyObject> = msg_send![class!(UIGlassContainerEffect), new];
                let _: () = msg_send![&*effect, setSpacing: *spacing as f64];
                effect
            }
            GlassContent::Pane(style) => glass_effect(*style.borrow()),
        };
        let view: Retained<AnyObject> =
            msg_send![msg_send![class!(UIVisualEffectView), alloc], initWithEffect: &*effect];
        passive(&view);
        if let GlassContent::Pane(style) = content {
            set_corners(&view, *style.borrow());
        }
        view
    }
}

/// The system's glass for `style`.
///
/// # Safety
/// On the main thread.
unsafe fn glass_effect(style: GlassStyle) -> Retained<AnyObject> {
    let variant: isize = match style.variant {
        GlassVariant::Regular => 0,
        GlassVariant::Clear => 1,
    };
    // SAFETY: UIKit's constructors and setters, as the caller promises.
    unsafe {
        let effect: Retained<AnyObject> =
            msg_send![class!(UIGlassEffect), effectWithStyle: variant];
        let _: () = msg_send![&*effect, setInteractive: style.interactive];
        if let Some(tint) = style.tint {
            let channel = |v: u8| v as f64 / 255.;
            let color: Retained<AnyObject> = msg_send![class!(UIColor),
                colorWithRed: channel(tint.0),
                green: channel(tint.1),
                blue: channel(tint.2),
                alpha: channel(tint.3)];
            let _: () = msg_send![&*effect, setTintColor: &*color];
        }
        effect
    }
}

/// # Safety
/// `view` is a UIView, on the main thread.
unsafe fn set_corners(view: &AnyObject, style: GlassStyle) {
    // SAFETY: UIKit's constructors and setter, as the caller promises.
    unsafe {
        let corners: Retained<AnyObject> = if style.radius.is_finite() {
            let radius: Retained<AnyObject> =
                msg_send![class!(UICornerRadius), fixedRadius: style.radius as f64];
            msg_send![class!(UICornerConfiguration), configurationWithRadius: &*radius]
        } else {
            msg_send![class!(UICornerConfiguration), capsuleConfiguration]
        };
        let _: () = msg_send![view, setCornerConfiguration: &*corners];
    }
}

fn content_view(view: &AnyObject) -> Retained<AnyObject> {
    // SAFETY: UIVisualEffectView's getter, on the main thread.
    unsafe { msg_send![view, contentView] }
}

/// Give a pane's view `style`, animated as the system's changes are.
fn restyle(view: &AnyObject, style: GlassStyle) {
    // SAFETY: UIKit's calls, on the main thread.
    unsafe {
        let effect = glass_effect(style);
        set_corners(view, style);
        let view = Retained::retain(view as *const _ as *mut AnyObject);
        let apply = block2::RcBlock::new(move || {
            if let Some(view) = &view {
                let _: () = msg_send![&**view, setEffect: &*effect];
            }
        });
        let _: () = msg_send![class!(UIView), animateWithDuration: 0.35f64, animations: &*apply];
    }
}
