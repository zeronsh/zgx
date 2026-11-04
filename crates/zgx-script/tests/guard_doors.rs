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
    let dir = std::env::temp_dir().join(format!("zgx-guard-web-{}", std::process::id()));
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
