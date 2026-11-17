//! What `ScriptHost::guard` is asked before the app's own scripts leave the
//! interface: a process (`ui.term`), a page (`ui.web`, `ui.webview`), the
//! clipboard, dialogs, files, folders and notifications. A preview of
//! someone's template runs under it: refused, nothing happens, and it says so.
mod common;

use common::Harness;
use std::{cell::RefCell, rc::Rc};
use zgx::{
    native_view::NativeContent,
    terminal::Terminal,
    web_view::{NavigationKind, NavigationRequest, WebView},
};
use zgx_script::{Capability, GuardKind, Plugin, ScriptHost};

type Asked = Rc<RefCell<Vec<(GuardKind, String)>>>;

/// `code` under a guard that lets `allow` by, and what it was asked.
fn guarded(code: &'static str, allow: fn(GuardKind, &str) -> bool) -> (Harness, Asked) {
    let asked: Asked = Rc::default();
    let told = asked.clone();
    let h = Harness::with_desktop(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", code);
        host.guard(move |kind, name| {
            told.borrow_mut().push((kind, name.to_owned()));
            match allow(kind, name) {
                true => Ok(()),
                false => Err("not in a preview".into()),
            }
        });
        host
    });
    (h, asked)
}
fn none(_: GuardKind, _: &str) -> bool {
    false
}
fn all(_: GuardKind, _: &str) -> bool {
    true
}
fn was_asked(asked: &Asked, kind: GuardKind, name: &str) -> bool {
    asked.borrow().iter().any(|(k, n)| *k == kind && n == name)
}
fn terminal_of(h: &mut Harness) -> Option<Terminal> {
    h.ui.prepare_frame();
    h.natives
        .nodes()
        .into_iter()
        .find_map(|node| match h.natives.content(node) {
            Some(NativeContent::Terminal(term)) => Some(term),
            _ => None,
        })
}
fn web_of(h: &mut Harness) -> WebView {
    h.ui.prepare_frame();
    h.natives
        .nodes()
        .into_iter()
        .find_map(|node| match h.natives.content(node) {
            Some(NativeContent::Web(web)) => Some(web),
            _ => None,
        })
        .expect("a web view mounted")
}
fn navigation(url: &str, kind: NavigationKind) -> NavigationRequest {
    NavigationRequest {
        url: url.into(),
        main_frame: true,
        kind,
        source_url: "https://allowed.test/".into(),
    }
}

// The terminal: nothing spawns, whatever the script asks to run.

const TERM: &str = r##"
local term = ui.term("t", { command = "/usr/bin/touch /tmp/pwned" })
return ui.column { ui.terminal { term = term, w = 200, h = 100 } }
"##;

/// `code`'s first load under a guard refusing everything: its error, and
/// what the guard was asked.
fn refused_load(code: &'static str) -> (String, Asked) {
    let asked: Asked = Rc::default();
    let told = asked.clone();
    let loaded = zgx_script::testing::TestApp::try_with_desktop(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", code);
        host.guard(move |kind, name| {
            told.borrow_mut().push((kind, name.to_owned()));
            Err("not in a preview".into())
        });
        host
    });
    (loaded.err().expect("the load is refused"), asked)
}

/// Raised where the script asked, as any error: no panic, and the last load
/// (none here) stays as it was.
#[test]
fn a_refused_terminal_never_starts_and_the_app_is_told() {
    let (error, asked) = refused_load(TERM);
    assert!(was_asked(
        &asked,
        GuardKind::Term,
        "/usr/bin/touch /tmp/pwned"
    ));
    assert!(error.contains("not in a preview"), "{error}");
    assert!(error.contains("/usr/bin/touch /tmp/pwned"), "{error}");
}

/// Refused as it's reached in a handler, the script hears it and goes on,
/// and no terminal is mounted.
#[test]
fn a_terminal_refused_in_a_handler_leaves_the_app_running() {
    let (mut h, asked) = guarded(
        r##"local said = ui.state("said", "")
        return ui.column {
            ui.button { id = "go", w = 10, h = 10, on_click = function()
                local ok, why = pcall(function() return ui.term("t", { command = "/usr/bin/touch x" }) end)
                said:set(if ok then "started" else tostring(why))
            end },
            ui.text { id = "said", "{said}" },
        }"##,
        none,
    );
    h.click("go");
    h.settle();
    assert!(
        h.text("said").contains("not in a preview"),
        "{}",
        h.text("said")
    );
    assert!(was_asked(&asked, GuardKind::Term, "/usr/bin/touch x"));
    assert!(terminal_of(&mut h).is_none(), "nothing to start");
}

#[test]
fn an_allowed_terminal_starts_as_it_asked() {
    let (mut h, asked) = guarded(TERM, all);
    assert_eq!(h.error(), None);
    assert!(was_asked(
        &asked,
        GuardKind::Term,
        "/usr/bin/touch /tmp/pwned"
    ));
    let term = terminal_of(&mut h).expect("a terminal");
    assert_eq!(
        term.options().command.as_deref(),
        Some("/usr/bin/touch /tmp/pwned")
    );
}

#[test]
fn a_terminal_with_no_command_asks_for_the_shell_it_would_run() {
    let (error, asked) =
        refused_load(r##"return ui.terminal { term = ui.term("t"), w = 200, h = 100 }"##);
    assert!(error.contains("not in a preview"), "{error}");
    let asked = asked.borrow();
    let (kind, name) = asked.first().expect("asked");
    assert_eq!(*kind, GuardKind::Term);
    assert!(!name.is_empty());
}

/// A session already running, reached by its id alone, is asked for by what
/// it runs: a preview gets no handle to type into.
#[test]
fn reaching_a_session_by_its_id_asks_what_it_runs() {
    let (h, asked) = guarded(
        r##"local first = ui.term("t", { command = "app-shell" })
        local said = ui.state("said", "")
        return ui.column {
            ui.terminal { term = first, w = 200, h = 100 },
            ui.button { id = "reach", w = 10, h = 10, on_click = function()
                local ok = pcall(function() return ui.term("t") end)
                said:set(if ok then "reached" else "refused")
            end },
            ui.text { id = "said", "{said}" },
        }"##,
        |_, name| name == "app-shell",
    );
    assert_eq!(h.error(), None);
    let mut h = h;
    let before = asked.borrow().len();
    h.click("reach");
    h.settle();
    assert_eq!(h.text("said"), "reached");
    assert!(asked.borrow().len() > before);
    assert!(was_asked(&asked, GuardKind::Term, "app-shell"));
}

#[test]
fn what_a_script_types_into_a_terminal_is_asked_for() {
    let (mut h, asked) = guarded(
        r##"local term = ui.term("t", { command = "zsh" })
        local said = ui.state("said", "")
        local function try(name, f)
            local ok = pcall(f)
            said:set(said:peek() .. name .. (if ok then "=ok " else "=no "))
        end
        return ui.column {
            ui.terminal { term = term, w = 200, h = 100 },
            ui.button { id = "go", w = 10, h = 10, on_click = function()
                try("input", function() term:input("rm -rf ~\n") end)
                try("paste", function() term:paste("rm -rf ~\n") end)
                try("binding", function() term:binding("paste_from_clipboard") end)
            end },
            ui.text { id = "said", "{said}" },
        }"##,
        |kind, _| kind == GuardKind::Term,
    );
    h.click("go");
    h.settle();
    assert_eq!(h.text("said"), "input=no paste=no binding=no ");
    assert!(was_asked(&asked, GuardKind::TermInput, "t"));
}

// Web views: what the script loads, and each navigation the platform starts.

const LOADS: &str = r##"
local w = ui.web()
local said = ui.state("said", "")
local function try(name, f)
    local ok = pcall(f)
    said:set(said:peek() .. name .. (if ok then "=ok " else "=no "))
end
return ui.column {
    ui.webview { web = w, id = "page", w = 200, h = 100 },
    ui.button { id = "load", w = 10, h = 10, on_click = function()
        try("load", function() w:load("https://evil.test/") end)
    end },
    ui.button { id = "html", w = 10, h = 10, on_click = function()
        try("html", function() w:html("<script>fetch('https://evil.test')</script>", "https://evil.test/") end)
    end },
    ui.text { id = "said", "{said}" },
}"##;

#[test]
fn a_page_a_script_loads_is_asked_for_and_refused_goes_nowhere() {
    let (mut h, asked) = guarded(LOADS, none);
    let web = web_of(&mut h);
    h.click("load");
    h.settle();
    h.click("html");
    h.settle();
    assert_eq!(h.text("said"), "load=no html=no ");
    assert_ne!(web.url(), "https://evil.test/");
    assert!(was_asked(&asked, GuardKind::Web, "https://evil.test/"));
    // Allowed, it goes.
    let (mut h, _) = guarded(LOADS, all);
    let web = web_of(&mut h);
    h.click("load");
    h.settle();
    assert_eq!(h.text("said"), "load=ok ");
    assert_eq!(web.url(), "https://evil.test/");
}

#[test]
fn a_src_is_asked_for_each_time_and_refused_stays_where_it_was() {
    let (mut h, asked) = guarded(
        r##"local src = ui.state("src", "https://allowed.test/")
        return ui.column {
            ui.webview { src = src, id = "page", w = 200, h = 100 },
            ui.button { id = "go", w = 10, h = 10, on_click = ui.set(src, "https://evil.test/") },
        }"##,
        |_, name| name.starts_with("https://allowed.test"),
    );
    let web = web_of(&mut h);
    h.settle();
    assert_eq!(h.error(), None);
    assert_eq!(web.url(), "https://allowed.test/");
    h.click("go");
    h.settle();
    assert!(was_asked(&asked, GuardKind::Web, "https://evil.test/"));
    assert_eq!(web.url(), "https://allowed.test/", "it stays");
    let error = h.error().unwrap_or_default();
    assert!(error.contains("not in a preview"), "{error}");
}

#[test]
fn a_first_src_refused_loads_nothing() {
    let (mut h, _) = guarded(
        r##"return ui.webview { src = "https://evil.test/", id = "page", w = 200, h = 100 }"##,
        none,
    );
    let web = web_of(&mut h);
    h.settle();
    assert_ne!(web.url(), "https://evil.test/");
    assert!(h.error().unwrap_or_default().contains("not in a preview"));
}

#[test]
fn html_as_a_prop_is_asked_for_by_its_base() {
    let (mut h, asked) = guarded(
        r##"return ui.webview { html = "<img src=https://evil.test>", base_url = "https://evil.test/", id = "page", w = 200, h = 100 }"##,
        none,
    );
    let web = web_of(&mut h);
    h.settle();
    assert!(was_asked(&asked, GuardKind::Web, "https://evil.test/"));
    assert!(h.error().unwrap_or_default().contains("not in a preview"));
    assert_ne!(web.url(), "https://evil.test/");
}

/// What a page does itself reaches the platform's question: a link, a form,
/// a redirect, a script's `location`, history.
#[test]
fn each_navigation_the_platform_starts_is_asked_and_refused_is_cancelled() {
    let (mut h, asked) = guarded(LOADS, |_, name| name.starts_with("https://allowed.test"));
    let web = web_of(&mut h);
    for kind in [
        NavigationKind::Link,
        NavigationKind::FormSubmission,
        NavigationKind::BackForward,
        NavigationKind::Reload,
        NavigationKind::Other,
    ] {
        assert!(
            !web.allows(&navigation("https://evil.test/", kind)),
            "{kind:?}"
        );
        assert!(
            web.allows(&navigation("https://allowed.test/next", kind)),
            "{kind:?}"
        );
    }
    assert!(was_asked(&asked, GuardKind::Web, "https://evil.test/"));
    let error = h.error().unwrap_or_default();
    assert!(error.contains("not in a preview"), "{error}");
    // An empty page reaches nothing.
    assert!(web.allows(&navigation("about:blank", NavigationKind::Other)));
    let _ = &mut h;
}

#[test]
fn a_guard_set_after_a_web_view_was_made_asks_of_it_too() {
    let mut h = Harness::with_desktop(|runtime| ScriptHost::inline(runtime, "app", LOADS));
    let web = web_of(&mut h);
    assert!(web.allows(&navigation("https://evil.test/", NavigationKind::Link)));
    h.host.guard(|_, _| Err("not in a preview".into()));
    assert!(!web.allows(&navigation("https://evil.test/", NavigationKind::Link)));
}

/// A plugin's web view goes by its grants: the guard is the app VM's.
#[test]
fn a_plugins_web_view_is_not_asked_of() {
    let dir = common::temp_dir("guard-web");
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(
        &entry,
        r#"ui.contribute("bar", ui.webview { src = "https://plugin.test/", id = "page", w = 100, h = 50 })"#,
    )
    .unwrap();
    let asked: Asked = Rc::default();
    let told = asked.clone();
    let mut h = Harness::with_desktop(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", r#"return ui.slot("bar")"#);
        host.guard(move |kind, name| {
            told.borrow_mut().push((kind, name.to_owned()));
            Err("no".into())
        });
        host.set_plugins(vec![Plugin {
            name: "browser".into(),
            entry,
            grants: vec![Capability::Web],
        }]);
        host
    });
    let web = web_of(&mut h);
    h.settle();
    assert!(asked.borrow().is_empty(), "{:?}", asked.borrow());
    assert!(web.allows(&navigation("https://plugin.test/x", NavigationKind::Link)));
    let _ = std::fs::remove_dir_all(&dir);
}

// The rest of what leaves the interface.

const DOORS: &str = r##"
local said = ui.state("said", "")
local function try(name, f)
    local ok = pcall(f)
    said:set(said:peek() .. name .. (if ok then "=ok " else "=no "))
end
return ui.column {
    ui.button { id = "go", w = 10, h = 10, on_click = function()
        try("read", function() return ui.clipboard.read() end)
        try("write", function() return ui.clipboard.write("x") end)
        try("read_image", function() return ui.clipboard.read_image() end)
        try("notify", function() return ui.notify { title = "Hello" } end)
        try("badge", function() return ui.badge(3) end)
        try("open_files", function() return ui.open_files() end)
        try("save_file", function() return ui.save_file() end)
        try("open_folder", function() return ui.open_folder() end)
        try("remembered", function() return ui.remembered_folder("library") end)
    end },
    ui.text { id = "said", "{said}" },
}"##;

#[test]
fn the_clipboard_dialogs_folders_and_notifications_are_asked_for() {
    let (mut h, asked) = guarded(DOORS, none);
    let desktop = h.desktop.clone().unwrap();
    desktop.copy("secret");
    h.click("go");
    h.settle();
    assert_eq!(
        h.text("said"),
        "read=no write=no read_image=no notify=no badge=no open_files=no \
         save_file=no open_folder=no remembered=no "
    );
    assert_eq!(desktop.clipboard(), "secret", "unchanged");
    assert!(desktop.asked().is_empty(), "no dialog");
    assert!(desktop.notifications().is_empty());
    assert_eq!(desktop.badge(), None);
    for (kind, name) in [
        (GuardKind::Clipboard, "read"),
        (GuardKind::Clipboard, "write"),
        (GuardKind::Clipboard, "read_image"),
        (GuardKind::Notification, "Hello"),
        (GuardKind::Notification, "badge"),
        (GuardKind::Dialog, "open_files"),
        (GuardKind::Dialog, "save_file"),
        (GuardKind::Dialog, "open_folder"),
        (GuardKind::Folder, "library"),
    ] {
        assert!(was_asked(&asked, kind, name), "{kind:?} {name}");
    }
    // Allowed, they all go.
    let (mut h, _) = guarded(DOORS, all);
    h.click("go");
    h.settle();
    assert_eq!(
        h.text("said"),
        "read=ok write=ok read_image=ok notify=ok badge=ok open_files=ok \
         save_file=ok open_folder=ok remembered=ok "
    );
}

/// With no guard, every door is as it was.
#[test]
fn with_no_guard_nothing_is_asked() {
    let mut h = Harness::with_desktop(|runtime| ScriptHost::inline(runtime, "app", DOORS));
    h.click("go");
    h.settle();
    assert!(!h.text("said").contains("=no"), "{}", h.text("said"));
}

// A link's click, a window, a kept value and a download's own dialog.

const LINK: &str = r##"return ui.rich {
    id = "para", w = 120,
    ui.span { "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", href = "https://evil.test/" },
}"##;

#[test]
fn a_link_clicked_is_asked_for_and_refused_stays_shut() {
    let (mut h, asked) = guarded(LINK, none);
    let desktop = h.desktop.clone().unwrap();
    h.ui.prepare_frame();
    h.click("para");
    h.settle();
    assert!(desktop.opened().is_empty(), "{:?}", desktop.opened());
    assert!(was_asked(&asked, GuardKind::OpenLink, "https://evil.test/"));
    assert!(h.error().unwrap_or_default().contains("not in a preview"));
    // Allowed, and with no guard, it opens.
    let (mut h, _) = guarded(LINK, all);
    let desktop = h.desktop.clone().unwrap();
    h.ui.prepare_frame();
    h.click("para");
    h.settle();
    assert_eq!(desktop.opened(), ["https://evil.test/"]);
    let mut h = Harness::with_desktop(|runtime| ScriptHost::inline(runtime, "app", LINK));
    let desktop = h.desktop.clone().unwrap();
    h.ui.prepare_frame();
    h.click("para");
    h.settle();
    assert_eq!(desktop.opened(), ["https://evil.test/"]);
}

#[test]
fn a_links_open_item_is_asked_for_and_refused_stays_shut() {
    let (mut h, asked) = guarded(LINK, none);
    let desktop = h.desktop.clone().unwrap();
    h.ui.prepare_frame();
    let bounds = h.bounds("para");
    zgx_script::driver::right_click_at(
        &mut h.ui,
        (bounds.x + bounds.width / 2., bounds.y + bounds.height / 2.),
    );
    h.settle();
    h.ui.prepare_frame();
    h.click("link-open");
    h.settle();
    assert!(desktop.opened().is_empty(), "{:?}", desktop.opened());
    assert!(was_asked(&asked, GuardKind::OpenLink, "https://evil.test/"));
}

#[test]
fn a_window_a_script_declares_is_asked_for() {
    let code = r##"local open = ui.state("open", true)
    ui.window { id = "fake-login", open = open, title = "Sign in", ui.text { "password?" } }
    return ui.text { "app" }"##;
    let (error, asked) = refused_load(code);
    assert!(was_asked(&asked, GuardKind::Window, "fake-login"));
    assert!(error.contains("not in a preview"), "{error}");
    let (h, _) = guarded(code, all);
    assert_eq!(h.error(), None);
}

const PERSIST: &str = r##"local kept = ui.persist("token", "initial")
return ui.column {
    ui.text { id = "kept", function() return kept:get() end },
    ui.button { id = "set", w = 10, h = 10, on_click = ui.set(kept, "overwritten") },
}"##;

/// The app's saved `token`, in a storage of its own, and a host of `PERSIST`
/// on it under a guard refusing everything.
struct Kept {
    dir: std::path::PathBuf,
    file: std::path::PathBuf,
}
impl Kept {
    fn new(name: &str) -> Self {
        let dir = common::temp_dir(&format!("guard-persist-{name}"));
        std::fs::create_dir_all(dir.join("app")).unwrap();
        let file = dir.join("app").join("token.json");
        std::fs::write(&file, "\"the app's own\"").unwrap();
        Self { dir, file }
    }
    fn on_disk(&self) -> String {
        std::fs::read_to_string(&self.file).unwrap()
    }
    fn refusing(&self) -> Harness {
        let store = self.dir.clone();
        Harness::new(move |runtime| {
            let host = ScriptHost::inline(runtime, "app", PERSIST).storage(store);
            host.guard(|_, _| Err("not in a preview".into()));
            host
        })
    }
}
impl Drop for Kept {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A preview in the app's VM names the app's keys: refused, it looks as the
/// app does, and what it writes is held, reaching nothing saved.
#[test]
fn a_kept_value_refused_starts_from_what_is_saved_and_saves_nothing() {
    let kept = Kept::new("held");
    let mut h = kept.refusing();
    assert_eq!(h.text("kept"), "the app's own", "as the app shows it");
    assert!(
        h.host
            .warnings()
            .iter()
            .any(|w| w.contains("not in a preview"))
    );
    h.click("set");
    h.settle();
    assert_eq!(h.text("kept"), "overwritten", "the preview's own");
    // Held across a reload of the preview.
    h.host.reload().unwrap();
    h.settle();
    assert_eq!(h.text("kept"), "overwritten");
    h.host.save();
    drop(h);
    // Quit: never saved.
    assert_eq!(kept.on_disk(), "\"the app's own\"");
}

#[test]
fn what_is_held_is_saved_when_the_person_keeps_it() {
    let kept = Kept::new("keep");
    let mut h = kept.refusing();
    h.click("set");
    h.settle();
    assert_eq!(h.host.keep_held_state(), 1);
    h.host.save();
    assert_eq!(kept.on_disk(), "\"overwritten\"");
    assert_eq!(h.host.keep_held_state(), 0, "nothing left held");
    drop(h);
    // Next run, with no guard: the kept value.
    let store = kept.dir.clone();
    let mut h =
        Harness::new(move |runtime| ScriptHost::inline(runtime, "app", PERSIST).storage(store));
    assert_eq!(h.text("kept"), "overwritten");
}

#[test]
fn what_is_held_of_a_key_with_no_file_yet_is_saved_when_kept() {
    let kept = Kept::new("keep-new");
    std::fs::remove_file(&kept.file).unwrap();
    let mut h = kept.refusing();
    assert_eq!(h.text("kept"), "initial");
    h.click("set");
    h.settle();
    assert_eq!(h.host.keep_held_state(), 1);
    h.host.save();
    assert_eq!(kept.on_disk(), "\"overwritten\"");
    drop(h);
    // Next run, with no guard: the kept value.
    let store = kept.dir.clone();
    let mut h =
        Harness::new(move |runtime| ScriptHost::inline(runtime, "app", PERSIST).storage(store));
    assert_eq!(h.text("kept"), "overwritten");
}

#[test]
fn a_held_value_with_no_file_never_changed_writes_none_when_kept() {
    let kept = Kept::new("unchanged-new");
    std::fs::remove_file(&kept.file).unwrap();
    let h = kept.refusing();
    assert_eq!(h.host.keep_held_state(), 0);
    h.host.save();
    assert!(
        !kept.file.exists(),
        "its initial stays the script's to change"
    );
}

#[test]
fn a_held_value_read_and_never_changed_saves_nothing_when_kept() {
    let kept = Kept::new("unchanged");
    let h = kept.refusing();
    assert_eq!(h.host.keep_held_state(), 0);
    h.host.save();
    assert_eq!(kept.on_disk(), "\"the app's own\"");
}

#[test]
fn what_is_held_is_gone_when_the_person_reverts() {
    let kept = Kept::new("drop");
    let mut h = kept.refusing();
    h.click("set");
    h.settle();
    h.host.drop_held_state();
    h.host.reload().unwrap();
    h.settle();
    assert_eq!(h.text("kept"), "the app's own");
    assert_eq!(h.host.keep_held_state(), 0);
    h.host.save();
    assert_eq!(kept.on_disk(), "\"the app's own\"");
}

/// A host can let pictures show and keep scripts off the network.
#[test]
fn a_remote_picture_is_asked_apart_from_a_fetch() {
    let asked: Asked = Rc::default();
    let urls: Rc<RefCell<Vec<String>>> = Rc::default();
    let (told, fetched) = (asked.clone(), urls.clone());
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r##"local said = ui.state("said", "")
            ui.task(function()
                local ok, failure = ui.try(function() return ui.fetch("https://api.test/x"):await() end)
                said:set(if ok then "fetched" else failure.kind .. ": " .. failure.message)
            end)
            return ui.column {
                ui.image { src = "https://img.test/a.png", id = "a", w = 20, h = 20 },
                ui.text { id = "said", "{said}" },
            }"##,
        );
        host.fetcher(move |request: zgx_script::Request| {
            fetched.borrow_mut().push(request.url.clone());
            async { Err("no answer here".to_string()) }
        });
        host.guard(move |kind, name| {
            told.borrow_mut().push((kind, name.to_owned()));
            match kind {
                GuardKind::Fetch => Err("not in a preview".into()),
                _ => Ok(()),
            }
        });
        host
    });
    h.settle();
    h.frames(2);
    assert!(was_asked(
        &asked,
        GuardKind::Picture,
        "https://img.test/a.png"
    ));
    assert!(was_asked(&asked, GuardKind::Fetch, "https://api.test/x"));
    assert_eq!(
        *urls.borrow(),
        ["https://img.test/a.png"],
        "the picture alone"
    );
    // A refused fetch rejects: what handles a failure says so, in its words.
    let said = h.text("said");
    assert!(said.starts_with("network: "), "{said}");
    assert!(said.contains("not in a preview"), "{said}");
    assert_eq!(h.error(), None);
}

/// A refused fetch at a file's top level doesn't fail its load.
#[test]
fn a_refused_fetch_at_the_top_level_leaves_the_load_standing() {
    let (mut h, asked) = guarded(
        r##"local answer = ui.fetch("https://api.test/x")
        return ui.text { id = "shown", "here" }"##,
        none,
    );
    assert_eq!(h.text("shown"), "here");
    assert!(was_asked(&asked, GuardKind::Fetch, "https://api.test/x"));
}

#[test]
fn a_downloads_own_dialog_is_asked_for() {
    let (mut h, asked) = guarded(
        r##"local said = ui.state("said", "")
        return ui.column {
            ui.webview { id = "page", w = 100, h = 50, on_download = function(d)
                local ok = pcall(function() return d:ask() end)
                said:set(if ok then "asked" else "refused")
                d:cancel()
            end },
            ui.text { id = "said", "{said}" },
        }"##,
        |kind, _| kind != GuardKind::Dialog,
    );
    let web = web_of(&mut h);
    let desktop = h.desktop.clone().unwrap();
    web.download(
        zgx::web_view::DownloadRequest::new("https://page.test/a.txt", "a.txt", "text/plain"),
        zgx::web_view::DownloadResponder::new(|_| {}),
    );
    h.settle();
    assert_eq!(h.text("said"), "refused");
    assert!(was_asked(&asked, GuardKind::Dialog, "ask"));
    assert!(desktop.asked().is_empty(), "no dialog");
}

/// A refused picture leaves nothing behind: allowed later (the person kept
/// the template), the next load fetches it. Refused, it isn't left "loading",
/// and what a picture that can't be shown does, it does.
#[test]
fn a_picture_refused_then_allowed_is_fetched_on_the_next_load() {
    let allowed = Rc::new(std::cell::Cell::new(false));
    let urls: Rc<RefCell<Vec<String>>> = Rc::default();
    let (open, fetched) = (allowed.clone(), urls.clone());
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r##"return ui.image {
                src = "https://img.test/a.png", id = "art", w = 20, h = 20,
                fallback = ui.text { id = "instead", "no picture" },
            }"##,
        );
        host.fetcher(move |request: zgx_script::Request| {
            fetched.borrow_mut().push(request.url.clone());
            async { Err("no answer here".to_string()) }
        });
        host.guard(
            move |kind, _| match kind == GuardKind::Picture && !open.get() {
                true => Err("not in a preview".into()),
                false => Ok(()),
            },
        );
        host
    });
    h.settle();
    h.frames(2);
    assert!(urls.borrow().is_empty(), "{:?}", urls.borrow());
    assert_eq!(h.text("instead"), "no picture");
    // Asked again, still refused: nothing reaches the network.
    h.host.reload().unwrap();
    h.settle();
    assert!(urls.borrow().is_empty(), "{:?}", urls.borrow());
    // Allowed: the next load asks, and it goes.
    allowed.set(true);
    h.host.reload().unwrap();
    h.settle();
    h.frames(2);
    assert_eq!(*urls.borrow(), ["https://img.test/a.png"]);
}

// The web guard: pages asked by the id they were made under.

const TABS: &str = r##"
local tab = ui.web("tab")
local loose = ui.web()
local said = ui.state("said", "")
return ui.column {
    ui.button { id = "load", w = 10, h = 10, on_click = function()
        local ok, why = pcall(function() loose:load("http://plain.test/loose") end)
        said:set(if ok then "loaded" else tostring(why))
    end },
    ui.text { id = "said", function() return said:get() end },
    ui.webview { web = tab, id = "tab", w = 200, h = 40 },
    ui.webview { web = loose, id = "loose", w = 200, h = 40 },
    ui.webview { id = "own", w = 200, h = 40, src = "http://plain.test/own" },
}
"##;

type WebAsked = Rc<RefCell<Vec<(Option<String>, String)>>>;

/// `TABS` under a web guard letting `http:` only into the page `tab`, and a
/// guard that refuses everything (never asked of pages while the web guard
/// is set): what each was asked.
fn web_guarded() -> (Harness, WebAsked, Asked) {
    let (web_asked, asked): (WebAsked, Asked) = Default::default();
    let (told_web, told) = (web_asked.clone(), asked.clone());
    let h = Harness::with_desktop(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", TABS);
        host.guard(move |kind, name| {
            told.borrow_mut().push((kind, name.to_owned()));
            Err("not in a preview".into())
        });
        host.web_guard(move |web, url| {
            told_web
                .borrow_mut()
                .push((web.map(str::to_owned), url.to_owned()));
            match web == Some("tab") || url.starts_with("https:") {
                true => Ok(()),
                false => Err("https only here".into()),
            }
        });
        host
    });
    (h, web_asked, asked)
}
fn web_named(h: &mut Harness, id: &str) -> WebView {
    h.ui.prepare_frame();
    let node = h.find(id).expect("the web view");
    match h.natives.content(node) {
        Some(NativeContent::Web(web)) => web,
        _ => panic!("{id} is no web view"),
    }
}

/// A main frame, an iframe and a redirect of the page made as `ui.web("tab")`
/// are each asked with its id; an anonymous page's with none; and the
/// guard isn't asked of pages at all.
#[test]
fn the_web_guard_is_told_which_page_navigates() {
    let (mut h, web_asked, asked) = web_guarded();
    let tab = web_named(&mut h, "tab");
    let main = navigation("http://plain.test/main", NavigationKind::Link);
    let iframe = NavigationRequest {
        main_frame: false,
        ..navigation("http://plain.test/frame", NavigationKind::Other)
    };
    let redirect = NavigationRequest {
        source_url: "http://plain.test/main".into(),
        ..navigation("http://plain.test/moved", NavigationKind::Other)
    };
    for request in [&main, &iframe, &redirect] {
        assert!(tab.allows(request), "{}", request.url);
    }
    let tab_id = Some("tab".to_owned());
    assert_eq!(
        web_asked.borrow()[web_asked.borrow().len() - 3..],
        [
            (tab_id.clone(), "http://plain.test/main".to_owned()),
            (tab_id.clone(), "http://plain.test/frame".to_owned()),
            (tab_id, "http://plain.test/moved".to_owned()),
        ]
    );
    // The anonymous page: no id, so `http:` is refused, its iframe too.
    let loose = web_named(&mut h, "loose");
    assert!(!loose.allows(&main));
    assert!(!loose.allows(&iframe));
    assert!(loose.allows(&navigation("https://secure.test/", NavigationKind::Link)));
    assert_eq!(
        web_asked.borrow().last().unwrap(),
        &(None, "https://secure.test/".to_owned())
    );
    assert!(h.error().unwrap_or_default().contains("https only here"));
    assert!(
        !asked
            .borrow()
            .iter()
            .any(|(kind, _)| *kind == GuardKind::Web),
        "the guard was asked of a page: {:?}",
        asked.borrow()
    );
}

/// What a script loads is asked too: refused, the page doesn't go there and
/// the script hears why; a webview's own `src` (no `ui.web` id) likewise.
#[test]
fn a_page_the_web_guard_refuses_loads_nothing() {
    let (mut h, web_asked, _) = web_guarded();
    let loose = web_named(&mut h, "loose");
    h.click("load");
    h.settle();
    assert!(
        h.text("said").contains("https only here"),
        "{}",
        h.text("said")
    );
    assert_ne!(loose.url(), "http://plain.test/loose");
    assert!(
        web_asked
            .borrow()
            .contains(&(None, "http://plain.test/own".to_owned())),
        "{:?}",
        web_asked.borrow()
    );
    assert_ne!(web_named(&mut h, "own").url(), "http://plain.test/own");
}

/// With no web guard, the guard decides of pages as before.
#[test]
fn with_no_web_guard_the_guard_decides_of_pages() {
    let (mut h, asked) = guarded(TABS, |_, name| name.starts_with("https:"));
    let tab = web_named(&mut h, "tab");
    assert!(!tab.allows(&navigation("http://plain.test/main", NavigationKind::Link)));
    assert!(tab.allows(&navigation("https://secure.test/", NavigationKind::Link)));
    assert!(was_asked(&asked, GuardKind::Web, "http://plain.test/main"));
}
