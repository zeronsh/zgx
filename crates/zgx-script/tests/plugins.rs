//! Scripts an app loads beside its root script (`set_scripts`): loaded from
//! the disk, reaching what the root script does, each within the limits
//! its host sets, and given each slot's context (one panel component, an
//! instance per pane).
mod common;
use common::Harness;
use std::path::PathBuf;
use zgx_script::{Limits, Script, ScriptHost, Value};

fn script(name: &str, code: &str) -> PathBuf {
    // A directory of each call's own: tests run in parallel in one process,
    // and two writing one name's file would load each other's code.
    static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = common::temp_dir(&format!("scripts-{n}-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(&entry, code).unwrap();
    entry
}
/// A 2×2 PNG.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 6, 0,
    0, 0, 114, 182, 13, 36, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240, 31, 132,
    25, 96, 12, 0, 71, 202, 7, 249, 103, 89, 110, 183, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
    130,
];
/// The memory limit of `bytes`, what states hold counted in, and no other.
fn memory(bytes: usize) -> Limits {
    Limits {
        memory: Some(bytes),
        count_state: true,
        ..Default::default()
    }
}

const APP: &str = r#"
return ui.column {
    ui.slot("panel:notes", { id = "left", context = { pane = "p1" } }),
    ui.slot("panel:notes", { id = "right", context = { pane = "p2" } }),
    ui.slot("bar", { id = "bar" }),
}
"#;

#[test]
fn a_panel_component_is_an_instance_per_slot_with_its_context() {
    let entry = script(
        "notes",
        r#"
local Notes = ui.component(function(props)
    return ui.text { id = `note-{props.slot.pane}`, `notes for {props.slot.pane}` }
end)
ui.contribute("panel:notes", Notes {})
"#,
    );
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.set_scripts(vec![Script::file("notes", entry)]);
        host
    });
    assert_eq!(h.text("note-p1"), "notes for p1");
    assert_eq!(h.text("note-p2"), "notes for p2");
}

#[test]
fn scripts_are_set_at_runtime_and_load_on_reload() {
    let mut h = Harness::inline(APP);
    assert!(h.find("hello").is_none());
    let entry = script(
        "late",
        r#"ui.contribute("bar", ui.text { "hello", id = "hello" })"#,
    );
    h.host.set_scripts(vec![Script::file("late", entry)]);
    h.host.reload().unwrap();
    h.settle();
    assert_eq!(h.text("hello"), "hello");
    // Taken away again.
    h.host.set_scripts(vec![]);
    h.host.reload().unwrap();
    h.settle();
    assert!(h.find("hello").is_none());
}

#[test]
fn a_script_given_as_code_loads_beside_the_root() {
    let mut h = Harness::inline(APP);
    h.host.set_scripts(vec![Script::code(
        "inline",
        r#"ui.contribute("bar", ui.text { "from code", id = "code" })"#,
    )]);
    h.host.reload().unwrap();
    h.settle();
    assert_eq!(h.text("code"), "from code");
}

#[test]
fn a_script_beside_the_root_invokes_the_apps_actions_and_reads_its_shared_state() {
    let entry = script(
        "peek",
        r#"
local seen = ui.shared("sessions", "none")
ui.invoke("send_text", "ls")
ui.contribute("bar", ui.text { function() return seen:get() end, id = "peeked" })
"#,
    );
    let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let log = sent.clone();
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.action("send_text", move |text| {
            if let Value::Str(text) = text {
                log.borrow_mut().push(text.to_string());
            }
        });
        host.shared("sessions", "all of them");
        host.set_scripts(vec![Script::file("peek", entry)]);
        host
    });
    assert_eq!(h.host.error().get(), None);
    assert_eq!(h.text("peeked"), "all of them", "the host's value first");
    assert_eq!(*sent.borrow(), vec!["ls".to_string()]);
}

#[test]
fn a_slot_from_a_script_shows_only_what_it_contributes() {
    let mine = script(
        "mine",
        r#"ui.contribute("panel:x", ui.text { "mine", id = "mine" })"#,
    );
    let other = script(
        "other",
        r#"ui.contribute("panel:x", ui.text { "other", id = "other" })"#,
    );
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"return ui.column { ui.slot("panel:x", { from = "mine" }) }"#,
        );
        host.set_scripts(vec![
            Script::file("mine", mine),
            Script::file("other", other),
        ]);
        host
    });
    assert_eq!(h.text("mine"), "mine");
    assert!(h.find("other").is_none(), "another script's is not shown");
}

#[test]
fn a_script_named_as_the_root_does_not_read_its_saved_state() {
    let entry = script(
        "app",
        r#"ui.contribute("bar", ui.text { ui.persist("token", "none"):peek(), id = "token" })"#,
    );
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"ui.persist("token", "s3cret") return ui.column { ui.slot("bar") }"#,
        );
        host.set_scripts(vec![Script::file("app", entry)]);
        host
    });
    assert_eq!(h.text("token"), "none");
}

/// One namespace of command ids for every script; on a shortcut clash the
/// root script's commands keep their keys, then the others' in load order.
#[test]
fn a_shortcut_clash_keeps_the_roots_key_then_the_first_scripts() {
    let keys = script(
        "keys",
        r#"
ui.command { id = "steal", title = "Steal", menu = "Edit", key = "Cmd+C", run = function() end }
ui.command { id = "note", title = "New Note", menu = "Notes", key = "cmd+shift+n", run = function() end }
"#,
    );
    let more = script(
        "more",
        r#"ui.command { id = "memo", title = "New Memo", menu = "Notes", key = "cmd+shift+n", run = function() end }"#,
    );
    let h = Harness::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"ui.command { id = "copy", title = "Copy", menu = "Edit", key = "cmd+c", run = function() end }
return ui.column {}"#,
        );
        host.set_scripts(vec![Script::file("keys", keys), Script::file("more", more)]);
        host
    });
    assert_eq!(h.host.error().get(), None);
    let list = h.host.commands().get();
    let key = |id: &str| {
        list.iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("no command `{id}`: {list:?}"))
            .key
            .clone()
    };
    assert_eq!(key("copy").as_deref(), Some("cmd+c"));
    assert_eq!(key("steal"), None);
    assert_eq!(key("note").as_deref(), Some("cmd+shift+n"));
    assert_eq!(key("memo"), None);
    let dropped = h.host.dropped_shortcuts();
    let steal = dropped.iter().find(|d| d.command == "steal").unwrap();
    assert_eq!(steal.script, "keys");
    assert_eq!(steal.key, "Cmd+C");
    assert_eq!(steal.reason, "taken by the root script");
    assert_eq!(steal.holder, "copy");
    let memo = dropped.iter().find(|d| d.command == "memo").unwrap();
    assert_eq!(memo.script, "more");
    assert_eq!(memo.reason, "taken by the script `keys`");
    assert_eq!(memo.holder, "note");
}

#[test]
fn a_command_id_another_script_declared_is_declared_twice() {
    let first = script(
        "first",
        r#"ui.command { id = "copy", title = "Copy", run = function() end }"#,
    );
    let second = script(
        "second",
        r#"ui.command { id = "copy", title = "Copy again", run = function() end }"#,
    );
    let h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.set_scripts(vec![
            Script::file("first", first),
            Script::file("second", second),
        ]);
        host
    });
    let error = h.host.error().get().unwrap_or_default();
    assert!(error.contains("`copy` is declared twice"), "{error}");
    let titles: Vec<String> = h
        .host
        .commands()
        .get()
        .into_iter()
        .map(|c| c.title)
        .collect();
    assert_eq!(titles, ["Copy"], "the first script's, no prefix");
}

/// A script that fails as it loads takes everything it declared with it: no
/// command of its stays in the menu bar unable to run, no window of its
/// stays declared. The root script's own commands, and the error, remain.
#[test]
fn a_script_that_fails_to_load_leaves_no_commands_behind() {
    let entry = script(
        "failing",
        r#"
ui.command { id = "note", title = "New Note", menu = "Notes", key = "cmd+shift+n", run = function() end }
ui.window { id = "notes", title = "Notes", open = ui.state("open", false), ui.text { "notes" } }
error("script bug")
"#,
    );
    let h = Harness::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"ui.command { id = "copy", title = "Copy", menu = "Edit", key = "cmd+c", run = function() end }
return ui.column {}"#,
        );
        host.set_scripts(vec![Script::file("failing", entry)]);
        host
    });
    let ids: Vec<String> = h.host.commands().get().into_iter().map(|c| c.id).collect();
    assert_eq!(ids, ["copy"], "the failed script's command stayed");
    assert!(!h.host.run_command("note"));
    assert!(h.host.run_command("copy"));
    assert!(
        h.host
            .windows()
            .get()
            .iter()
            .all(|window| !window.id.contains("notes")),
        "the failed script's window stayed"
    );
    let error = h.host.error().get().unwrap_or_default();
    assert!(error.contains("script bug"), "{error:?}");
}

#[test]
fn a_script_of_bytecode_is_read_as_source_and_never_runs() {
    // Luau checks no bytecode it loads: crafted bytecode would corrupt the
    // VM, so a script's files are source, always.
    let bytecode = mlua::chunk::Compiler::new()
        .compile(r#"ui.contribute("bar", ui.text { "ran", id = "ran" })"#)
        .unwrap();
    let entry = script("bytecode", "");
    std::fs::write(&entry, &bytecode).unwrap();
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.set_scripts(vec![Script::file("bytecode", entry)]);
        host
    });
    assert!(h.find("ran").is_none(), "the bytecode ran");
}

#[test]
fn a_script_beside_the_root_shows_remote_images() {
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let entry = script(
        "viewer",
        r#"ui.contribute("bar", ui.image { src = "https://images.test/picture?k=v", w = 10, h = 10 })"#,
    );
    let log = asked.clone();
    let h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.fetcher(move |request: zgx_script::Request| {
            log.borrow_mut().push(request.url.clone());
            async move { Err::<zgx_script::Response, String>("offline".into()) }
        });
        host.set_scripts(vec![Script::file("viewer", entry)]);
        host
    });
    h.frames(2);
    assert_eq!(asked.borrow().len(), 1, "the script's request");
}

#[test]
fn a_script_shows_a_file_outside_its_directory_picked_or_named() {
    let outside = common::temp_dir("picked");
    std::fs::create_dir_all(&outside).unwrap();
    // A red pixel.
    let png: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2,
        0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 0, 0,
        3, 1, 1, 0, 201, 254, 146, 239, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    std::fs::write(outside.join("picked.png"), png).unwrap();
    let named = outside.join("picked.png").display().to_string();
    for (name, image) in [
        ("picked", "folder:file(\"picked.png\"):await().path"),
        ("named", &*format!("{named:?}")),
    ] {
        let entry = script(
            &format!("viewer-{name}"),
            &format!(
                r#"local src = ui.state("src", nil :: string?)
ui.contribute("bar", ui.column {{
    ui.button {{ id = "go", w = 20, h = 20, "go", on_click = function()
        ui.task(function()
            local folder = ui.open_folder():await()
            if folder then src:set({image}) end
        end)
    end }},
    ui.show(function() return src:get() ~= nil end,
        ui.image {{ id = "shown", src = function() return src:get() or "" end, w = 4, h = 4 }}),
}})"#
            ),
        );
        let mut h = Harness::with_desktop(|runtime| {
            let host = ScriptHost::inline(runtime, "app", APP);
            host.set_scripts(vec![Script::file(format!("viewer-{name}"), entry)]);
            host
        });
        h.desktop.clone().unwrap().pick_folder(&outside);
        h.click("go");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while h.find("shown").is_none()
            && h.host.error().get().is_none()
            && std::time::Instant::now() < deadline
        {
            h.settle();
            h.frames(1);
        }
        h.frames(2);
        let error = h.host.error().get();
        assert!(
            h.find("shown").is_some(),
            "{name}: {error:?} {:?} {:?}",
            h.texts(),
            h.desktop.clone().unwrap().asked().len()
        );
        assert_eq!(error, None, "{name}");
    }
}

#[cfg(unix)]
#[test]
fn a_script_requires_nothing_outside_its_directory_through_a_link() {
    // A module that is a link to a file outside is outside, whatever its
    // name says.
    let outside = common::temp_dir("outside").join("outside.luau");
    std::fs::write(&outside, r#"return "outside""#).unwrap();
    let entry = script(
        "linked",
        r#"local ok, got = pcall(require, "./secret")
ui.contribute("bar", ui.text { tostring(ok) .. " " .. tostring(got), id = "got" })"#,
    );
    let link = entry.parent().unwrap().join("secret.luau");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.set_scripts(vec![Script::file("linked", entry)]);
        host
    });
    let got = h.text("got");
    assert!(got.starts_with("false"), "{got}");
    assert!(got.contains("directory or below"), "{got}");
}

#[test]
fn a_scripts_images_come_from_its_folder_or_any_absolute_path() {
    let outside = common::temp_dir("outside").join("outside.png");
    std::fs::write(&outside, PNG).unwrap();
    let entry = script(
        "pictures",
        &format!(
            r#"ui.contribute("bar", ui.row {{
    ui.image {{ src = "{}", w = 10, h = 10 }},
    ui.image {{ src = "own.png", w = 10, h = 10 }},
    ui.canvas {{ w = 10, h = 10, draw = function(c) c:image("{}", 0, 0, 10, 10) end }},
}})"#,
            outside.display(),
            outside.display()
        ),
    );
    std::fs::write(entry.parent().unwrap().join("own.png"), PNG).unwrap();
    let h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.set_scripts(vec![Script::file("pictures", entry)]);
        host
    });
    h.frames(2);
    assert_eq!(h.error(), None);
}

#[test]
fn a_script_beside_the_root_opens_windows_and_reads_the_title_bar() {
    let entry = script(
        "inspector",
        r#"
local ok = pcall(ui.window, {
    id = "w", open = ui.state("open", true), ui.text { "window" },
})
ui.contribute("bar", ui.text {
    tostring(ok) .. " " .. tostring(ui.window.titlebar_area == nil), id = "got",
})
"#,
    );
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.set_scripts(vec![Script::file("inspector", entry)]);
        host
    });
    assert_eq!(h.text("got"), "true false");
}

/// A script that holds `mb` strings of 1 MB, saying how many it made.
fn holding(name: &str, mb: usize) -> PathBuf {
    script(
        name,
        &format!(
            r#"
local held, made = table.create({mb}), 0
for i = 1, {mb} do
    if not pcall(function() held[i] = string.rep("x", 1048576) .. i end) then break end
    made = i
end
ui.contribute("bar", ui.text {{ id = "{name}", tostring(made) }})
"#
        ),
    )
}

#[test]
fn each_script_holds_what_its_own_memory_limit_lets_it() {
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.set_scripts(vec![
            Script::file("big", holding("big", 48)).limits(memory(64 << 20)),
            Script::file("small", holding("small", 48)).limits(memory(32 << 20)),
            Script::file("free", holding("free", 48)),
        ]);
        host
    });
    assert_eq!(h.error(), None);
    // 48 MB fits the 64 and no limit; the other stops short of its 32.
    assert_eq!(h.text("big"), "48");
    assert_eq!(h.text("free"), "48");
    let small: usize = h.text("small").parse().unwrap();
    assert!(small < 32, "{small}");
}

#[test]
fn a_scripts_decoded_pictures_count_against_its_memory_limit() {
    // A `ui.decode_image` picture keeps its file's bytes in Rust, outside
    // the VM: counted as a state's value is, a script keeping picture after
    // picture (6 MB each) stops at its 32 MB.
    use base64::Engine as _;
    // The 2×2 PNG with a chunk of its own of 6 MB (ancillary: read past,
    // and kept with the file) after its header.
    let crc = |bytes: &[u8]| {
        let mut crc = !0u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & (!(crc & 1)).wrapping_add(1));
            }
        }
        !crc
    };
    let mut chunk = b"zgXz".to_vec();
    chunk.resize(4 + (6 << 20), 0);
    let mut png = PNG[..33].to_vec();
    png.extend_from_slice(&((chunk.len() - 4) as u32).to_be_bytes());
    png.extend_from_slice(&chunk);
    png.extend_from_slice(&crc(&chunk).to_be_bytes());
    png.extend_from_slice(&PNG[33..]);
    let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
    let entry = script(
        "pictures",
        &format!(
            r#"
local kept, made = {{}}, ui.state("made", 0)
local encoded = "{encoded}"
local _ = ui.task(function()
    for i = 1, 8 do
        kept[i] = ui.decode_image(encoded):await()
        made:set(i)
    end
end)
ui.contribute("bar", ui.text {{ id = "made", function() return tostring(made:get()) end }})
"#
        ),
    );
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.set_scripts(vec![
            Script::file("pictures", entry).limits(memory(32 << 20)),
        ]);
        host
    });
    // The picture past its room is refused: the task stops there.
    h.wait_until(|h| h.error().is_some());
    let error = h.error().unwrap_or_default();
    assert!(
        error.contains("the script's memory is 32 MB at most"),
        "{error}"
    );
    let made: usize = h.text("made").parse().unwrap();
    assert!(
        (1..5).contains(&made),
        "{made} pictures of 6 MB kept in 32 MB"
    );
}

#[test]
fn a_scripts_state_counts_against_its_memory_limit() {
    // A state's value lives in Rust, outside the VM: without counting it,
    // a script making state after state held as much as it liked.
    let entry = script(
        "hoarder",
        r#"
-- Everything made before memory runs short (the keys, the result's
-- state), and the errors kept, not formatted, until there is room again.
local keys = table.create(1000)
for i = 1, 1000 do
    keys[i] = "k" .. i
end
local result = ui.state("result", "")
ui.contribute("bar", ui.text { id = "got", "{result}" })
local made, made_error = 0, nil
for i = 1, 1000 do
    local ok, e = pcall(function() ui.state(keys[i], string.rep("x", 100000) .. i) end)
    if not ok then made_error = e break end
    made = i
end
local pushed, push_error = 0, nil
local list = ui.state("list", {})
for i = 1, 1000 do
    local ok, e = pcall(function() list:push(string.rep("y", 100000) .. i) end)
    if not ok then push_error = e break end
    pushed = i
end
-- The room given back: what the states held let go (written over).
for i = 1, made do
    ui.state(keys[i], 0):set(0)
end
list:set({})
-- One state written over and over holds one value.
local again = ui.state("again", 0)
local rewrites = 0
for i = 1, 200 do
    if pcall(function() again:set(string.rep("z", 10000) .. i) end) then
        rewrites = i
    end
end
local function memory(e: any): boolean
    return e ~= nil and string.find(tostring(e), "memory", 1, true) ~= nil
end
result:set(`{made} {memory(made_error)} {pushed} {memory(push_error)} {rewrites}`)
"#,
    );
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.set_scripts(vec![Script::file("hoarder", entry).limits(memory(8 << 20))]);
        host
    });
    assert_eq!(h.error(), None);
    let got = h.text("got");
    let parts: Vec<&str> = got.split(' ').collect();
    let made: usize = parts[0].parse().unwrap();
    let pushed: usize = parts[2].parse().unwrap();
    // 8 MB is some 80 strings of 100 KB, less what the VM holds.
    assert!((20..90).contains(&made), "{got}");
    assert_eq!(parts[1], "true", "{got}");
    // The first loop's states still hold theirs: little room is left.
    assert!(pushed < 20, "{got}");
    assert_eq!(parts[3], "true", "{got}");
    assert_eq!(parts[4], "200", "{got}");
}

#[test]
fn a_bitmap_kept_in_state_counts_its_pixels() {
    // Its pixels live in Rust too: a pasted 4K screenshot is 33 MB.
    let entry = script(
        "painter",
        r#"
local result = ui.state("result", "")
ui.contribute("bar", ui.text { id = "got", "{result}" })
local small = pcall(function() ui.state("small", ui.bitmap(8, 8, function() return 1 end)) end)
local big = ui.bitmap(1024, 1024, function() return 1 end)
local kept, e = pcall(function() ui.state("big", big) end)
local short = e ~= nil and string.find(tostring(e), "memory", 1, true) ~= nil
result:set(`{small} {kept} {short}`)
"#,
    );
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.set_scripts(vec![Script::file("painter", entry).limits(memory(4 << 20))]);
        host
    });
    assert_eq!(h.error(), None);
    // 4 MB of pixels is more than the 4 MB limit leaves.
    assert_eq!(h.text("got"), "true false true");
}

#[test]
fn a_script_beside_the_root_hears_what_the_system_opens() {
    let entry = script(
        "opener",
        r#"
local ok = pcall(ui.on_open, function() end)
ui.contribute("bar", ui.text { `{ok} {#ui.args}`, id = "got" })
"#,
    );
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP).args(["file.txt"]);
        host.set_scripts(vec![Script::file("opener", entry)]);
        host
    });
    assert_eq!(h.text("got"), "true 1");
}

#[test]
fn a_script_beside_the_root_reserves_a_shortcut() {
    let entry = script(
        "reserver",
        r#"ui.command { id = "quit", title = "Quit", menu = "File", key = "cmd+k", reserved = true, run = function() end }"#,
    );
    let h = Harness::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"ui.command { id = "tab", title = "Tab", menu = "Shell", key = "cmd+t", reserved = true, run = function() end }
return ui.column {}"#,
        );
        host.set_scripts(vec![Script::file("reserver", entry)]);
        host
    });
    assert_eq!(h.host.error().get(), None);
    let mut reserved = h.host.reserved_keys().get();
    reserved.sort();
    assert_eq!(reserved, ["cmd+k", "cmd+t"]);
}
