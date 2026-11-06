//! Long-pressing a node with a system menu (`ui.system_menu`) is
//! `UIContextMenuInteraction`'s: the node lifts out of the page, the rest blurs,
//! and the menu is UIKit's, with its haptic and its Haptic Touch timing.
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol},
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use std::{cell::RefCell, rc::Rc};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use zgx::{
    native_control::{MenuAction, system_menu_at},
    scene::{Rect, Scene},
};

struct State {
    scene: Rc<RefCell<Scene>>,
    window: Rc<dyn Fn()>,
    /// What the menu showing was asked for, in the window's points.
    rect: std::cell::Cell<Rect>,
    root: Retained<AnyObject>,
}

fn cg(rect: Rect) -> CGRect {
    CGRect {
        origin: CGPoint {
            x: rect.x as f64,
            y: rect.y as f64,
        },
        size: CGSize {
            width: rect.width as f64,
            height: rect.height as f64,
        },
    }
}

fn ns_string(text: &str) -> Retained<AnyObject> {
    crate::ios_system::ns_string(text)
}

/// `actions` as a `UIMenu`.
fn menu(actions: Vec<MenuAction>, redraw: Rc<dyn Fn()>) -> *mut AnyObject {
    // SAFETY: UIKit's constructors, on the main thread; each handler holds what
    // it runs, and the array is the menu's.
    unsafe {
        let children: Retained<AnyObject> = msg_send![class!(NSMutableArray), array];
        for action in actions {
            let run = action.run.clone();
            let redraw = redraw.clone();
            let handler = block2::RcBlock::new(move |_: *mut AnyObject| {
                run();
                redraw();
            });
            let title = ns_string(&action.title);
            let image: Option<Retained<AnyObject>> = action.symbol.as_deref().and_then(|name| {
                let name = ns_string(name);
                msg_send![class!(UIImage), systemImageNamed: &*name]
            });
            let identifier: *const AnyObject = std::ptr::null();
            let item: Retained<AnyObject> = msg_send![class!(UIAction),
                actionWithTitle: &*title,
                image: image.as_deref(),
                identifier: identifier,
                handler: &*handler];
            if action.destructive {
                // UIMenuElementAttributesDestructive.
                let _: () = msg_send![&*item, setAttributes: 2usize];
            }
            let _: () = msg_send![&*children, addObject: &*item];
        }
        let title = ns_string("");
        let made: Retained<AnyObject> =
            msg_send![class!(UIMenu), menuWithTitle: &*title, children: &*children];
        Retained::autorelease_return(made)
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<State>]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    impl Delegate {
        #[unsafe(method(contextMenuInteraction:configurationForMenuAtLocation:))]
        fn configuration(&self, _interaction: &AnyObject, at: CGPoint) -> *mut AnyObject {
            let state = self.ivars();
            let found = {
                let scene = state.scene.borrow();
                system_menu_at(&scene, at.x as f32, at.y as f32)
            };
            let Some((rect, actions)) = found else {
                return std::ptr::null_mut();
            };
            state.rect.set(rect);
            let redraw = state.window.clone();
            let actions = RefCell::new(Some(actions));
            let provider = block2::RcBlock::new(move |_: *mut AnyObject| -> *mut AnyObject {
                match actions.borrow_mut().take() {
                    Some(actions) => menu(actions, redraw.clone()),
                    None => std::ptr::null_mut(),
                }
            });
            let identifier: *const AnyObject = std::ptr::null();
            let preview: *const AnyObject = std::ptr::null();
            // SAFETY: UIContextMenuConfiguration's constructor, which keeps the
            // block; no custom preview, the node itself lifting.
            unsafe {
                let made: Retained<AnyObject> = msg_send![class!(UIContextMenuConfiguration),
                    configurationWithIdentifier: identifier,
                    previewProvider: preview,
                    actionProvider: &*provider];
                Retained::autorelease_return(made)
            }
        }
        #[unsafe(method(contextMenuInteraction:configuration:highlightPreviewForItemWithIdentifier:))]
        fn highlight(
            &self,
            _interaction: &AnyObject,
            _configuration: &AnyObject,
            _identifier: &AnyObject,
        ) -> *mut AnyObject {
            self.preview()
        }
        #[unsafe(method(contextMenuInteraction:previewForHighlightingMenuWithConfiguration:))]
        fn highlighting(&self, _interaction: &AnyObject, _configuration: &AnyObject) -> *mut AnyObject {
            self.preview()
        }
    }
);

impl Delegate {
    /// What lifts: what the node's box shows now, where it is.
    fn preview(&self) -> *mut AnyObject {
        let state = self.ivars();
        let rect = cg(state.rect.get());
        let zero = objc2_foundation_insets();
        // SAFETY: UIKit's constructors and getters, on the main thread.
        unsafe {
            let shot: Option<Retained<AnyObject>> = msg_send![&*state.root,
                resizableSnapshotViewFromRect: rect,
                afterScreenUpdates: false,
                withCapInsets: zero];
            let Some(shot) = shot else {
                return std::ptr::null_mut();
            };
            let center = CGPoint {
                x: rect.origin.x + rect.size.width / 2.,
                y: rect.origin.y + rect.size.height / 2.,
            };
            let target: Retained<AnyObject> = msg_send![msg_send![class!(UIPreviewTarget), alloc],
                initWithContainer: &*state.root,
                center: center];
            let parameters: Retained<AnyObject> = msg_send![class!(UIPreviewParameters), new];
            let made: Retained<AnyObject> = msg_send![msg_send![class!(UITargetedPreview), alloc],
                initWithView: &*shot,
                parameters: &*parameters,
                target: &*target];
            Retained::autorelease_return(made)
        }
    }
}

/// `UIEdgeInsets`, all zero.
#[repr(C)]
struct Insets(f64, f64, f64, f64);
// SAFETY: four CGFloats, as UIEdgeInsets is declared on a 64-bit platform.
unsafe impl objc2::Encode for Insets {
    const ENCODING: objc2::Encoding = objc2::Encoding::Struct(
        "UIEdgeInsets",
        &[
            objc2::Encoding::Double,
            objc2::Encoding::Double,
            objc2::Encoding::Double,
            objc2::Encoding::Double,
        ],
    );
}
fn objc2_foundation_insets() -> Insets {
    Insets(0., 0., 0., 0.)
}

/// Long presses over `window` open the system menus of what is under them.
pub(crate) fn install(window: &std::sync::Arc<winit::window::Window>, scene: Rc<RefCell<Scene>>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: winit's view, alive with its window.
    let Some(root) = (unsafe { Retained::<AnyObject>::retain(handle.ui_view.as_ptr().cast()) })
    else {
        return;
    };
    let redraw = window.clone();
    let delegate = Delegate::alloc(mtm).set_ivars(Rc::new(State {
        scene,
        window: Rc::new(move || redraw.request_redraw()),
        rect: std::cell::Cell::new(Rect::new(0., 0., 0., 0.)),
        root: root.clone(),
    }));
    // SAFETY: NSObject's `init`; UIKit's constructor and method on the main
    // thread. The interaction doesn't keep its delegate: forgotten, as the window
    // it serves lasts the app's life.
    unsafe {
        let delegate: Retained<Delegate> = msg_send![super(delegate), init];
        let interaction: Retained<AnyObject> = msg_send![msg_send![class!(UIContextMenuInteraction), alloc],
            initWithDelegate: &*delegate];
        let _: () = msg_send![&*root, addInteraction: &*interaction];
        std::mem::forget(delegate);
    }
}
