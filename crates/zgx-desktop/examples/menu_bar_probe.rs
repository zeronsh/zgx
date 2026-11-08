//! AppKit's menu bar made once, not on every mouse move (macOS): with the bar
//! set to hide itself, an app that activated before its first main menu was
//! in had AppKit make the bar's windows again for each move it was given.
//! Run with `-_HIHideMenuBar YES` (this process's own defaults: the user's
//! setting is untouched), it counts the backing views AppKit makes for the
//! bar, and whether the app activated before or after its bar was set, over
//! 200 mouse moves posted to its own window (the user's pointer is never
//! moved, nothing outside the process is touched). The count reads AppKit's
//! private `-[NSMenuBarImpl _newMenuBarStackViewWithAppearance:]`, a probe's
//! instrument, never the library's. Prints one line for
//! `scripts/macos_menu_bar_probe.py`.
use zgx::compose::prelude::*;
use zgx_desktop::{AppMenu, AppMenuEntry, Application, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    macos::count();
    Application::new()
        .menus(vec![AppMenu::new(
            "Probe",
            [AppMenuEntry::item("Nothing", "nothing")],
        )])?
        .window(WindowOptions {
            title: "Menu bar probe".into(),
            width: 640.,
            height: 420.,
            ..Default::default()
        })
        .run(|cx| {
            cx.render(div().w_full().h_full());
            #[cfg(target_os = "macos")]
            {
                let window = cx.window.clone();
                cx.tasks.spawn(async move {
                    macos::run().await;
                    window.close();
                });
            }
            #[cfg(not(target_os = "macos"))]
            {
                println!("macOS only");
                cx.window.close();
            }
        })
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::{MainThreadMarker, rc::Retained};
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSWindow};
    use objc2_foundation::NSPoint;
    use std::ffi::{CStr, c_char, c_void};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::time::Duration;
    use zgx_gpu::present_log::now_ns;

    type Id = *mut c_void;
    type Sel = *const c_void;
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn class_getInstanceMethod(class: Id, sel: Sel) -> *mut c_void;
        fn method_setImplementation(method: *mut c_void, imp: *const c_void) -> *const c_void;
    }
    /// Each hooked method's own implementation, called through.
    static BUILT_IMP: AtomicUsize = AtomicUsize::new(0);
    static ACTIVATED_IMP: AtomicUsize = AtomicUsize::new(0);
    static MENU_IMP: AtomicUsize = AtomicUsize::new(0);
    /// Backing views made for the bar.
    static BUILT: AtomicUsize = AtomicUsize::new(0);
    /// When the app first activated, and when its main menu was first set
    /// (ns, 0 for never).
    static ACTIVATED_AT: AtomicU64 = AtomicU64::new(0);
    static MENU_AT: AtomicU64 = AtomicU64::new(0);

    type Imp = extern "C" fn(Id, Sel, Id) -> Id;
    fn through(imp: &AtomicUsize) -> Imp {
        // SAFETY: stored from `method_setImplementation` for a method of
        // one object argument (each hooked one's signature).
        unsafe { std::mem::transmute::<usize, Imp>(imp.load(Ordering::Relaxed)) }
    }
    extern "C" fn built(this: Id, sel: Sel, appearance: Id) -> Id {
        BUILT.fetch_add(1, Ordering::Relaxed);
        through(&BUILT_IMP)(this, sel, appearance)
    }
    extern "C" fn activated(this: Id, sel: Sel, event: Id) -> Id {
        let _ = ACTIVATED_AT.compare_exchange(0, now_ns(), Ordering::Relaxed, Ordering::Relaxed);
        through(&ACTIVATED_IMP)(this, sel, event)
    }
    extern "C" fn set_menu(this: Id, sel: Sel, menu: Id) -> Id {
        if !menu.is_null() {
            let _ = MENU_AT.compare_exchange(0, now_ns(), Ordering::Relaxed, Ordering::Relaxed);
        }
        through(&MENU_IMP)(this, sel, menu)
    }
    fn hook(class: &CStr, sel: &CStr, imp: Imp, original: &AtomicUsize) {
        // SAFETY: the class and method are looked up, and replaced only when
        // found, by an implementation of the same signature.
        unsafe {
            let class = objc_getClass(class.as_ptr());
            let method = if class.is_null() {
                std::ptr::null_mut()
            } else {
                class_getInstanceMethod(class, sel_registerName(sel.as_ptr()))
            };
            if method.is_null() {
                println!("MENU_BAR missing {class:?} {sel:?}");
                return;
            }
            original.store(
                method_setImplementation(method, imp as *const c_void) as usize,
                Ordering::Relaxed,
            );
        }
    }
    /// Counted from before the app launches: its activation and first bar
    /// come within its first turns.
    pub fn count() {
        hook(
            c"NSMenuBarImpl",
            c"_newMenuBarStackViewWithAppearance:",
            built,
            &BUILT_IMP,
        );
        hook(
            c"NSApplication",
            c"_handleActivatedEvent:",
            activated,
            &ACTIVATED_IMP,
        );
        hook(c"NSApplication", c"setMainMenu:", set_menu, &MENU_IMP);
    }

    fn window() -> Option<Retained<NSWindow>> {
        let mtm = MainThreadMarker::new()?;
        NSApplication::sharedApplication(mtm)
            .windows()
            .iter()
            .find(|window| window.title().to_string() == "Menu bar probe")
    }
    fn post_move(window: &NSWindow, i: usize) {
        let angle = i as f64 * 0.7;
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            NSEventType::MouseMoved,
            NSPoint::new(320. + 2. * angle.cos(), 210. + 2. * angle.sin()),
            NSEventModifierFlags::empty(),
            now_ns() as f64 / 1e9,
            window.windowNumber(),
            None,
            0,
            0,
            0.,
        )
        .unwrap();
        let mtm = MainThreadMarker::new().unwrap();
        NSApplication::sharedApplication(mtm).postEvent_atStart(&event, false);
    }
    pub async fn run() {
        // Launched, activated, its bar in.
        zgx::timer::sleep(Duration::from_millis(3000)).await;
        let window = window().expect("probe window");
        let before = BUILT.load(Ordering::Relaxed);
        for i in 0..200 {
            post_move(&window, i);
            zgx::timer::sleep(Duration::from_millis(8)).await;
        }
        zgx::timer::sleep(Duration::from_millis(300)).await;
        let moves = BUILT.load(Ordering::Relaxed) - before;
        let (activated, menu) = (
            ACTIVATED_AT.load(Ordering::Relaxed),
            MENU_AT.load(Ordering::Relaxed),
        );
        let order = match (activated, menu) {
            (0, _) | (_, 0) => "unknown",
            (a, m) if a < m => "activated_first",
            _ => "menu_first",
        };
        println!("MENU_BAR order={order} built_at_launch={before} built_over_200_moves={moves}");
    }
}
