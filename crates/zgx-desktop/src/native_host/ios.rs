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
    native_control::ToggleState,
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
fn plain_view(frame: CGRect) -> Retained<AnyObject> {
    // SAFETY: UIView's initializer, on the main thread.
    unsafe { msg_send![msg_send![class!(UIView), alloc], initWithFrame: frame] }
}

fn passive_view(frame: CGRect) -> Retained<AnyObject> {
    // SAFETY: UIView's initializer and setters, on the main thread (every
    // caller holds `Surfaces`, which is made there).
    unsafe {
        let view: Retained<AnyObject> =
            msg_send![msg_send![class!(UIView), alloc], initWithFrame: frame];
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
    /// A `UISwitch` (`NativeContent::Toggle`).
    Toggle(Rc<ToggleState>, #[allow(dead_code)] Retained<ToggleTarget>),
}

impl Content {
    /// A control a person touches, cut by what clips its node.
    fn is_control(&self) -> bool {
        matches!(self, Self::Platform | Self::Toggle(..))
    }
}

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
    frame: Option<Rect>,
    alpha: f32,
    hidden: bool,
}

pub(crate) struct Surfaces {
    /// Each pane in a group, with its group (`read_groups`).
    groups: HashMap<NodeId, NodeId>,
    glass_host: Retained<AnyObject>,
    overlay_view: Retained<AnyObject>,
    overlay_shown: Option<Option<Rect>>,
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
        let (glass_host, overlay_view) = unsafe {
            let bounds: CGRect = msg_send![&*root, bounds];
            let glass_host = passive_view(bounds);
            let _: () = msg_send![&*glass_host, setAutoresizingMask: FILL];
            let _: () = msg_send![&*root, addSubview: &*glass_host];
            let overlay_view = passive_view(bounds);
            let _: () = msg_send![&*overlay_view, setHidden: true];
            let _: () = msg_send![&*root, addSubview: &*overlay_view];
            (glass_host, overlay_view)
        };
        Ok(Self {
            groups: HashMap::new(),
            glass_host,
            overlay_view,
            overlay_shown: None,
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
        // SAFETY: a UIView's setter, on the main thread.
        unsafe {
            let _: () = msg_send![&*self.overlay_view, setContentScaleFactor: scale as f64];
        }
        let view = NonNull::from(&*self.overlay_view).cast();
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
        if self.overlay_shown == Some(shown) {
            return;
        }
        self.overlay_shown = Some(shown);
        // SAFETY: UIView's setters, on the main thread.
        unsafe {
            let _: () = msg_send![&*self.overlay_view, setHidden: shown.is_none()];
            if let Some(shown) = shown {
                let _: () = msg_send![&*self.overlay_view, setFrame: rect(shown)];
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
                _ => continue,
            };
            let clip = content.is_control().then(|| {
                let clip = plain_view(rect(Rect::new(0., 0., 1., 1.)));
                // SAFETY: UIView's setter and method, on the main thread.
                unsafe {
                    let _: () = msg_send![&*clip, setClipsToBounds: true];
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
                frame: None,
                alpha: 1.,
                hidden: false,
            });
        }
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
                let _: () = msg_send![&*self.glass_host, setUserInteractionEnabled: interactive];
            }
        }
        // Later paint is higher: reorder only when the order changes.
        if order != self.order {
            for node in &order {
                if let Some(mounted) = self.mounted.get(node) {
                    let host = match mounted.parent {
                        Some(group) => self.mounted.get(&group).map(|g| content_view(&g.view)),
                        None => Some(self.glass_host.clone()),
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
            None => Some(self.glass_host.clone()),
        };
        let Some(mounted) = self.mounted.get_mut(&placement.node) else {
            return;
        };
        if mounted.parent != parent || mounted.frame.is_none() {
            mounted.parent = parent;
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
            // A control shows what clips leave of it, as an image would.
            if mounted.frame != Some(placement.frame) {
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
