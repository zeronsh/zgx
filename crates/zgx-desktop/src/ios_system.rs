//! What iOS says about how an app should look, read from UIKit: semantic
//! colours and Dynamic Type sizes as the themes' tokens, display settings as
//! `SystemPreferences`, and the window's safe area. Nothing here imitates:
//! each value is the system's for the current light or dark, contrast and text
//! size, read again when one changes.
use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use std::sync::atomic::{AtomicBool, Ordering};
use zgx::{
    scene::Color,
    theme::{SystemPreferences, TokenId, TokenInput, tok},
};

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    fn UIAccessibilityIsReduceMotionEnabled() -> bool;
    fn UIAccessibilityIsReduceTransparencyEnabled() -> bool;
    fn UIAccessibilityDarkerSystemColorsEnabled() -> bool;
    fn UIAccessibilityIsBoldTextEnabled() -> bool;
    static UIAccessibilityReduceMotionStatusDidChangeNotification: *const AnyObject;
    static UIAccessibilityReduceTransparencyStatusDidChangeNotification: *const AnyObject;
    static UIAccessibilityDarkerSystemColorsStatusDidChangeNotification: *const AnyObject;
    static UIAccessibilityBoldTextStatusDidChangeNotification: *const AnyObject;
    static UIContentSizeCategoryDidChangeNotification: *const AnyObject;
    static UIFontTextStyleLargeTitle: *const AnyObject;
    static UIFontTextStyleTitle2: *const AnyObject;
    static UIFontTextStyleTitle3: *const AnyObject;
    static UIFontTextStyleBody: *const AnyObject;
    static UIFontTextStyleSubheadline: *const AnyObject;
    static UIFontTextStyleFootnote: *const AnyObject;
}

pub(crate) fn preferences() -> SystemPreferences {
    // SAFETY: UIKit's plain getters.
    unsafe {
        SystemPreferences {
            reduced_motion: UIAccessibilityIsReduceMotionEnabled(),
            increased_contrast: UIAccessibilityDarkerSystemColorsEnabled(),
            reduced_transparency: UIAccessibilityIsReduceTransparencyEnabled(),
        }
    }
}

/// `UIColor`'s class method `name`, as the current traits resolve it.
fn system_color(name: objc2::runtime::Sel) -> Option<Color> {
    // SAFETY: a UIColor class method named below, returning a UIColor, and
    // its resolved components, on the main thread where the host syncs.
    unsafe {
        let color: Option<Retained<AnyObject>> = msg_send![class!(UIColor), performSelector: name];
        let color = color?;
        let traits: Retained<AnyObject> =
            msg_send![class!(UITraitCollection), currentTraitCollection];
        let color: Retained<AnyObject> =
            msg_send![&*color, resolvedColorWithTraitCollection: &*traits];
        let (mut r, mut g, mut b, mut a) = (0f64, 0f64, 0f64, 0f64);
        let ok: bool =
            msg_send![&*color, getRed: &mut r, green: &mut g, blue: &mut b, alpha: &mut a];
        let channel = |v: f64| (v.clamp(0., 1.) * 255.).round() as u8;
        ok.then(|| Color(channel(r), channel(g), channel(b), channel(a)))
    }
}

/// `style`'s size in points at the user's text size, and the line height
/// Apple gives it, in that ratio.
fn text_style(style: *const AnyObject, line_ratio: f32) -> (f32, f32) {
    // SAFETY: UIFont's class method with UIKit's own style name.
    let size = unsafe {
        let font: Retained<AnyObject> = msg_send![class!(UIFont), preferredFontForTextStyle: style];
        let size: f64 = msg_send![&*font, pointSize];
        size as f32
    };
    (size, (size * line_ratio).round())
}

/// The themes' tokens as UIKit has them: what makes a zgx app on iOS take the
/// system's colours and text sizes before it asks for anything.
pub(crate) fn tokens() -> Vec<(TokenId, TokenInput)> {
    use objc2::sel;
    let mut tokens: Vec<(TokenId, TokenInput)> = Vec::with_capacity(48);
    let colors = [
        (tok::BACKGROUND.0, sel!(systemBackgroundColor)),
        (tok::SURFACE_BASE.0, sel!(systemBackgroundColor)),
        (tok::SURFACE_SUNKEN.0, sel!(systemGroupedBackgroundColor)),
        (
            tok::SURFACE_RAISED.0,
            sel!(secondarySystemGroupedBackgroundColor),
        ),
        (tok::SURFACE_OVERLAY.0, sel!(tertiarySystemBackgroundColor)),
        (tok::TEXT_PRIMARY.0, sel!(labelColor)),
        (tok::TEXT_SECONDARY.0, sel!(secondaryLabelColor)),
        (tok::TEXT_TERTIARY.0, sel!(tertiaryLabelColor)),
        (tok::TEXT_DISABLED.0, sel!(quaternaryLabelColor)),
        (tok::BORDER.0, sel!(separatorColor)),
        (tok::BORDER_SUBTLE.0, sel!(separatorColor)),
        (tok::BORDER_STRONG.0, sel!(opaqueSeparatorColor)),
        (tok::ACCENT.0, sel!(systemBlueColor)),
        (tok::DANGER.0, sel!(systemRedColor)),
        (tok::WARNING.0, sel!(systemOrangeColor)),
        (tok::SUCCESS.0, sel!(systemGreenColor)),
        (tok::INFO.0, sel!(systemBlueColor)),
        (tok::BUTTON_BG.0, sel!(tertiarySystemFillColor)),
        (tok::BUTTON_HOVER.0, sel!(secondarySystemFillColor)),
        (tok::BUTTON_PRESSED.0, sel!(systemFillColor)),
        (tok::INPUT_BG.0, sel!(tertiarySystemFillColor)),
        (tok::HOVER.0, sel!(quaternarySystemFillColor)),
        (tok::PRESSED.0, sel!(systemGray4Color)),
        (tok::MENU_BG.0, sel!(secondarySystemBackgroundColor)),
        (tok::POPOVER_BG.0, sel!(secondarySystemBackgroundColor)),
    ];
    for (id, name) in colors {
        if let Some(color) = system_color(name) {
            tokens.push((id, color.into()));
        }
    }
    // Colours UIKit names that zgx's tokens don't: `$ios.…`, for a look of
    // an app's own or a kit's.
    for (name, selector) in [
        ("ios.grouped", sel!(systemGroupedBackgroundColor)),
        ("ios.cell", sel!(secondarySystemGroupedBackgroundColor)),
        ("ios.fill", sel!(systemFillColor)),
        ("ios.gray", sel!(systemGrayColor)),
        ("ios.red", sel!(systemRedColor)),
        ("ios.orange", sel!(systemOrangeColor)),
        ("ios.yellow", sel!(systemYellowColor)),
        ("ios.green", sel!(systemGreenColor)),
        ("ios.teal", sel!(systemTealColor)),
        ("ios.blue", sel!(systemBlueColor)),
        ("ios.indigo", sel!(systemIndigoColor)),
        ("ios.purple", sel!(systemPurpleColor)),
        ("ios.pink", sel!(systemPinkColor)),
    ] {
        if let Some(color) = system_color(selector) {
            tokens.push((zgx::theme::token(name), color.into()));
        }
    }
    // SAFETY: UIKit's constants, there for the process's life.
    let styles = unsafe {
        [
            (
                tok::FONT_SIZE_XS.0,
                tok::FONT_LINE_XS.0,
                UIFontTextStyleFootnote,
                18. / 13.,
            ),
            (
                tok::FONT_SIZE_SM.0,
                tok::FONT_LINE_SM.0,
                UIFontTextStyleSubheadline,
                20. / 15.,
            ),
            (
                tok::FONT_SIZE_MD.0,
                tok::FONT_LINE_MD.0,
                UIFontTextStyleBody,
                22. / 17.,
            ),
            (
                tok::FONT_SIZE_LG.0,
                tok::FONT_LINE_LG.0,
                UIFontTextStyleTitle3,
                25. / 20.,
            ),
            (
                tok::FONT_SIZE_XL.0,
                tok::FONT_LINE_XL.0,
                UIFontTextStyleTitle2,
                28. / 22.,
            ),
            (
                tok::FONT_SIZE_XXL.0,
                tok::FONT_LINE_XXL.0,
                UIFontTextStyleLargeTitle,
                41. / 34.,
            ),
        ]
    };
    for (size_token, line_token, style, ratio) in styles {
        let (size, line) = text_style(style, ratio);
        tokens.push((size_token, size.into()));
        tokens.push((line_token, line.into()));
    }
    // SAFETY: UIKit's plain getter.
    if unsafe { UIAccessibilityIsBoldTextEnabled() } {
        tokens.push((tok::FONT_WEIGHT_REGULAR.0, 600f32.into()));
        tokens.push((tok::FONT_WEIGHT_MEDIUM.0, 700f32.into()));
    }
    // UIKit's hit target, growing with the text it holds.
    // SAFETY: UIKit's constant, there for the process's life.
    let (_, body_line) = text_style(unsafe { UIFontTextStyleBody }, 22. / 17.);
    tokens.push((tok::CONTROL_HEIGHT.0, (body_line + 12.).max(44.).into()));
    let preferences = preferences();
    if preferences.reduced_motion {
        tokens.extend([
            (tok::MOTION_DURATION_FAST.0, 0f32.into()),
            (tok::MOTION_DURATION_NORMAL.0, 0f32.into()),
            (tok::MOTION_DURATION_SLOW.0, 0f32.into()),
        ]);
    }
    tokens
}

static CHANGED: AtomicBool = AtomicBool::new(false);
/// How much of the screen the keyboard is about to cover, in points, as bits.
static KEYBOARD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// How much of the screen's height the keyboard covers, in points. The first
/// call starts listening.
pub(crate) fn keyboard_height() -> f32 {
    static LISTENING: std::sync::Once = std::sync::Once::new();
    LISTENING.call_once(|| {
        unsafe extern "C" {
            static UIKeyboardWillChangeFrameNotification: *const AnyObject;
            static UIKeyboardFrameEndUserInfoKey: *const AnyObject;
        }
        let block = block2::RcBlock::new(|note: *mut AnyObject| {
            // SAFETY: UIKit's notification, its user info holding the frame in
            // an NSValue, read on the main thread it posts on.
            unsafe {
                let info: Option<Retained<AnyObject>> = msg_send![note, userInfo];
                let Some(info) = info else { return };
                let value: Option<Retained<AnyObject>> =
                    msg_send![&*info, objectForKey: UIKeyboardFrameEndUserInfoKey];
                let Some(value) = value else { return };
                let frame: objc2_core_foundation::CGRect = msg_send![&*value, CGRectValue];
                let screen: Retained<AnyObject> = msg_send![class!(UIScreen), mainScreen];
                let bounds: objc2_core_foundation::CGRect = msg_send![&*screen, bounds];
                let covered = (bounds.size.height - frame.origin.y).max(0.) as f32;
                KEYBOARD.store(covered.to_bits(), Ordering::Relaxed);
            }
        });
        // SAFETY: NSNotificationCenter's method, with UIKit's own name; the
        // block lives as long as the app, with its observer.
        unsafe {
            let center: Retained<AnyObject> =
                msg_send![class!(NSNotificationCenter), defaultCenter];
            let observer: Retained<AnyObject> = msg_send![&*center,
                addObserverForName: UIKeyboardWillChangeFrameNotification,
                object: std::ptr::null::<AnyObject>(),
                queue: std::ptr::null::<AnyObject>(),
                usingBlock: &*block];
            std::mem::forget(observer);
        }
    });
    f32::from_bits(KEYBOARD.load(Ordering::Relaxed))
}

/// A display setting or the text size changed since the last call. The
/// first call starts listening.
pub(crate) fn options_changed() -> bool {
    static LISTENING: std::sync::Once = std::sync::Once::new();
    LISTENING.call_once(|| {
        let block = block2::RcBlock::new(|_: *mut AnyObject| {
            CHANGED.store(true, Ordering::Relaxed);
        });
        // SAFETY: NSNotificationCenter's method, with UIKit's own names; the
        // block only sets a flag; the observers last as long as the app.
        unsafe {
            let center: Retained<AnyObject> =
                msg_send![class!(NSNotificationCenter), defaultCenter];
            for name in [
                UIAccessibilityReduceMotionStatusDidChangeNotification,
                UIAccessibilityReduceTransparencyStatusDidChangeNotification,
                UIAccessibilityDarkerSystemColorsStatusDidChangeNotification,
                UIAccessibilityBoldTextStatusDidChangeNotification,
                UIContentSizeCategoryDidChangeNotification,
            ] {
                let observer: Retained<AnyObject> = msg_send![&*center,
                    addObserverForName: name,
                    object: std::ptr::null::<AnyObject>(),
                    queue: std::ptr::null::<AnyObject>(),
                    usingBlock: &*block];
                std::mem::forget(observer);
            }
        }
    });
    CHANGED.swap(false, Ordering::Relaxed)
}

/// The window's safe area in points: top, right, bottom, left.
pub(crate) fn safe_area(window: &winit::window::Window) -> [f32; 4] {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return [0.; 4];
    };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else {
        return [0.; 4];
    };
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Insets {
        top: f64,
        left: f64,
        bottom: f64,
        right: f64,
    }
    // SAFETY: the encoding of UIEdgeInsets, four CGFloats.
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
    // SAFETY: winit's view, alive with its window; UIView's getter.
    let insets: Insets = unsafe {
        let view: *const AnyObject = handle.ui_view.as_ptr().cast();
        msg_send![&*view, safeAreaInsets]
    };
    [
        insets.top as f32,
        insets.right as f32,
        insets.bottom as f32,
        insets.left as f32,
    ]
}

/// A system symbol's pixels (`zgx::symbol`): `UIImage`'s, in `style`'s size,
/// weight and colour, at the screen's scale.
pub(crate) fn symbol(
    name: &str,
    style: zgx::symbol::SymbolStyle,
) -> Option<(std::sync::Arc<zgx::image::ImageData>, f32, f32)> {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGColorSpaceCreateDeviceRGB() -> *mut std::ffi::c_void;
        fn CGColorSpaceRelease(space: *mut std::ffi::c_void);
        fn CGBitmapContextCreate(
            data: *mut std::ffi::c_void,
            width: usize,
            height: usize,
            bits: usize,
            row: usize,
            space: *mut std::ffi::c_void,
            info: u32,
        ) -> *mut std::ffi::c_void;
        fn CGContextRelease(context: *mut std::ffi::c_void);
        fn CGContextTranslateCTM(context: *mut std::ffi::c_void, x: f64, y: f64);
        fn CGContextScaleCTM(context: *mut std::ffi::c_void, x: f64, y: f64);
    }
    #[link(name = "UIKit", kind = "framework")]
    unsafe extern "C" {
        fn UIGraphicsPushContext(context: *mut std::ffi::c_void);
        fn UIGraphicsPopContext();
    }
    // UIImageSymbolWeight: ultraLight 1 … black 9, as CSS's hundreds.
    let weight = (style.weight.clamp(100, 900) / 100) as isize;
    // SAFETY: UIKit's constructors and getters, on the main thread where
    // views lower; a bitmap context over `pixels`, which outlives it, with
    // the premultiplied RGBA layout its info names.
    unsafe {
        let name = objc2_foundation_string(name);
        let config: Retained<AnyObject> = msg_send![class!(UIImageSymbolConfiguration),
            configurationWithPointSize: style.size as f64, weight: weight];
        let image: Option<Retained<AnyObject>> = msg_send![class!(UIImage),
            systemImageNamed: &*name, withConfiguration: &*config];
        let image = image?;
        let channel = |v: u8| v as f64 / 255.;
        let color: Retained<AnyObject> = msg_send![class!(UIColor),
            colorWithRed: channel(style.color.0),
            green: channel(style.color.1),
            blue: channel(style.color.2),
            alpha: channel(style.color.3)];
        // UIImageRenderingModeAlwaysOriginal keeps the tint it is given.
        let image: Retained<AnyObject> =
            msg_send![&*image, imageWithTintColor: &*color, renderingMode: 1isize];
        let size: CGSize = msg_send![&*image, size];
        let screen: Retained<AnyObject> = msg_send![class!(UIScreen), mainScreen];
        let scale: f64 = msg_send![&*screen, scale];
        let width = (size.width * scale).ceil().max(1.) as usize;
        let height = (size.height * scale).ceil().max(1.) as usize;
        let mut pixels = vec![0u8; width * height * 4];
        let space = CGColorSpaceCreateDeviceRGB();
        // kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big.
        let info = 1u32 | (4 << 12);
        let context = CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            width,
            height,
            8,
            width * 4,
            space,
            info,
        );
        CGColorSpaceRelease(space);
        if context.is_null() {
            return None;
        }
        // UIKit's top-left origin over the bitmap's bottom-left one.
        CGContextTranslateCTM(context, 0., height as f64);
        CGContextScaleCTM(context, scale, -scale);
        UIGraphicsPushContext(context);
        let rect = CGRect {
            origin: CGPoint { x: 0., y: 0. },
            size,
        };
        let _: () = msg_send![&*image, drawInRect: rect];
        UIGraphicsPopContext();
        CGContextRelease(context);
        // The image type holds straight alpha.
        for pixel in pixels.chunks_exact_mut(4) {
            let alpha = pixel[3] as u32;
            if alpha != 0 && alpha != 255 {
                for channel in &mut pixel[..3] {
                    *channel = ((*channel as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
                }
            }
        }
        let data = zgx::image::ImageData::new(width as u32, height as u32, pixels).ok()?;
        Some((
            std::sync::Arc::new(data),
            size.width as f32,
            size.height as f32,
        ))
    }
}

/// An `NSString` of `text`.
pub(crate) fn ns_string(text: &str) -> Retained<AnyObject> {
    // SAFETY: as `objc2_foundation_string` promises.
    unsafe { objc2_foundation_string(text) }
}

/// # Safety
/// On any thread: NSString's constructor, over a NUL-terminated copy.
unsafe fn objc2_foundation_string(text: &str) -> Retained<AnyObject> {
    let text = std::ffi::CString::new(text).unwrap_or_default();
    // SAFETY: as above.
    unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
}

/// `ZGX_IOS_AX_DUMP=1`: three seconds in, print what assistive technology is
/// offered from `window`'s view, as `dump_element` walks it: a check of the
/// tree where VoiceOver doesn't run (a simulator).
pub(crate) fn dump_accessibility_later(window: &winit::window::Window) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    if zgx::env::var_os("ZGX_IOS_AX_DUMP").is_none() {
        return;
    }
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else {
        return;
    };
    let view = handle.ui_view.as_ptr() as usize;
    let fire = block2::RcBlock::new(move |_: *mut AnyObject| {
        // SAFETY: winit's view, alive with its window for the app's life on
        // iOS; this timer fires on the main thread.
        println!("AX dump");
        dump_element(unsafe { &*(view as *const AnyObject) }, 0);
    });
    // Twice: the first read asks the host for the tree, which comes after.
    for delay in [3f64, 4.5] {
        // SAFETY: NSTimer's constructor; the run loop keeps it.
        let _: Retained<AnyObject> = unsafe {
            msg_send![class!(NSTimer),
                scheduledTimerWithTimeInterval: delay,
                repeats: false,
                block: &*fire]
        };
    }
}

fn dump_element(element: &AnyObject, depth: usize) {
    let text = |value: Option<Retained<AnyObject>>| -> String {
        // SAFETY: an NSString's UTF-8 view, copied before it can go.
        value.map_or(String::new(), |value| unsafe {
            let bytes: *const std::ffi::c_char = msg_send![&*value, UTF8String];
            if bytes.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(bytes)
                    .to_string_lossy()
                    .into_owned()
            }
        })
    };
    // SAFETY: the UIAccessibility informal protocol, which every NSObject
    // answers, on the main thread.
    unsafe {
        let is_element: objc2::runtime::Bool = msg_send![element, isAccessibilityElement];
        if is_element.as_bool() {
            let frame: objc2_core_foundation::CGRect = msg_send![element, accessibilityFrame];
            let traits: u64 = msg_send![element, accessibilityTraits];
            println!(
                "AX {}label={:?} value={:?} hint={:?} traits={traits:#x} frame=({:.0},{:.0},{:.0},{:.0})",
                " ".repeat(depth),
                text(msg_send![element, accessibilityLabel]),
                text(msg_send![element, accessibilityValue]),
                text(msg_send![element, accessibilityHint]),
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            );
        }
        let children: Option<Retained<AnyObject>> = msg_send![element, accessibilityElements];
        let Some(children) = children else {
            return;
        };
        let count: usize = msg_send![&*children, count];
        for index in 0..count {
            let child: Retained<AnyObject> = msg_send![&*children, objectAtIndex: index];
            dump_element(&child, depth + 1);
        }
    }
}

/// Haptics and alerts, as UIKit does them.
pub(crate) struct IosSystemUi;
impl zgx::native_control::SystemUi for IosSystemUi {
    fn haptic(&self, kind: &str) {
        // SAFETY: UIKit's constructors and methods, on the main thread.
        unsafe {
            match kind {
                "success" | "warning" | "error" => {
                    let kind: isize = match kind {
                        "success" => 0,
                        "warning" => 1,
                        _ => 2,
                    };
                    let made: Retained<AnyObject> =
                        msg_send![class!(UINotificationFeedbackGenerator), new];
                    let _: () = msg_send![&*made, notificationOccurred: kind];
                }
                "selection" => {
                    let made: Retained<AnyObject> =
                        msg_send![class!(UISelectionFeedbackGenerator), new];
                    let _: () = msg_send![&*made, selectionChanged];
                }
                _ => {
                    let style: isize = match kind {
                        "medium" => 1,
                        "heavy" => 2,
                        "soft" => 3,
                        "rigid" => 4,
                        _ => 0,
                    };
                    let made: Retained<AnyObject> = msg_send![msg_send![class!(UIImpactFeedbackGenerator), alloc], initWithStyle: style];
                    let _: () = msg_send![&*made, impactOccurred];
                }
            }
        }
    }
    fn alert(
        &self,
        style: zgx::native_control::AlertStyle,
        title: &str,
        message: &str,
        buttons: &[(String, zgx::native_control::AlertButtonStyle)],
        done: Box<dyn FnOnce(usize)>,
    ) {
        use zgx::native_control::{AlertButtonStyle, AlertStyle};
        let done = std::rc::Rc::new(std::cell::RefCell::new(Some(done)));
        // SAFETY: UIKit's constructors and methods, on the main thread. The
        // window's root view controller presents; a sheet on an iPad needs a
        // place to point from, the middle of the window.
        unsafe {
            let preferred: isize = match style {
                AlertStyle::Alert => 1,
                AlertStyle::ActionSheet => 0,
            };
            let title = (!title.is_empty()).then(|| ns_string(title));
            let message = (!message.is_empty()).then(|| ns_string(message));
            let controller: Retained<AnyObject> = msg_send![class!(UIAlertController),
                alertControllerWithTitle: title.as_deref().map_or(std::ptr::null(), |t| t as *const AnyObject),
                message: message.as_deref().map_or(std::ptr::null(), |m| m as *const AnyObject),
                preferredStyle: preferred];
            for (index, (label, kind)) in buttons.iter().enumerate() {
                let kind: isize = match kind {
                    AlertButtonStyle::Default => 0,
                    AlertButtonStyle::Cancel => 1,
                    AlertButtonStyle::Destructive => 2,
                };
                let done = done.clone();
                let handler = block2::RcBlock::new(move |_: *mut AnyObject| {
                    if let Some(done) = done.borrow_mut().take() {
                        done(index);
                    }
                    zgx::native_view::wake();
                });
                let action: Retained<AnyObject> = msg_send![class!(UIAlertAction),
                    actionWithTitle: &*ns_string(label),
                    style: kind,
                    handler: &*handler];
                let _: () = msg_send![&*controller, addAction: &*action];
            }
            let Some(root) = root_controller() else {
                return;
            };
            let popover: Option<Retained<AnyObject>> =
                msg_send![&*controller, popoverPresentationController];
            if let Some(popover) = popover {
                let view: Retained<AnyObject> = msg_send![&*root, view];
                let bounds: objc2_core_foundation::CGRect = msg_send![&*view, bounds];
                let middle = objc2_core_foundation::CGRect {
                    origin: objc2_core_foundation::CGPoint {
                        x: bounds.size.width / 2.,
                        y: bounds.size.height / 2.,
                    },
                    size: objc2_core_foundation::CGSize {
                        width: 1.,
                        height: 1.,
                    },
                };
                let _: () = msg_send![&*popover, setSourceView: &*view];
                let _: () = msg_send![&*popover, setSourceRect: middle];
            }
            let _: () = msg_send![&*root,
                presentViewController: &*controller,
                animated: true,
                completion: std::ptr::null::<AnyObject>()];
        }
    }
}

/// The key window's root view controller: what presents the system's sheets.
pub(crate) fn root_controller() -> Option<Retained<AnyObject>> {
    // SAFETY: UIApplication's getters, on the main thread. `keyWindow` is
    // deprecated for scenes; winit's app has the one window.
    unsafe {
        let app: Retained<AnyObject> = msg_send![class!(UIApplication), sharedApplication];
        let windows: Retained<AnyObject> = msg_send![&*app, windows];
        let first: Option<Retained<AnyObject>> = msg_send![&*windows, firstObject];
        let window = first?;
        msg_send![&*window, rootViewController]
    }
}
