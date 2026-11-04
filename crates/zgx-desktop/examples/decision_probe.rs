//! A decision control (`ui.decision`) in a real window, against a plugin that
//! tries to cover it: a modal, and popovers past the window's edge. At each
//! step it reports what the pointer finds at the control's button, the popup
//! windows in the scene and those the system shows (macOS; on X11,
//! `scripts/decision_probe_smoke.py` counts them), and exits non-zero if any
//! is wrong. The plugin's popups show in windows of their own until a decision
//! is up, and in this one from then on; the app's own keep theirs. The plugin
//! opens them from its own buttons, once the window has settled.
//!
//! `--sealed`: the same against a sealed page (`ui.sealed`): a plugin's modal
//! over it, then one the plugin opens in its own page there, which stays in
//! that page's box.
//!
//! Clicks go through the input routing, this process alone (the user's pointer
//! is never moved), unless `DECISION_PROBE_EXTERNAL` is set: then it prints
//! `CLICK x y` and waits for a click from outside. `DECISION_PROBE_DWELL` is
//! how long, in ms, it holds each step.
use std::{path::PathBuf, time::Duration};
use zgx::{
    compose::ViewHandle,
    input::{InputEvent, PointerButton},
    widgets::{Ui, WeakUi},
};
use zgx_desktop::{Application, WindowOptions};
use zgx_script::{Plugin, ScriptHost, Value};

const APP: &str = r##"
local up = ui.state("up", false)
local app_open = ui.state("app_open", false)
local clicked = ui.shared("clicked", 0)
ui.command { id = "up", run = function() up:set(true) end }
ui.command { id = "app-popup", run = function() app_open:set(true) end }
ui.command { id = "app-popup-close", run = function() app_open:set(false) end }
return ui.column { w_full = true, h_full = true, p = 12,
    ui.popover { open = app_open, label = "App's",
        anchor = ui.button { id = "app-trigger", w = 60, h = 24, "App" },
        ui.div { id = "app-panel", w = 200, h = 900 } },
    ui.text { id = "after", "After" },
    ui.show(up, ui.decision { key = "approve",
        ui.button { id = "allow", w = 120, h = 40, "Allow",
            on_click = function() clicked:set(clicked:peek() + 1) end },
    }),
}
"##;

const PLUGIN: &str = r##"
local free = ui.state("free", false)
local modal = ui.state("modal", false)
local nested = ui.state("nested", true)
ui.contribute { target = "#after", after = ui.popover { open = free, label = "Plugin's",
    anchor = ui.button { id = "plugin-trigger", w = 60, h = 24, "Plugin",
        on_click = function() free:set(true) end },
    -- In its panel, which covers the window once it can't have a window of its own.
    ui.div { id = "plugin-panel", w = 300, h = 900,
        ui.button { id = "open-modal", w = 60, h = 24, "Modal",
            on_click = function() modal:set(true) end } } } }
ui.contribute { target = "#after", after = ui.modal { open = modal, z = 2147483647,
    ui.div { id = "plugin-modal", w = 2000, h = 2000, z = 2147483647, on_click = function() end,
        ui.popover { open = nested, label = "Plugin's nested",
            anchor = ui.button { id = "nested-trigger", w = 60, h = 24, "Nested" },
            ui.div { id = "nested-panel", w = 300, h = 900 } } } } }
"##;

/// `--sealed`: a Settings page, and in it a page of the plugin's.
const SEALED_APP: &str = r##"
local clicked = ui.shared("clicked", 0)
return ui.column { w_full = true, h_full = true, p = 12,
    ui.text { id = "after", "After" },
    ui.sealed { key = "settings", w = 300, h = 200,
        ui.column { w_full = true, h_full = true,
            ui.button { id = "app-setting", w = 300, h = 40, "Setting",
                on_click = function() clicked:set(clicked:peek() + 1) end },
            ui.plugin_page { plugin = "plugin", w = 300, h = 100 },
            ui.button { id = "app-below", w = 300, h = 40, "Below",
                on_click = function() clicked:set(clicked:peek() + 1) end },
        },
    },
}
"##;

const SEALED_PLUGIN: &str = r##"
local over = ui.state("over", false)
local in_page = ui.state("in_page", false)
local nested = ui.state("nested", true)
ui.contribute { target = "#after", after = ui.button { id = "open-over", w = 60, h = 24, "Over",
    on_click = function() over:set(true) end } }
ui.contribute { target = "#after", after = ui.modal { open = over, z = 2147483647,
    ui.div { id = "plugin-modal", w = 2000, h = 2000, z = 2147483647, on_click = function() end,
        ui.popover { open = nested, label = "Plugin's nested",
            anchor = ui.button { id = "nested-trigger", w = 60, h = 24, "Nested" },
            ui.div { id = "nested-panel", w = 300, h = 900 } } } } }
ui.contribute("plugin_page", ui.column {
    ui.button { id = "page-button", w = 100, h = 24, "Page",
        on_click = function() in_page:set(true) end },
    ui.modal { open = in_page, z = 2147483647,
        ui.div { id = "page-modal", w = 3000, h = 3000, on_click = function() end } },
})
"##;

fn plugin_entry(code: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("zgx-decision-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a directory for the plugin");
    let entry = dir.join("init.luau");
    std::fs::write(&entry, code).expect("the plugin's script");
    entry
}

/// The system's windows of this process on screen, past the main one.
#[cfg(target_os = "macos")]
fn native_popups() -> i64 {
    use std::ffi::c_void;
    type Ref = *const c_void;
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> Ref;
        static kCGWindowOwnerPID: Ref;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: Ref) -> isize;
        fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
        fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
        fn CFNumberGetValue(number: Ref, kind: isize, out: *mut c_void) -> bool;
        fn CFRelease(object: Ref);
    }
    const ON_SCREEN_ONLY: u32 = 1;
    const SINT32: isize = 3;
    let me = std::process::id() as i32;
    // SAFETY: the list is owned here and released once; each entry and value
    // is borrowed from it while it lives, and the pid is read into an i32.
    unsafe {
        let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY, 0);
        let mut mine = 0;
        for at in 0..CFArrayGetCount(list) {
            let window = CFArrayGetValueAtIndex(list, at);
            let pid = CFDictionaryGetValue(window, kCGWindowOwnerPID);
            let mut owner = 0i32;
            CFNumberGetValue(pid, SINT32, (&raw mut owner).cast());
            mine += i64::from(owner == me);
        }
        CFRelease(list);
        mine - 1
    }
}
/// Counted from outside: none of the system's asked here.
#[cfg(not(target_os = "macos"))]
fn native_popups() -> i64 {
    -1
}

struct Probe {
    ui: WeakUi,
    view: ViewHandle,
    clicked: zgx_script::Shared,
    dwell: Duration,
    external: bool,
    failed: std::cell::Cell<bool>,
}
impl Probe {
    fn check(&self, what: &str, ok: bool) {
        if !ok {
            self.failed.set(true);
            println!("FAILED {what}");
        }
    }
    fn ui(&self) -> Ui {
        self.ui.upgrade().expect("the window is open")
    }
    /// The button's middle, in the window.
    fn at(&self, id: &str) -> Option<(f32, f32)> {
        let node = self.view.find(id)?;
        let b = self.ui().scene.borrow().visible_bounds(node)?;
        Some((b.x + b.width / 2., b.y + b.height / 2.))
    }
    /// What the pointer finds at `id`'s middle is `id`.
    fn finds(&self, id: &str) -> bool {
        let (Some(node), Some((x, y))) = (self.view.find(id), self.at(id)) else {
            return false;
        };
        let ui = self.ui();
        let scene = ui.scene.borrow();
        ui.input.pointer_target(&scene, x, y) == Some(node)
    }
    fn report(&self, step: &str, target: &str) -> (usize, i64) {
        let scene = self.ui().scene.borrow().popups().len();
        let native = native_popups();
        println!(
            "STEP {step} scene_popups={scene} native_popups={native} hit_{target}={} clicked={}",
            self.finds(target),
            self.clicks(),
        );
        (scene, native)
    }
    fn clicks(&self) -> f64 {
        self.clicked.with_untracked(|value| match value {
            Value::Num(n) => *n,
            _ => 0.,
        })
    }
    /// Click `id` from inside: what the plugin does on its own.
    fn poke(&self, id: &str) {
        let Some((x, y)) = self.at(id) else {
            return self.check(id, false);
        };
        let button = PointerButton::Primary;
        let mut ui = self.ui();
        ui.dispatch(InputEvent::PointerMove { x, y });
        ui.dispatch(InputEvent::PointerDown { x, y, button });
        ui.dispatch(InputEvent::PointerUp { x, y, button });
    }
    async fn click(&self, id: &str, wanted: f64) {
        let Some((x, y)) = self.at(id) else {
            return self.check(id, false);
        };
        if self.external {
            println!("CLICK {x} {y}");
        } else {
            self.poke(id);
        }
        zgx::timer::sleep(self.dwell).await;
        self.check("the click reached the control", self.clicks() == wanted);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: "Decision probe".into(),
            width: 480.,
            height: 320.,
            ..Default::default()
        })
        .run(|cx| {
            let sealed = std::env::args().any(|arg| arg == "--sealed");
            let (app, plugin) = match sealed {
                true => (SEALED_APP, SEALED_PLUGIN),
                false => (APP, PLUGIN),
            };
            let host = ScriptHost::inline(&cx.ui.runtime, "app", app);
            host.set_plugins(vec![Plugin {
                name: "plugin".into(),
                entry: plugin_entry(plugin),
                grants: Vec::new(),
            }]);
            let clicked = host.shared("clicked", Value::Num(0.));
            let _ = host.reload();
            let view = cx.render(host.view());
            let probe = Probe {
                ui: cx.ui.downgrade(),
                view,
                clicked,
                dwell: Duration::from_millis(
                    std::env::var("DECISION_PROBE_DWELL")
                        .ok()
                        .and_then(|ms| ms.parse().ok())
                        .unwrap_or(800),
                ),
                external: std::env::var_os("DECISION_PROBE_EXTERNAL").is_some(),
                failed: Default::default(),
            };
            let macos = cfg!(target_os = "macos");
            cx.tasks.spawn(async move {
                let sleep = zgx::timer::sleep;
                let wait = probe.dwell;
                sleep(wait).await;
                println!("READY");
                if sealed {
                    sleep(wait).await;
                    let (scene, native) = probe.report("sealed", "app-setting");
                    sleep(wait).await;
                    probe.check("the pointer finds the setting", probe.finds("app-setting"));
                    probe.check("no popup in a window", scene == 0);
                    probe.check("none on the system's list", !macos || native == 0);
                    probe.poke("open-over");
                    sleep(wait).await;
                    probe.check("the modal shows", probe.view.find("plugin-modal").is_some());
                    probe.check(
                        "its popover shows",
                        probe.view.find("nested-panel").is_some(),
                    );
                    let (scene, native) = probe.report("plugin-modal", "app-setting");
                    sleep(wait).await;
                    probe.check("no popup in a window", scene == 0);
                    probe.check("none on the system's list", !macos || native == 0);
                    probe.check("the pointer finds the setting", probe.finds("app-setting"));
                    probe.click("app-setting", 1.).await;
                    probe.check("the plugin's page shows", probe.finds("page-button"));
                    probe.poke("page-button");
                    sleep(wait).await;
                    probe.check("its modal shows", probe.view.find("page-modal").is_some());
                    let (scene, native) = probe.report("page-modal", "app-setting");
                    sleep(wait).await;
                    probe.check("no popup in a window", scene == 0);
                    probe.check("none on the system's list", !macos || native == 0);
                    probe.check(
                        "the page's modal stays in its box",
                        probe.finds("app-below"),
                    );
                    probe.click("app-below", 2.).await;
                    let failed = probe.failed.get();
                    println!("RESULT {}", if failed { "fail" } else { "pass" });
                    std::process::exit(i32::from(failed));
                }
                sleep(wait).await;
                probe.poke("plugin-trigger");
                sleep(wait).await;
                // Without a decision up, a plugin's popup is a window: what
                // follows shows it no longer is, not that there never was one.
                let (scene, native) = probe.report("free", "plugin-trigger");
                sleep(wait).await;
                probe.check("a plugin's popup is a window before", scene >= 1);
                probe.check("the system shows it", !macos || native >= 1);

                host.run_command("up");
                sleep(wait).await;
                let (scene, native) = probe.report("decision", "allow");
                sleep(wait).await;
                probe.check("no plugin popup in a window", scene == 0);
                probe.check("none on the system's list", !macos || native == 0);
                probe.check("the pointer finds the control", probe.finds("allow"));
                probe.click("allow", 1.).await;

                host.run_command("app-popup");
                sleep(wait).await;
                let (scene, native) = probe.report("app-popup", "allow");
                sleep(wait).await;
                probe.check("the app's popup keeps its window", scene == 1);
                probe.check("the system shows it", !macos || native == 1);

                // While the app's popup holds focus nothing outside it is
                // live to the plugin's own button: as it closes, it is.
                host.run_command("app-popup-close");
                sleep(wait).await;
                probe.poke("open-modal");
                sleep(wait).await;
                probe.check("the modal shows", probe.view.find("plugin-modal").is_some());
                probe.check(
                    "its popover shows",
                    probe.view.find("nested-panel").is_some(),
                );
                let (scene, native) = probe.report("plugin-modal", "allow");
                sleep(wait).await;
                probe.check("no plugin popup in a window", scene == 0);
                probe.check("none on the system's list", !macos || native == 0);
                probe.check("the pointer finds the control", probe.finds("allow"));
                probe.click("allow", 2.).await;

                let failed = probe.failed.get();
                println!("RESULT {}", if failed { "fail" } else { "pass" });
                std::process::exit(i32::from(failed));
            });
        })
}
