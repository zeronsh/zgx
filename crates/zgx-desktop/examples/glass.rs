//! A pane of `zgx::glass` over stripes, with text over it: the system's glass
//! on macOS 26, drawn elsewhere (`GLASS_DRAWN=1` here too). The layout
//! `scripts/glass_compare.py` measures. `GLASS_DARK=1`: dark stripes.
//! `GLASS_CLEAR=1`: the clear variant. With `GLASS_OUT=path` on macOS, the
//! window's own pixels as the window server composited them go there as a PNG
//! after a moment, and it quits (a window may read itself, asking no
//! permission). `ZGX_NO_ACTIVATE=1` shows it without taking focus.
//! Run: cargo run -p zgx-desktop --example glass
use std::time::Duration;
use zgx::compose::prelude::*;
use zgx::glass::{GlassStyle, GlassVariant, glass};
use zgx::scene::Color;
use zgx_desktop::{Application, WindowOptions};

fn stripes(dark: bool) -> View {
    let colors: &[Color] = if dark {
        &[
            Color(12, 14, 20, 255),
            Color(60, 20, 90, 255),
            Color(10, 70, 90, 255),
            Color(90, 30, 30, 255),
        ]
    } else {
        &[
            Color(242, 38, 38, 255),
            Color(26, 64, 242, 255),
            Color(245, 200, 60, 255),
            Color(255, 255, 255, 255),
        ]
    };
    let mut column = div()
        .absolute()
        .left(0.)
        .top(0.)
        .w_full()
        .h_full()
        .flex_col();
    for row in 0..20 {
        column = column.child(div().w_full().h(16.).bg(colors[(19 - row) % colors.len()]));
    }
    column
}

/// The window's own pixels, to `path`.
#[cfg(target_os = "macos")]
fn capture(path: &str) {
    use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_app_kit::{NSApplication, NSBitmapImageFileType};
    #[repr(C)]
    struct CgImage(u8);
    // SAFETY: an opaque CGImage, encoded as the runtime has it.
    unsafe impl objc2::encode::RefEncode for CgImage {
        const ENCODING_REF: objc2::encode::Encoding =
            objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Struct("CGImage", &[]));
    }
    type Capture =
        unsafe extern "C" fn(objc2_core_foundation::CGRect, u32, u32, u32) -> *mut CgImage;
    unsafe extern "C" {
        fn dlsym(
            handle: *mut std::ffi::c_void,
            symbol: *const std::ffi::c_char,
        ) -> *mut std::ffi::c_void;
        fn dlopen(path: *const std::ffi::c_char, mode: i32) -> *mut std::ffi::c_void;
    }
    // SAFETY: the system's symbol, called as declared (an API the SDK has
    // retired, so looked up); AppKit's calls, on the main thread.
    unsafe {
        let framework = dlopen(
            c"/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics".as_ptr(),
            2,
        );
        let symbol = dlsym(framework, c"CGWindowListCreateImage".as_ptr());
        assert!(!symbol.is_null(), "the capture function");
        let capture: Capture = std::mem::transmute(symbol);
        let app = NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
        let window = app.windows().firstObject().expect("the window");
        let null = objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(f64::INFINITY, f64::INFINITY),
            objc2_core_foundation::CGSize::new(0., 0.),
        );
        // Just this window, without its frame, at its pixels.
        let image = capture(null, 8, window.windowNumber() as u32, 1 | 8);
        assert!(!image.is_null(), "the window's pixels");
        let rep: *mut AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
        let rep: Retained<AnyObject> =
            Retained::from_raw(msg_send![rep, initWithCGImage: image]).unwrap();
        let properties =
            objc2_foundation::NSDictionary::<objc2_foundation::NSString, AnyObject>::new();
        let png: Retained<objc2_foundation::NSData> = msg_send![&*rep,
            representationUsingType: NSBitmapImageFileType::PNG,
            properties: &*properties];
        std::fs::write(path, png.to_vec()).expect("the file");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dark = zgx::env::var("GLASS_DARK").is_ok();
    let clear = zgx::env::var("GLASS_CLEAR").is_ok();
    let drawn = zgx::env::var("GLASS_DRAWN").is_ok();
    let out = zgx::env::var("GLASS_OUT").ok();
    Application::new()
        .window(WindowOptions {
            title: "zgx glass".into(),
            width: 420.,
            height: 320.,
            // `GLASS_WINDOW=1`: the system's glass behind the whole window.
            glass: zgx::env::var("GLASS_WINDOW")
                .is_ok()
                .then(|| zgx_desktop::Glass::new(zgx_desktop::GlassMaterial::Liquid)),
            ..Default::default()
        })
        .run(move |cx| {
            cx.render(component(move |cx| {
                if let Some(out) = out.clone() {
                    cx.tasks().spawn(async move {
                        zgx::timer::sleep(Duration::from_millis(1800)).await;
                        #[cfg(target_os = "macos")]
                        capture(&out);
                        let _ = &out;
                        std::process::exit(0);
                    });
                }
                let style = move || GlassStyle {
                    variant: if clear {
                        GlassVariant::Clear
                    } else {
                        GlassVariant::Regular
                    },
                    ..Default::default()
                };
                let word = || {
                    text("Over the glass")
                        .text_size(20.)
                        .text_color(Color(255, 255, 255, 255))
                        .ml(70.)
                        .mt(84.)
                };
                let pane = match drawn {
                    true => div()
                        .w(300.)
                        .h(200.)
                        .child(
                            div()
                                .absolute()
                                .left(0.)
                                .top(0.)
                                .w_full()
                                .h_full()
                                .rounded(24.)
                                .shader_drive(move || style().drawn(true)),
                        )
                        .child(word()),
                    false => glass(style).w(300.).h(200.).child(word()),
                };
                let whole = zgx::env::var("GLASS_WINDOW").is_ok();
                let mut root = div().size(420., 320.).relative();
                if !whole {
                    root = root.child(stripes(dark));
                }
                root.child(div().absolute().left(60.).top(60.).child(pane))
            }));
        })
}
