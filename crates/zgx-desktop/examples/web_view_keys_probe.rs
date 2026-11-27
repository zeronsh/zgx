//! An app's shortcuts while a web page's text field has the keyboard
//! (macOS), off-screen (`ZGX_OFFSCREEN=1`: no window is ever shown).
//!
//! An app like a browser: its keys are menu items, most of them reserved
//! (`Application::reserved_shortcuts`: ⌘T ⌘W ⌘L ⌘K ⌘1 ⌘/ ⌘, ⌃Tab), the
//! Edit menu's ⌘C ⌘A AppKit's own, and ⌘U a menu item not reserved. The page
//! is a focused field reporting each key it hears and whether it took it.
//!
//! An off-screen window is never the key window, so AppKit would offer its
//! keys to the main menu alone. Each key is sent as AppKit sends a key
//! window's: the window's views (`performKeyEquivalent:`), then the main
//! menu, then the first responder (`sendEvent:`). A key the page leaves
//! alone WebKit re-sends through `-[NSApplication sendEvent:]`, which reaches
//! the main menu without a key window (but not zgx's own view).
//!
//! Prints `ACTION <name>` for each menu action that fired, `PAGE <key>` for
//! what the page heard, `FIRST <class>` for the first responder and
//! `WEB_KEYS_DONE`. `scripts/macos_web_view_keys_probe.py` runs it and
//! checks the lines.
use zgx::{
    compose::prelude::*,
    input::{EventPhase, InputEvent, Key, Modifiers},
    web_view::{WebView, WebViewEvent, WebViewOptions, web_view},
};
use zgx_desktop::{
    AppMenu, AppMenuEntry, Application, MenuAccelerator, MenuAction, StandardItem, WindowOptions,
};

const PAGE: &str = r#"<!doctype html><body style="background:#223"><input id=f autofocus value="some text">
<script>
const post = (text) => window.webkit.messageHandlers.zgx.postMessage(text);
const field = document.getElementById('f');
document.addEventListener('keydown', (e) => {
  if (e.key === 'Meta' || e.key === 'Control') return;
  post((e.ctrlKey ? 'ctrl+' : '') + (e.metaKey ? 'meta+' : '') + e.key);
}, true);
</script>"#;

/// The reserved keys: name, character, key code, Control instead of ⌘.
const RESERVED: &[(&str, &str, u16, bool)] = &[
    ("new-tab", "t", 17, false),
    ("close-tab", "w", 13, false),
    ("address", "l", 37, false),
    ("palette", "k", 40, false),
    ("tab-1", "1", 18, false),
    ("model", "/", 44, false),
    ("settings", ",", 43, false),
    ("next-tab", "\t", 48, true),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let accelerator = |key: &str, control: bool| {
        let key = if key == "\t" {
            Key::Tab
        } else {
            Key::Character(key.into())
        };
        MenuAccelerator::new(
            key,
            Modifiers {
                control,
                meta: !control,
                ..Default::default()
            },
        )
    };
    let mut items: Vec<AppMenuEntry> = RESERVED
        .iter()
        .map(|(name, key, _, control)| {
            AppMenuEntry::item(*name, *name).accelerator(accelerator(key, *control))
        })
        .collect();
    items.push(AppMenuEntry::item("Unreserved", "unreserved").accelerator(accelerator("u", false)));
    Application::new()
        .menus(vec![
            AppMenu::new("Probe", items),
            AppMenu::new(
                "Edit",
                [
                    AppMenuEntry::Standard(StandardItem::Copy),
                    AppMenuEntry::Standard(StandardItem::SelectAll),
                ],
            ),
        ])?
        .reserved_shortcuts(
            RESERVED
                .iter()
                .map(|(_, key, _, control)| accelerator(key, *control))
                .collect(),
        )
        .window(WindowOptions {
            title: "Web view keys".into(),
            width: 600.,
            height: 300.,
            ..Default::default()
        })
        .run(|cx| {
            let web = WebView::new(&cx.ui.runtime, WebViewOptions::default());
            web.load_html(PAGE, None);
            let page = web.on_event(|event| {
                if let WebViewEvent::Message { body, .. } = event {
                    println!("PAGE {body}");
                }
            });
            let _view = cx.render(
                column()
                    .w_full()
                    .h_full()
                    .p(8.)
                    .child(web_view(&web).w_full().grow()),
            );
            let root = cx.ui.root();
            cx.ui.on_event(root, false, move |event| {
                if event.phase != EventPhase::Capture
                    && let InputEvent::Action(action) = &event.event
                    && let Some(action) = action.downcast_ref::<MenuAction>()
                {
                    println!("ACTION {}", action.0);
                    event.prevent_default();
                }
            });
            let window = cx.window.clone();
            cx.tasks.spawn(async move {
                let _keep = page;
                let pause = |ms| zgx::timer::sleep(std::time::Duration::from_millis(ms));
                pause(2500).await;
                web.focus();
                pause(800).await;
                keys::first_responder();
                for (name, key, code, control) in RESERVED {
                    println!("PRESS {name}");
                    keys::press(key, *code, *control);
                    pause(300).await;
                }
                for (name, key, code) in [
                    ("unreserved", "u", 32),
                    ("copy", "c", 8),
                    ("select-all", "a", 0),
                ] {
                    println!("PRESS {name}");
                    keys::press(key, code, false);
                    pause(400).await;
                }
                println!("WEB_KEYS_DONE");
                window.close();
            });
        })
}

#[cfg(target_os = "macos")]
mod keys {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSWindow};
    use objc2_foundation::{NSPoint, NSProcessInfo, NSString};

    fn window() -> Option<objc2::rc::Retained<NSWindow>> {
        let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        app.keyWindow().or_else(|| app.windows().firstObject())
    }
    /// `characters` (key code `code`) with ⌘ (or Control), down and up, as
    /// AppKit sends a key window's keys.
    pub fn press(characters: &str, code: u16, control: bool) {
        let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        let Some(window) = window() else {
            println!("NO_WINDOW");
            return;
        };
        let flags = if control {
            NSEventModifierFlags::Control
        } else {
            NSEventModifierFlags::Command
        };
        for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                kind,
                NSPoint::new(0., 0.),
                flags,
                NSProcessInfo::processInfo().systemUptime(),
                window.windowNumber(),
                None,
                &NSString::from_str(characters),
                &NSString::from_str(characters),
                false,
                code,
            )
            .unwrap();
            // SAFETY: AppKit's current event, as its own dispatch sets it
            // (WebKit reads it); the event outlives the calls.
            let _: () = unsafe { objc2::msg_send![&*app, _setCurrentEvent: &*event] };
            let taken = kind == NSEventType::KeyDown
                && (window.performKeyEquivalent(&event)
                    || app
                        .mainMenu()
                        .is_some_and(|menu| menu.performKeyEquivalent(&event)));
            if !taken {
                window.sendEvent(&event);
            }
        }
    }
    /// The window's first responder.
    pub fn first_responder() {
        let class = window()
            .and_then(|window| window.firstResponder())
            .map(|responder| responder.class().name().to_string_lossy().into_owned());
        println!("FIRST {class:?}");
    }
}
#[cfg(not(target_os = "macos"))]
mod keys {
    pub fn press(_: &str, _: u16, _: bool) {}
    pub fn first_responder() {}
}
