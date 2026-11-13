//! Developer diagnostics: the error banner over the window, and the layout
//! inspector's view of a script (where each node was created).
mod common;

use common::Harness;
use std::time::Duration;
use zgx::{inspect::Inspector, scene::NodeKind};
use zgx_script::ScriptHost;

const APP: &str = r#"local count = ui.state("count", 0)
local function bump()
    error("bump failed")
end
return ui.column {
    p = 12, gap = 8,
    ui.text { "Count: {count}", id = "label" },
    ui.button { id = "inc", w = 120, h = 40, on_click = ui.add(count), "Increment" },
    ui.button { id = "bump", w = 120, h = 40, on_click = bump, "Bump" },
}
"#;

fn temp_script(name: &str, code: &str) -> std::path::PathBuf {
    let dir = common::temp_dir(&format!("diag-{name}"));
    let path = dir.join("app.luau");
    std::fs::write(&path, code).unwrap();
    path
}
fn rewrite(path: &std::path::Path, code: &str) {
    std::fs::write(path, code).unwrap();
    let later = std::time::SystemTime::now() + Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(later)
        .unwrap();
}
/// Every text under node `id`, in order.
fn texts(h: &mut Harness, id: &str) -> Vec<String> {
    let node = h.find(id).unwrap_or_else(|| panic!("no `{id}`"));
    let scene = h.ui.scene.borrow();
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if let NodeKind::Text { text, .. } = scene.kind(n) {
            out.push(text.to_string());
        }
        stack.extend(scene.children(n).iter().rev());
    }
    out
}

#[test]
fn no_error_means_no_banner_nodes() {
    let mut h = Harness::inline(APP);
    let nodes = h.ui.scene.borrow().len();
    let host_root = h.handle.node();
    assert_eq!(
        h.ui.scene.borrow().children(host_root).len(),
        1,
        "the interface alone: no banner region"
    );
    assert!(h.find("script-error-banner").is_none());

    h.host.error().set(Some("app: something broke".into()));
    h.settle();
    assert!(h.find("script-error-banner").is_some());
    assert!(h.ui.scene.borrow().len() > nodes);

    h.host.error().set(None);
    h.settle();
    assert!(h.find("script-error-banner").is_none());
    assert_eq!(h.ui.scene.borrow().len(), nodes, "nothing left behind");
    assert_eq!(h.ui.scene.borrow().children(host_root).len(), 1);
}

#[test]
fn a_failed_reload_shows_where_and_a_good_one_clears_it() {
    let path = temp_script("reload", APP);
    let mut h = Harness::new(|runtime| ScriptHost::new(runtime, &path));
    let layout = |h: &mut Harness| {
        let node = h.find("inc").unwrap();
        h.ui.scene.borrow().bounds(node)
    };
    let before = layout(&mut h);
    // The user's case: a local function used before its definition.
    rewrite(
        &path,
        "local function render()\n    return ui.column { helper() }\nend\n\
         local function helper()\n    return ui.text 'hi'\nend\nreturn render()\n",
    );
    assert!(h.host.reload_if_changed());
    h.settle();
    let banner = texts(&mut h, "script-error-banner");
    assert_eq!(banner[0], "app.luau:2", "{banner:?}");
    assert!(
        banner[1].contains("attempt to call a nil value"),
        "{banner:?}"
    );
    assert!(
        banner[3..].iter().any(|line| line.contains("app.luau:7")),
        "the traceback follows: {banner:?}"
    );
    // Over the interface, not in its flow: the old interface stays put.
    assert_eq!(layout(&mut h), before, "the banner takes no room");
    let b = {
        let node = h.find("script-error-banner").unwrap();
        h.ui.scene.borrow().bounds(node)
    };
    assert!(b.x >= 12. && b.x + b.width <= 388. + 0.5, "{b:?}");
    assert!(b.y + b.height <= 288. + 0.5 && b.height > 10., "{b:?}");
    assert_eq!(h.text("label"), "Count: 0", "the interface keeps running");

    rewrite(&path, APP);
    assert!(h.host.reload_if_changed());
    h.settle();
    assert_eq!(h.host.error().get(), None);
    assert!(
        h.find("script-error-banner").is_none(),
        "a good reload clears it"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn handler_errors_show_and_the_banner_dismisses() {
    let mut h = Harness::inline(APP);
    h.click("bump");
    let banner = texts(&mut h, "script-error-banner");
    assert_eq!(banner[0], "app:3", "{banner:?}");
    assert!(banner[1].contains("bump failed"), "{banner:?}");
    h.click("script-error-dismiss");
    assert!(h.find("script-error-banner").is_none());
    assert_eq!(h.host.error().get(), None);
    h.click("inc");
    assert_eq!(h.text("label"), "Count: 1");
}

/// An error raised in an open modal: the banner shows over the modal and
/// its × dismisses it, the click going to the banner, not to the modal's
/// backdrop (which closed the modal and left the error).
#[test]
fn the_banner_dismisses_over_an_open_modal() {
    let mut h = Harness::inline(
        r#"local open = ui.state("open", true)
return ui.column {
    p = 12,
    ui.text { id = "state", function() return if open:get() then "open" else "closed" end },
    ui.modal {
        open = open,
        label = "Dialog",
        w = 200, h = 120,
        ui.button { id = "boom", w = 120, h = 40, on_click = function() error("boom") end, "Boom" },
    },
}
"#,
    );
    assert_eq!(h.text("state"), "open");
    h.click("boom");
    assert!(h.find("script-error-banner").is_some());
    h.click("script-error-dismiss");
    assert_eq!(h.host.error().get(), None, "the click reached the banner");
    assert!(h.find("script-error-banner").is_none());
    assert_eq!(h.text("state"), "open", "the modal stays open");
    // It still takes its own clicks.
    h.click("boom");
    assert!(h.find("script-error-banner").is_some());
}

#[test]
fn the_inspector_names_nodes_by_the_script_line_that_made_them() {
    let inspector = Inspector::off();
    let code = r#"return ui.column {
    w = 400, h = 300,
    ui.column {
        id = "clip", class = "pane", w = 100, h = 40, overflow = "hidden",
        ui.div { w = 100, h = 120, shrink = 0 },
    },
    ui.text { "plain", id = "plain" },
}
"#;
    let mut h = Harness::inspected(
        |runtime| ScriptHost::inline(runtime, "app", code),
        &inspector,
    );
    let clip = h.find("clip").unwrap();
    assert_eq!(inspector.label_of(clip), None, "off: nothing recorded");

    // Turning it on records sources, reloading the script.
    inspector.set_enabled(true);
    h.settle();
    h.ui.prepare_frame();
    let clip = h.find("clip").unwrap();
    let label = inspector.label_of(clip).expect("labelled");
    assert_eq!(&*label.name, "column #clip .pane");
    assert_eq!(label.source.as_deref(), Some("app:3"));
    let plain = h.find("plain").unwrap();
    assert_eq!(
        inspector.label_of(plain).and_then(|l| l.source).as_deref(),
        Some("app:7")
    );

    inspector.update(&h.ui, None);
    let warnings = inspector.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:#?}");
    assert_eq!(
        warnings[0].to_string(),
        "app:3: column #clip .pane: content overflows its clip vertically by 80 (120 in 40) \
         and it does not scroll; use ui.scroll, or make room"
    );
    let b = h.ui.scene.borrow().bounds(clip);
    inspector.update(&h.ui, Some((b.x + b.width - 1., b.y + 1.)));
    // The pointer is over the clipped content: a `ui.div`, laid out as a column.
    let details = inspector.details().join("\n");
    assert!(details.contains("div · column"), "{details}");
    assert!(details.contains("created at app:5"), "{details}");
    assert!(details.contains("clipped to 100 × 40"), "{details}");

    inspector.set_enabled(false);
    inspector.update(&h.ui, None);
    assert_eq!(inspector.overlay(), None);
}

/// A panic while reloading shows as a script error does, and the next save
/// reloads: hot reload outlives it.
#[test]
fn a_panic_in_a_reload_is_reported_and_the_next_save_reloads() {
    let path = temp_script("reload-panic", APP);
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::new(runtime, &path);
        host.action("boom", |_| panic!("boom in a reload"));
        host
    });
    h.settle();
    rewrite(&path, "ui.invoke(\"boom\")\nreturn ui.text { \"never\" }\n");
    assert!(h.host.reload_if_changed());
    h.settle();
    let error = h.host.error().get().unwrap_or_default();
    assert!(error.contains("boom in a reload"), "{error}");
    rewrite(&path, &APP.replace("Count:", "Total:"));
    assert!(h.host.reload_if_changed());
    h.settle();
    h.ui.prepare_frame();
    assert_eq!(h.host.error().get(), None);
    assert_eq!(h.text("label"), "Total: 0");
}

/// Watching, a reload follows a save without waiting out the poll: the
/// kernel says when a file changes (macOS), in place or replaced.
#[cfg(target_os = "macos")]
#[test]
fn a_watching_host_reloads_when_a_file_is_saved() {
    let path = temp_script("watch", APP);
    let h = Harness::new(|runtime| ScriptHost::new(runtime, &path));
    // A poll far longer than the test: only a file event reloads in time.
    let watching = h.host.watch(Duration::from_secs(600));
    h.executor.borrow_mut().spawn(watching);
    h.settle();
    let mut h = h;
    let wait_for = |h: &mut Harness, label: &str| {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            h.settle();
            h.ui.prepare_frame();
            if h.text("label") == label {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    };
    // Written in place.
    rewrite(&path, &APP.replace("Count:", "Total:"));
    assert!(wait_for(&mut h, "Total: 0"), "{}", h.text("label"));
    // Replaced, as editors save: written beside it and renamed over it.
    let beside = path.with_extension("tmp");
    std::fs::write(&beside, APP.replace("Count:", "Sum:")).unwrap();
    std::fs::rename(&beside, &path).unwrap();
    assert!(wait_for(&mut h, "Sum: 0"), "{}", h.text("label"));
    // A burst of writes, as a save in several parts: what shows is the
    // last (a reload waits for the writes to stop).
    for word in ["One:", "Two:", "Three:"] {
        rewrite(&path, &APP.replace("Count:", word));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(wait_for(&mut h, "Three: 0"), "{}", h.text("label"));
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn a_debug_build_lints_its_scripts_after_each_load() {
    let dir = common::temp_dir("lint-on-load");
    let app = dir.join("app.luau");
    std::fs::write(
        &app,
        "local n = ui.state(\"n\", 0)\nreturn ui.text { \"n: \" .. n:get() }\n",
    )
    .unwrap();
    let h = Harness::new(|runtime| ScriptHost::new(runtime, app.clone()));
    let findings = h.host.lint_findings().with_untracked(|f| f.clone());
    let text = findings.expect("the untracked read is found");
    assert!(
        text.contains("app.luau:2:") && text.contains("untracked-get"),
        "{text}"
    );
    assert!(
        h.host
            .warnings()
            .iter()
            .any(|w| w.contains("lint[untracked-get]")),
        "{:?}",
        h.host.warnings()
    );
    std::fs::write(
        &app,
        "local n = ui.state(\"n\", 0)\nreturn ui.text { \"n: {n}\" }\n",
    )
    .unwrap();
    h.host.reload().unwrap();
    assert_eq!(
        h.host.lint_findings().with_untracked(|f| f.clone()),
        None,
        "fixed, it goes"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The same error raised twice reaches `on_error` twice: each is an error,
/// though the banner (shown or not) is the same.
#[test]
fn a_repeated_error_reaches_the_error_hook_each_time() {
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = seen.clone();
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "hook_probe",
            r#"return ui.button { id = "fail", w = 120, h = 35, "Fail", on_click = function() error("same-boom") end }"#,
        )
        .error_banner(false);
        host.on_error(move |e| sink.borrow_mut().push(e.to_string()));
        host
    });
    h.click("fail");
    h.frames(1);
    h.click("fail");
    h.frames(1);
    assert_eq!(seen.borrow().len(), 2, "{:?}", seen.borrow());
    assert!(seen.borrow().iter().all(|e| e.contains("same-boom")));
}

/// A scroll view given only a cap (`max_h`) scrolls: its content taller than
/// the cap is not an overflow that "does not scroll".
#[test]
fn a_scroll_view_capped_by_max_h_is_not_flagged_as_overflowing() {
    let inspector = Inspector::off();
    let code = r#"return ui.column {
    w = 300,
    ui.scroll {
        id = "s", w_full = true, max_h = 100,
        ui.column { ui.div { w = 100, h = 300, shrink = 0 } },
    },
}
"#;
    let mut h = Harness::inspected(
        |runtime| ScriptHost::inline(runtime, "app", code),
        &inspector,
    );
    inspector.set_enabled(true);
    h.settle();
    h.ui.prepare_frame();
    let scroll = h.find("s").unwrap();
    assert_eq!(h.ui.scene.borrow().bounds(scroll).height, 100.);
    inspector.update(&h.ui, None);
    let warnings = inspector.warnings();
    assert!(warnings.is_empty(), "{warnings:#?}");
}

/// `zgx inspect gc`: a full collection per VM with the heap before and after,
/// and what is left by type and code chunk.
#[test]
fn gc_report_separates_garbage_from_what_scripts_keep() {
    let code = r#"
local kept = {}
for i = 1, 3000 do kept[i] = { i, tostring(i) } end
for i = 1, 30000 do local garbage = { i, i + 1, i + 2 } end
return ui.button { id = "b", on_click = function() local _ = #kept end, "keeps a table" }
"#;
    let h = Harness::new(|runtime| ScriptHost::inline(runtime, "app", code));
    h.settle();
    let report = h.host.gc_report(&Default::default());
    assert!(!report.is_empty());
    let vm = &report[0];
    assert!(
        vm.before > vm.after,
        "{} before, {} after",
        vm.before,
        vm.after
    );
    let tables = vm
        .by_type
        .iter()
        .find(|(name, ..)| name == "table")
        .expect("tables");
    assert!(tables.1 >= 3000, "{tables:?}");
    assert!(
        vm.chunks.iter().any(|(name, ..)| name.contains("app")),
        "{:?}",
        vm.chunks
    );
    // A second collection frees nothing more: what is left is kept.
    let again = &h.host.gc_report(&Default::default())[0];
    assert!(
        again.before <= vm.after + 4096,
        "{} vs {}",
        again.before,
        vm.after
    );
}

/// `zgx inspect gc --mark`, `--since-mark` and `--retained`: what a click
/// leaves behind after the mark is named, with the chain of holders from a
/// root, its closures by chunk and line.
#[test]
fn gc_names_what_survives_after_a_mark_and_who_holds_it() {
    use zgx_script::GcQuery;
    let code = r#"
local kept = {}
local function remember(i) kept[#kept + 1] = { label = "row" .. i, run = function() return i end } end
return ui.button { id = "b", on_click = function() remember(#kept) end, "keeps" }
"#;
    let mut h = Harness::new(|runtime| ScriptHost::inline(runtime, "app", code));
    h.settle();
    let mark = GcQuery {
        top: 10,
        mark: true,
        ..Default::default()
    };
    assert!(h.host.gc_report(&mark)[0].found.marked.unwrap() > 0);
    let b = h.find("b").unwrap();
    let rect = h.ui.scene.borrow().bounds(b);
    let (x, y) = (rect.x + rect.width / 2., rect.y + rect.height / 2.);
    for _ in 0..7 {
        let button = zgx::input::PointerButton::Primary;
        h.ui.dispatch(zgx::input::InputEvent::PointerDown { x, y, button });
        h.ui.dispatch(zgx::input::InputEvent::PointerUp { x, y, button });
        h.settle();
    }
    let query = GcQuery {
        top: 10,
        retained: Some("run".into()),
        sample: 3,
        since_mark: true,
        ..Default::default()
    };
    let found = &h.host.gc_report(&query)[0].found;
    assert!(!found.no_mark);
    assert!(
        found
            .since
            .iter()
            .any(|(name, count, _)| name.starts_with("function run @") && *count == 7),
        "{:?}",
        found.since
    );
    assert_eq!(found.matched, 7, "{found:?}");
    let holder = &found.holders[0];
    assert_eq!(holder.count, 7);
    let path = holder.path.join(" | ");
    assert!(
        path.starts_with("registry") || path.contains("mainthread"),
        "{path}"
    );
    assert!(path.contains("upvalue") && path.contains(".run"), "{path}");
    assert!(path.contains("@") && path.contains(":"), "{path}");
}

/// A node's callbacks (handlers and function bindings) are let go when it
/// unmounts: after a full collection what they captured is gone, mount after
/// mount (a busy window waits on the collector until the host asks for one).
#[test]
fn a_nodes_prop_callbacks_are_released_when_it_unmounts() {
    for prop in [
        "on_click = function() local _ = #big end",
        "w = function() local _ = #big return 10 end",
        "on_mount = function() local _ = #big end",
        "on_resize = function() local _ = #big end",
        "",
    ] {
        let code = format!(
            r##"
local on = ui.state("on", false)
local Probe = ui.component(function()
    local big = {{}} for i = 1, 300 do big[i] = {{ i }} end
    return ui.div {{ id = "p", w = 10, h = 10, {prop} }}
end)
return ui.column {{
    ui.button {{ id = "t", w = 40, h = 20, "t", on_click = function() on:set(not on:peek()) end }},
    ui.show(function() return on:get() end, Probe {{}}),
}}"##
        );
        let dir = common::temp_dir("prop-callbacks");
        std::fs::write(dir.join("app.luau"), &code).unwrap();
        let path = dir.join("app.luau");
        let mut h = Harness::new(|runtime| ScriptHost::new(runtime, &path));
        h.settle();
        let mut shown = false;
        let mut toggle = |h: &mut Harness| {
            shown = !shown;
            h.host
                .set_state("on", zgx_script::Value::Bool(shown))
                .unwrap();
            h.settle();
        };
        for _ in 0..20 {
            toggle(&mut h);
        }
        assert!(h.find("p").is_none(), "off after an even number of toggles");
        toggle(&mut h);
        assert!(h.find("p").is_some(), "mounted");
        toggle(&mut h);
        let before = h.host.gc_report(&Default::default())[0].after;
        for _ in 0..400 {
            toggle(&mut h);
        }
        let grown = h.host.gc_report(&Default::default())[0]
            .after
            .saturating_sub(before);
        assert!(
            grown < 256 * 1024,
            "`{prop}`: 200 mounts held {} KB more",
            grown / 1024
        );
    }
}

/// A component whose build reaches its own factory (a recursive component,
/// a module table holding both) leaves nothing behind when it unmounts: the
/// factory held a handle to its build inside its own userdata, a root the
/// collector could not see past, so each mount's states and closures stayed.
#[test]
fn a_component_that_reaches_its_own_factory_leaves_no_states_when_it_unmounts() {
    let code = r##"
local on = ui.state("on", false)
local Outer = ui.component(function()
    local Inner
    local shared = ui.use_state(0)
    Inner = ui.component(function(props)
        local hovered = ui.state("hovered", false)
        local _ = shared:peek()
        local depth = props.depth or 0
        return ui.div { id = "p", w = 50, h = 50,
            on_hover = function(inside) hovered:set(inside) end,
            bg = function() return if hovered:get() then "#112233" else "#000000" end,
            if depth < 1 then Inner { depth = depth + 1 } else nil }
    end)
    return Inner {}
end)
return ui.column { ui.show(function() return on:get() end, Outer {}) }"##;
    let mut h = Harness::new(|runtime| ScriptHost::inline(runtime, "app", code));
    h.settle();
    let mut shown = false;
    let mut toggle = |h: &mut Harness| {
        shown = !shown;
        h.host
            .set_state("on", zgx_script::Value::Bool(shown))
            .unwrap();
        h.settle();
        h.frames(2);
    };
    for _ in 0..10 {
        toggle(&mut h);
    }
    let _ = h.host.gc_report(&zgx_script::GcQuery {
        mark: true,
        ..Default::default()
    });
    for _ in 0..100 {
        toggle(&mut h);
    }
    let found = &h.host.gc_report(&zgx_script::GcQuery {
        retained: Some("State".into()),
        since_mark: true,
        ..Default::default()
    })[0]
        .found;
    assert!(
        found.matched <= 4,
        "100 mounts left {} states: {:?}",
        found.matched,
        found.holders.first().map(|h| &h.path)
    );
}

/// `gc --retained` says how many matches a Luau holder alone reaches: a
/// mounted node's state, held only by the handle Rust keeps to its handler's
/// closure, is not one.
#[test]
fn gc_tells_luau_holders_from_rust_handles() {
    let code = r##"
local on = ui.state("on", false)
local Probe = ui.component(function()
    local s = ui.use_state(0)
    return ui.div { id = "p", w = 10, h = 10, on_hover = function() s:set(1) end }
end)
return ui.column { ui.show(function() return on:get() end, Probe {}) }"##;
    let h = Harness::new(|runtime| ScriptHost::inline(runtime, "app", code));
    h.settle();
    h.host
        .set_state("on", zgx_script::Value::Bool(true))
        .unwrap();
    h.settle();
    h.frames(2);
    let found = &h.host.gc_report(&zgx_script::GcQuery {
        retained: Some("State".into()),
        sample: 3,
        ..Default::default()
    })[0]
        .found;
    assert!(found.matched >= 1, "{found:?}");
    assert_eq!(found.lua_reachable, 0, "held from Rust only: {found:?}");
    assert!(
        found
            .holders
            .iter()
            .any(|h| h.path.iter().any(|step| step.contains("on_hover"))),
        "{found:?}"
    );
}

/// A timer ends when what made it unmounts, not when it next falls due: what
/// its callback holds is let go at once (a component's polling `ui.every`
/// of a minute, a toast's `ui.after` of five, kept a tab's states that long).
#[test]
fn a_timer_of_an_unmounted_component_lets_go_of_what_it_holds_at_once() {
    let code = r##"
local on = ui.state("on", false)
local Probe = ui.component(function()
    local s = ui.use_state(0)
    ui.every(60000, function() s:set(s:peek() + 1) end)
    ui.after(300000, function() s:set(0) end)
    return ui.div { id = "p", w = 10, h = 10 }
end)
return ui.column { ui.show(function() return on:get() end, Probe {}) }"##;
    let mut h = Harness::new(|runtime| ScriptHost::inline(runtime, "app", code));
    h.settle();
    let mut shown = false;
    let mut toggle = |h: &mut Harness| {
        shown = !shown;
        h.host
            .set_state("on", zgx_script::Value::Bool(shown))
            .unwrap();
        h.settle();
        h.frames(2);
    };
    for _ in 0..4 {
        toggle(&mut h);
    }
    let _ = h.host.gc_report(&zgx_script::GcQuery {
        mark: true,
        ..Default::default()
    });
    for _ in 0..40 {
        toggle(&mut h);
    }
    let found = &h.host.gc_report(&zgx_script::GcQuery {
        retained: Some("State".into()),
        since_mark: true,
        ..Default::default()
    })[0]
        .found;
    assert!(
        found.matched <= 2,
        "40 mounts left {} states held by timers: {:?}",
        found.matched,
        found.holders.first().map(|h| &h.path)
    );
}

/// A state keyed by a tab that has since closed is dropped once nothing holds
/// it and it has not moved, so a window that opens tabs all day stays flat.
#[test]
fn states_keyed_by_closed_tabs_do_not_pile_up() {
    let code = r##"
local ids = ui.state("ids", { "start" })
local Tab = ui.component(function(props)
	local open = ui.state(`open:{props.id}`, false)
	local feed = ui.shared(`feed:{props.id}`, nil)
	return ui.div { id = "t", w = 10, h = 10 }
end)
return ui.column { ui.each(ids, function(id) return Tab { id = id } end) }"##;
    let mut h = Harness::new(|runtime| ScriptHost::inline(runtime, "app", code));
    h.settle();
    let flip = |h: &mut Harness, k: i64| {
        let ids = zgx_script::Value::List(
            vec![zgx_script::Value::from(format!("tab-{k}").as_str())].into(),
        );
        h.host.set_state("ids", ids).unwrap();
        h.settle();
        h.frames(2);
    };
    for k in 0..6 {
        flip(&mut h, k);
    }
    h.host.collect_garbage();
    let before = h.host.states().len();
    for k in 6..86 {
        flip(&mut h, k);
    }
    h.host.collect_garbage();
    let after = h.host.states().len();
    assert!(
        after <= before + 2,
        "80 tabs left {} more states",
        after - before
    );
}

const SOAK: &str = r##"
local step = ui.state("step", 0)
local mode = ui.state("mode", "plain")
local heap = ui.state("heap", 0)
ui.effect(function()
	if step:get() % 2 == 0 then heap:set(math.floor(collectgarbage("count"))) end
end)
local Box = ui.component(function(props)
	local big = table.create(2000, props.n)
	local m = mode:peek()
	if m == "plain" then
		return ui.column { ui.text { "open" } }
	elseif m == "key" then
		return ui.column { on_key = function(_e) return #big > 0 end, ui.text { "open" } }
	elseif m == "fn" then
		return ui.column { pr = function() return #big > 0 and 1 or 0 end, ui.text { "open" } }
	elseif m == "hover" then
		local on = ui.state("h", false)
		return ui.column { ui.div { w = 20, h = 20, on_hover = function(x) on:set(x and #big > 0) end } }
	elseif m == "build" then
		local open = ui.state("o", false)
		return ui.column { ui.popover { open = open, anchor = ui.button { "a" }, build = function() return ui.text { tostring(#big) } end } }
	elseif m == "use_state" then
		local s = ui.use_state(big)
		return ui.column { ui.text { function() return tostring(#s:get()) end } }
	end
	return ui.column { on_mount = function() local _ = #big end, ui.text { "open" } }
end)
return ui.column { w = 400, h = 300,
	ui.presence(function()
		local s = step:get()
		return if s % 2 == 1 then tostring(s) else ""
	end, function(key)
		return if key == "" then ui.text { "closed" } else Box { n = tonumber(key) or 0 }
	end),
}"##;

fn collected_heap_growth(mode: &str, cycles: i64) -> (usize, usize, usize) {
    let mut h = Harness::new(|runtime| ScriptHost::inline(runtime, "app", SOAK));
    h.host
        .set_state("mode", zgx_script::Value::from(mode))
        .unwrap();
    h.settle();
    let mut step = 0.;
    let mut cycle = |h: &mut Harness| {
        for _ in 0..2 {
            step += 1.;
            h.host
                .set_state("step", zgx_script::Value::Num(step))
                .unwrap();
            h.settle();
            h.frames(2);
        }
    };
    for _ in 0..20 {
        cycle(&mut h);
    }
    let before = h.host.gc_report(&Default::default())[0].after;
    let mut peak = 0;
    for _ in 0..cycles {
        cycle(&mut h);
        if let Some(zgx_script::Value::Num(n)) = h
            .host
            .states()
            .into_iter()
            .find(|(k, _)| k.ends_with("heap"))
            .map(|(_, v)| v)
        {
            peak = peak.max(n as usize);
        }
    }
    let r = &h.host.gc_report(&Default::default())[0];
    (before, r.after, peak)
}

/// What a component that unmounts holds (a handler, a function prop, a state
/// over a large table) goes with it: the collected heap stays flat across
/// mounts, and the uncollected one stays within a few times what is live.
#[test]
fn unmounted_components_let_go_of_what_their_closures_held() {
    for mode in ["key", "fn", "use_state", "mount", "hover", "build"] {
        let (before, after, peak) = collected_heap_growth(mode, 400);
        assert!(
            after < before + 64 * 1024,
            "`{mode}`: 400 mounts left {} KB more after a collection",
            after.saturating_sub(before) / 1024
        );
        assert!(
            peak < 6 * 1024,
            "`{mode}`: the heap reached {peak} KB between collections"
        );
    }
}

/// A key the host published into and set back to nil, which no script holds,
/// goes at the next collection.
#[test]
fn a_host_key_set_back_to_nil_goes_once_nothing_holds_it() {
    use zgx_script::Value as V;
    let h = Harness::inline(r##"return ui.column { w = 10, h = 10 }"##);
    h.settle();
    let shared = h.host.shared("chat_live:gone", V::from("rows"));
    assert!(h.host.states().iter().any(|(n, _)| n == "chat_live:gone"));
    shared.set(V::Nil);
    h.host.collect_garbage();
    assert!(
        h.host.states().iter().any(|(n, _)| n == "chat_live:gone"),
        "the host still holds it"
    );
    drop(shared);
    h.host.collect_garbage();
    assert!(!h.host.states().iter().any(|(n, _)| n == "chat_live:gone"));
}

#[test]
fn a_span_label_is_the_runs_accessible_name() {
    let h = Harness::inline(
        r##"return ui.column { w = 400, h = 100,
    ui.rich { id = "p", ui.span { "see " }, ui.span { "1", label = "Citation 1, Sleep study.pdf", on_click = function() end } },
}"##,
    );
    h.settle();
    h.frames(3);
    let names: Vec<String> =
        h.ui.semantics
            .borrow()
            .iter()
            .filter(|(_, s)| s.role == zgx::semantics::Role::Button)
            .map(|(_, s)| s.label.clone())
            .collect();
    assert_eq!(names, vec!["Citation 1, Sleep study.pdf".to_string()]);
}

#[test]
fn pressed_aria_hidden_and_inert_follow_their_states() {
    use zgx_script::Value as V;
    let mut h = Harness::inline(
        r##"
local on = ui.state("on", false)
return ui.column { w = 400, h = 300,
    ui.button { id = "toggle", label = "Bold", pressed = on, "B" },
    ui.column { id = "deco", aria_hidden = on, ui.text { "decoration" } },
    ui.column { id = "off", inert = on, ui.button { id = "inside", "Inside" } },
    ui.button { id = "after", "After" },
}"##,
    );
    h.settle();
    h.frames(3);
    let node = |h: &mut Harness, id: &str| h.find(id).unwrap();
    let sem = |h: &mut Harness, id: &str| {
        let n = h.find(id).unwrap();
        h.ui.semantics.borrow().get(n).cloned()
    };
    let toggle = sem(&mut h, "toggle").unwrap();
    assert_eq!(toggle.role, zgx::semantics::Role::Button);
    assert_eq!(toggle.checked, Some(false));
    assert!(!sem(&mut h, "deco").is_some_and(|s| s.hidden));
    h.host.set_state("on", V::Bool(true)).unwrap();
    h.frames(3);
    assert_eq!(sem(&mut h, "toggle").unwrap().checked, Some(true));
    assert!(sem(&mut h, "deco").is_some_and(|s| s.hidden));
    let inside = node(&mut h, "inside");
    let off = node(&mut h, "off");
    assert!(h.ui.scene.borrow().is_inert(off));
    h.ui.input.focus(&h.ui.scene, Some(inside));
    h.ui.dispatch(zgx::input::InputEvent::KeyDown {
        key: zgx::input::Key::Tab,
        modifiers: Default::default(),
        repeat: false,
    });
    h.frames(2);
    assert_ne!(
        h.ui.input.focused(),
        Some(inside),
        "focus moved off the inert subtree"
    );
    h.host.set_state("on", V::Bool(false)).unwrap();
    h.frames(3);
    assert!(!h.ui.scene.borrow().is_inert(off));
}

#[test]
fn an_sr_only_text_is_one_point_square_and_painted_nowhere() {
    let mut h = Harness::inline(
        r##"return ui.column { w = 400, h = 100,
    ui.text { id = "live", sr_only = true, live = "polite", "3 results" },
    ui.text { id = "after", "after" },
}"##,
    );
    h.settle();
    h.frames(3);
    assert_eq!(h.error(), None);
    let live = h.find("live").unwrap();
    let bounds = h.bounds("live");
    assert_eq!((bounds.width, bounds.height), (1., 1.));
    assert!(h.ui.scene.borrow().effects(live).opacity < 0.01);
    assert!(h.ui.semantics.borrow().get(live).is_some());
    assert_eq!(h.bounds("after").y, 0., "it takes no room");
}

#[test]
fn text_of_reads_what_a_control_shows() {
    let mut h = Harness::inline(
        r##"
local out = ui.state("out", "")
ui.command { id = "read", run = function()
    for _, c in ui.controls() do
        if c.role == "dialog" then out:set(ui.text_of(c.handle) .. "|" .. ui.text_of(c.handle, 5)) end
    end
end }
return ui.column { w = 400, h = 300,
    ui.column { role = "dialog", label = "Notice",
        ui.text { "Heads up" },
        ui.text { "something happened" },
        ui.column { aria_hidden = true, ui.text { "decoration" } },
    },
    ui.text { id = "out", function() return out:get() end },
}"##,
    );
    h.settle();
    h.frames(3);
    h.host.run_command("read");
    h.frames(2);
    assert_eq!(h.text("out"), "Heads up something happened|Heads");
}

#[test]
fn scroll_info_says_how_far_a_scroll_view_can_go() {
    let mut h = Harness::inline(
        r##"
local out = ui.state("out", "")
local box = ui.ref()
local lst = ui.ref()
ui.command { id = "read", run = function()
    local info = box:get():scroll_info()
    local list = lst:get():scroll_info()
    out:set(`{info.offset}/{info.range}/{tostring(info.at_start)}/{tostring(info.at_end)}|{list.range}/{tostring(list.at_start)}`)
end }
return ui.column { w = 200, h = 300,
    ui.scroll { ref = box, id = "s", w = 200, h = 100,
        ui.column { h = 400, ui.text { "tall" } } },
    ui.list { ref = lst, count = 100, height = 20, w = 200, h = 100,
        row = function(i) return ui.text("row " .. i) end },
    ui.text { id = "out", function() return out:get() end },
}"##,
    );
    h.settle();
    h.frames(3);
    h.host.run_command("read");
    h.frames(2);
    assert_eq!(h.text("out"), "0/300/true/false|1900/true");
}

#[test]
fn busy_follows_its_state() {
    use zgx_script::Value as V;
    let mut h = Harness::inline(
        r##"local on = ui.state("on", true)
return ui.column { w = 200, h = 100,
    ui.column { id = "feed", role = "region", label = "Notices", busy = on, ui.text { "loading" } },
}"##,
    );
    h.settle();
    h.frames(3);
    let feed = h.find("feed").unwrap();
    let state = |h: &Harness| h.ui.semantics.borrow().get(feed).map(|s| (s.role, s.busy));
    assert_eq!(state(&h), Some((zgx::semantics::Role::Region, true)));
    h.host.set_state("on", V::Bool(false)).unwrap();
    h.frames(3);
    assert_eq!(state(&h), Some((zgx::semantics::Role::Region, false)));
}

#[test]
fn a_cut_menu_says_whether_it_can_scroll() {
    let mut h = Harness::inline(
        r##"
local open = ui.state("open", true)
local items = {}
for i = 1, 20 do table.insert(items, ui.menu_item { "Item " .. i }) end
local out = ui.state("out", "")
ui.command { id = "read", run = function()
    for _, c in ui.controls() do
        if c.role == "menu" then out:set(`{tostring(c.at_start)}/{tostring(c.at_end)}/{tostring(c.scroll_range ~= nil)}`) end
    end
end }
return ui.column { w = 300, h = 300,
    ui.menu { open = open, anchor = ui.button { "Open" }, max_h = 60, table.unpack(items) },
    ui.text { id = "out", function() return out:get() end },
    ui.text { id = "pop", function()
        local p = ui.window.popups:get()[1]
        return p and `{p.kind}/{tostring(p.at_start)}/{tostring(p.at_end)}` or "none"
    end },
}"##,
    );
    h.settle();
    h.frames(4);
    h.host.window_state(&h.ui);
    h.frames(3);
    h.host.run_command("read");
    h.frames(2);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("out"), "true/false/true");
    assert_eq!(h.text("pop"), "menu/true/false");
}

#[test]
fn a_menu_item_with_a_checkable_role_takes_its_checked() {
    let mut h = Harness::inline(
        r##"
local on = ui.state("on", true)
return ui.column { w = 300, h = 200,
    ui.menu { id = "m", open = ui.state("m", true), anchor = ui.button { "Open" },
        ui.menu_item { id = "sw", role = "menuitemcheckbox", checked = on, "Notifications" } },
}"##,
    );
    h.settle();
    h.frames(3);
    assert_eq!(h.error(), None);
    let item = h.find("sw").unwrap();
    let semantic = h.ui.semantics.borrow().get(item).cloned().unwrap();
    assert_eq!(semantic.role, zgx::semantics::Role::MenuItem);
    assert_eq!(semantic.checked, Some(true));
}

#[test]
fn a_textarea_atom_is_one_unit_and_the_script_hears_it_go() {
    let mut h = Harness::inline(
        r##"
local draft = ui.state("draft", "see  now")
local atoms = ui.state("atoms", {})
local heard = ui.state("heard", "")
local box = ui.ref()
ui.command { id = "add", run = function()
    box:get():set_selection(4, 4)
    box:get():insert_atom("@file.pdf", { id = "file-1", data = { path = "a/b.pdf" } })
end }
return ui.column { w = 400, h = 200,
    ui.textarea { ref = box, id = "ta", value = draft, atoms = atoms, w = 300, h = 80,
        atom_style = { bg = "#3366ff40" },
        on_atoms = function(change)
            local gone = {}
            for _, a in change.removed do table.insert(gone, `{a.id}:{a.from}-{a.to}:{a.label}`) end
            heard:set(change.reason .. "|" .. table.concat(gone, ","))
        end },
    ui.text { id = "n", function() return tostring(#atoms:get()) end },
    ui.text { id = "info", function()
        local a = atoms:get()[1]
        return a and `{a.id}:{a.from}-{a.to}:{a.data.path}` or "none"
    end },
    ui.text { id = "heard", function() return heard:get() end },
}"##,
    );
    h.settle();
    h.frames(3);
    h.host.run_command("add");
    h.frames(3);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("n"), "1");
    assert_eq!(h.text("info"), "file-1:4-13:a/b.pdf");
    assert_eq!(h.text("ta"), "see @file.pdf now");
    let ta = h.find("ta").unwrap();
    h.ui.input.focus(&h.ui.scene, Some(ta));
    h.ui.dispatch(zgx::input::InputEvent::KeyDown {
        key: zgx::input::Key::Backspace,
        modifiers: Default::default(),
        repeat: false,
    });
    h.frames(3);
    assert_eq!(h.text("n"), "0");
    assert_eq!(h.text("heard"), "edit|file-1:4-13:@file.pdf");
}

#[test]
fn a_textarea_restores_atoms_from_a_draft_and_undo_brings_one_back() {
    use zgx_script::Value as V;
    let mut h = Harness::inline(
        r##"
local draft = ui.state("draft", "ask @a.pdf and /skill now")
local atoms = ui.state("atoms", {})
local heard = ui.state("heard", "")
local box = ui.ref()
ui.command { id = "read", run = function()
    local list = box:get():atoms()
    heard:set(`{#list}:{list[1].id}:{list[1].from}-{list[1].to}:{list[2].id}:{list[2].from}-{list[2].to}`)
end }
return ui.column { w = 400, h = 200,
    ui.textarea { ref = box, id = "ta", value = draft, atoms = atoms, w = 300, h = 80 },
    ui.text { id = "heard", function() return heard:get() end },
}"##,
    );
    h.settle();
    h.frames(3);
    let atom = |id: &str, from: f64, to: f64| {
        V::record([
            ("id", V::from(id)),
            ("from", V::Num(from)),
            ("to", V::Num(to)),
        ])
    };
    h.host
        .set_state(
            "atoms",
            V::List(vec![atom("file", 4., 10.), atom("skill", 15., 21.)].into()),
        )
        .unwrap();
    h.frames(3);
    h.host.run_command("read");
    h.frames(2);
    assert_eq!(h.text("heard"), "2:file:4-10:skill:15-21");
    // One Backspace takes the skill whole (the caret at its end), undo gives it back.
    let ta = h.find("ta").unwrap();
    h.ui.input.focus(&h.ui.scene, Some(ta));
    let editor = h.ui.editor(ta).unwrap();
    editor.set_selection(25, 25);
    assert_eq!(editor.atoms().len(), 2);
    editor.set_selection(21, 21);
    h.ui.dispatch(zgx::input::InputEvent::KeyDown {
        key: zgx::input::Key::Backspace,
        modifiers: Default::default(),
        repeat: false,
    });
    h.frames(3);
    assert_eq!(editor.atoms().len(), 1);
    assert!(editor.undo());
    h.frames(3);
    assert_eq!(editor.atoms().len(), 2);
}

#[test]
fn level_and_current_reach_the_tree_and_selected_prints() {
    use zgx_script::Value as V;
    let mut h = Harness::inline(
        r##"local page = ui.state("page", "page")
return ui.column { w = 300, h = 200,
    ui.column { id = "row", role = "treeitem", label = "Deep", level = 3, selected = true, current = page, ui.text { "Deep" } },
}"##,
    );
    h.settle();
    h.frames(3);
    let row = h.find("row").unwrap();
    let read = |h: &Harness| {
        h.ui.semantics
            .borrow()
            .get(row)
            .map(|s| (s.role, s.level, s.current, s.selected))
    };
    assert_eq!(
        read(&h),
        Some((
            zgx::semantics::Role::TreeItem,
            Some(3),
            Some(zgx::semantics::Current::Page),
            Some(true)
        ))
    );
    let tree = h.tree();
    assert!(tree.contains("selected"), "{tree}");
    assert!(tree.contains("current"), "{tree}");
    assert!(tree.contains("level=3"), "{tree}");
    h.host.set_state("page", V::Str("".into())).unwrap();
    h.frames(3);
    assert_eq!(read(&h).unwrap().2, None);
}

#[test]
fn dim_false_keeps_a_disabled_button_undimmed_but_disabled() {
    let mut h = Harness::inline(
        r##"return ui.column { w = 300, h = 200,
    ui.button { id = "plain", disabled = true, "Plain" },
    ui.button { id = "kept", disabled = true, dim = false, "Kept" },
}"##,
    );
    h.settle();
    h.frames(3);
    assert!(h.opacity("plain") < 0.9, "{}", h.opacity("plain"));
    assert_eq!(h.opacity("kept"), 1.);
    let kept = h.find("kept").unwrap();
    assert!(h.ui.semantics.borrow().get(kept).unwrap().disabled);
}

#[test]
fn a_blur_tweened_to_zero_rests_unblurred() {
    let mut h = Harness::inline(
        r##"local shown = ui.state("shown", false)
ui.on_mount(function() shown:set(true) end)
return ui.column { w = 200, h = 100,
    ui.text { id = "t", "Hello", blur = ui.tween(shown, { from = 3, to = 0, duration = 200 }) },
}"##,
    );
    h.settle();
    h.frames(30);
    let t = h.find("t").unwrap();
    let effects = h.ui.scene.borrow().effects(t);
    assert_eq!(effects.filter_blur, 0., "{effects:?}");
    assert_eq!(effects.blur_radius, 0., "{effects:?}");
}
