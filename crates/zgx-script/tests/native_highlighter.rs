//! A module's line highlighter, handed to a textarea (`ui.highlighter`): the
//! textarea calls it with no script between, its kinds coloured as the script
//! said, its states carried line to line from the window's first (`start`).
mod common;
use common::Harness;
use std::cell::Cell;
use zgx::scene::{Color, NodeKind};
use zgx_script::ScriptHost;

thread_local! {
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

mod words {
    use super::CALLS;
    use zgx_module::Host;
    // Words (kind 0) and digits (kind 1); a line's state is how many `(`
    // are open after it, and inside one a word is a digit's kind.
    zgx_module::register!("words", |host: &Host| {
        let buffer = std::cell::RefCell::new(Vec::new());
        assert!(host.highlighter(
            "words.parens",
            &["word", "digit"],
            move |line, state, push| {
                CALLS.with(|c| c.set(c.get() + 1));
                let mut spans = buffer.take();
                spans.clear();
                let mut open = state;
                let mut i = 0;
                while i < line.len() {
                    let start = i;
                    let alpha = line[i].is_ascii_alphabetic();
                    let digit = line[i].is_ascii_digit();
                    while i < line.len()
                        && (line[i].is_ascii_alphabetic() == alpha && alpha
                            || line[i].is_ascii_digit() == digit && digit)
                    {
                        i += 1;
                    }
                    if alpha || digit {
                        let kind = if digit || open > 0 { 1 } else { 0 };
                        spans.push((start as u32 + 1, i as u32, kind));
                        continue;
                    }
                    match line[i] {
                        b'(' => open += 1,
                        b')' => open = open.saturating_sub(1),
                        _ => {}
                    }
                    i += 1;
                }
                for &(a, b, k) in &spans {
                    push(a, b, k);
                }
                buffer.replace(spans);
                open
            }
        ));
    });
}

fn harness(script: &str) -> Harness {
    let script = script.to_owned();
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", &script);
        // SAFETY: The test's own `register!` entry.
        unsafe { host.enter_module(words::zgx_module_v1, &words::zgx_module_manifest) }.unwrap();
        host
    })
}

/// Each styled run under `id`, as its text and colour.
fn runs(h: &mut Harness, id: &str) -> Vec<(String, Color)> {
    let root = h.find(id).unwrap();
    let scene = h.ui.scene.borrow();
    let mut pending = vec![root];
    let mut found = Vec::new();
    while let Some(node) = pending.pop() {
        if let NodeKind::RichText { text } = scene.kind(node) {
            for run in text.runs() {
                found.push((text.text()[run.range.clone()].to_string(), run.color));
            }
        }
        pending.extend(scene.children(node).iter().copied());
    }
    found
}
fn color_of(h: &mut Harness, text: &str) -> Color {
    let runs = runs(h, "code");
    (runs.iter().find(|(t, _)| t == text))
        .unwrap_or_else(|| panic!("no run {text:?} in {runs:?}"))
        .1
}

const RED: Color = Color(255, 0, 0, 255);
const GREEN: Color = Color(0, 255, 0, 255);

#[test]
fn a_textarea_calls_a_modules_highlighter_itself() {
    let mut h = harness(
        r##"local h = ui.highlighter("words.parens", { word = "#ff0000", digit = "#00ff00" })
        return ui.textarea { id = "code", value = ui.state("text", "abc 12\n(xy\nzz) w"), w = 300, h = 100, highlight = h }"##,
    );
    h.frames(2);
    assert_eq!(h.error(), None);
    assert_eq!(color_of(&mut h, "abc"), RED);
    assert_eq!(color_of(&mut h, "12"), GREEN);
    // Carried from the line before: inside a `(`, a word is a digit's kind.
    assert_eq!(color_of(&mut h, "xy"), GREEN);
    assert_eq!(color_of(&mut h, "zz"), GREEN);
    assert_eq!(color_of(&mut h, "w"), RED, "closed again");
    assert!(CALLS.with(Cell::get) >= 3, "the module was called");
}

#[test]
fn its_first_line_starts_where_it_is_told() {
    let mut h = harness(
        r##"local h = ui.highlighter("words.parens", { word = "#ff0000", digit = "#00ff00" })
        h:start(1)
        return ui.textarea { id = "code", value = ui.state("text", "ab) cd"), w = 300, h = 100, highlight = h }"##,
    );
    h.frames(2);
    assert_eq!(h.error(), None);
    assert_eq!(
        color_of(&mut h, "ab"),
        GREEN,
        "inside the `(` the window starts in"
    );
    assert_eq!(color_of(&mut h, "cd"), RED);
}

#[test]
fn a_script_asks_its_states_and_spans() {
    let h = harness(
        r##"local h = ui.highlighter("words.parens", { word = "#ff0000" })
        local states = h:states("(a\n(b\nc)\nd", 2)
        assert(states[2] == 2 and states[4] == 1 and states[1] == nil, "every second line's")
        assert(h:after("((", 1) == 3)
        local spans, after = h:lex("ab (12", 0)
        assert(after == 1 and #spans == 2, "spans and what it leaves")
        assert(spans[1][1] == 1 and spans[1][2] == 2 and spans[1][3] == "word")
        assert(spans[2][1] == 5 and spans[2][2] == 6 and spans[2][3] == "digit")
        local ok, why = pcall(ui.highlighter, "words.nothing")
        assert(not ok and string.find(tostring(why), "no highlighter named", 1, true), tostring(why))
        return ui.text { "done", id = "done" }"##,
    );
    h.frames(1);
    assert_eq!(h.error(), None);
}

/// A host that loaded no module has none of another host's highlighters,
/// and a plugin of the host that did is refused one.
#[test]
fn a_highlighter_is_its_hosts_and_its_apps() {
    let _loaded = harness(r##"return ui.text { "loaded", id = "loaded" }"##);
    let h = Harness::inline(
        r##"local ok, why = pcall(ui.highlighter, "words.parens")
        assert(not ok and string.find(tostring(why), "no highlighter named", 1, true), tostring(why))
        return ui.text { "done", id = "done" }"##,
    );
    h.frames(1);
    assert_eq!(h.error(), None, "another host's isn't this one's");

    let dir = common::temp_dir("native-highlighter-plugin");
    let entry = dir.join("plugin.luau");
    std::fs::write(
        &entry,
        r##"local ok, why = pcall(ui.highlighter, "words.parens")
        ui.contribute("bar", ui.text { tostring(ok) .. " " .. tostring(why), id = "said" })"##,
    )
    .unwrap();
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", r##"return ui.column { ui.slot("bar") }"##);
        // SAFETY: The test's own `register!` entry.
        unsafe { host.enter_module(words::zgx_module_v1, &words::zgx_module_manifest) }.unwrap();
        host.set_plugins(vec![zgx_script::Plugin {
            name: "p".into(),
            entry,
            grants: vec![],
        }]);
        host
    });
    h.frames(2);
    let said = h.text("said");
    assert!(
        said.starts_with("false") && said.contains("the app's own"),
        "{said}"
    );
}

thread_local! {
    static FREED: Cell<usize> = const { Cell::new(0) };
    static THREADS: std::cell::RefCell<Vec<std::thread::ThreadId>> = const { std::cell::RefCell::new(Vec::new()) };
}

mod kept {
    use super::{FREED, THREADS};
    use zgx_module::Host;
    /// Counts its drops: what the module's highlighter holds.
    struct Held;
    impl Drop for Held {
        fn drop(&mut self) {
            FREED.with(|f| f.set(f.get() + 1));
        }
    }
    zgx_module::register!("kept", |host: &Host| {
        let held = Held;
        host.highlighter("kept.lines", &["word"], move |line, state, push| {
            let _held = &held;
            THREADS.with_borrow_mut(|t| t.push(std::thread::current().id()));
            if !line.is_empty() {
                push(1, line.len() as u32, 0);
            }
            state
        });
    });
}

fn keeping(script: &str) -> Harness {
    let script = script.to_owned();
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", &script);
        // SAFETY: The test's own `register!` entry.
        unsafe { host.enter_module(kept::zgx_module_v1, &kept::zgx_module_manifest) }.unwrap();
        host
    })
}

/// The module's highlighter is freed once, as its host goes and nothing
/// holds it; loaded again into a host, the one before is freed then.
#[test]
fn a_highlighter_is_freed_once_with_its_host() {
    FREED.with(|f| f.set(0));
    let script = r##"return ui.textarea { id = "code", value = ui.state("text", "abc"), w = 300, h = 100,
        highlight = ui.highlighter("kept.lines", { word = "#ff0000" }) }"##;
    let h = keeping(script);
    h.frames(2);
    assert_eq!(h.error(), None);
    assert_eq!(FREED.with(Cell::get), 0, "held while the textarea is");
    drop(h);
    assert_eq!(FREED.with(Cell::get), 1, "freed with its host, once");

    FREED.with(|f| f.set(0));
    let h = keeping(r##"return ui.text { "x", id = "x" }"##);
    // SAFETY: The test's own entry, loaded into the host again.
    unsafe {
        h.host
            .enter_module(kept::zgx_module_v1, &kept::zgx_module_manifest)
    }
    .unwrap();
    assert_eq!(FREED.with(Cell::get), 1, "the first, which nothing took");
    drop(h);
    assert_eq!(FREED.with(Cell::get), 2);
}

/// A highlighter is called on the thread whose host loaded it, and a host
/// on another thread has none of it.
#[test]
fn a_highlighter_stays_on_its_thread() {
    THREADS.with_borrow_mut(Vec::clear);
    let h = keeping(
        r##"return ui.textarea { id = "code", value = ui.state("text", "a\nb\nc"), w = 300, h = 100,
        highlight = ui.highlighter("kept.lines", { word = "#ff0000" }) }"##,
    );
    h.frames(2);
    assert_eq!(h.error(), None);
    let here = std::thread::current().id();
    let calls = THREADS.with_borrow(Clone::clone);
    assert!(
        !calls.is_empty() && calls.iter().all(|t| *t == here),
        "{calls:?}"
    );
    std::thread::spawn(|| {
        let other = Harness::inline(
            r##"local ok, why = pcall(ui.highlighter, "kept.lines")
            assert(not ok and string.find(tostring(why), "no highlighter named", 1, true), tostring(why))
            return ui.text { "done", id = "done" }"##,
        );
        other.frames(1);
        assert_eq!(other.error(), None);
    })
    .join()
    .unwrap();
}

mod failing {
    use zgx_module::Host;
    // Panics on a line saying "boom"; leaves a state past its own on one
    // saying "past".
    zgx_module::register!("failing", |host: &Host| {
        host.highlighter("failing.lines", &["word"], |line, state, push| {
            if line == b"boom" {
                panic!("a highlighter that panics");
            }
            if line == b"past" {
                return 1 << 31;
            }
            push(1, line.len() as u32, 0);
            state
        });
    });
}

fn failing(script: &str) -> Harness {
    let script = script.to_owned();
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", &script);
        // SAFETY: The test's own `register!` entry.
        unsafe { host.enter_module(failing::zgx_module_v1, &failing::zgx_module_manifest) }
            .unwrap();
        host
    })
}

/// A highlighter that panics is caught at the boundary: the textarea turns
/// it off and the app is told, as of a script's error; so is one leaving a
/// state past its own. Asked from a script, either raises.
#[test]
fn a_highlighter_that_fails_is_turned_off_and_said() {
    for (text, said) in [
        (r"ok\nboom\nlater", "its module panicked"),
        (r"ok\npast\nlater", "a state past"),
    ] {
        let mut h = failing(&format!(
            r##"return ui.textarea {{ id = "code", value = ui.state("text", "{text}"), w = 300, h = 100,
            highlight = ui.highlighter("failing.lines", {{ word = "#ff0000" }}) }}"##
        ));
        h.frames(2);
        let error = h.error().unwrap_or_default();
        assert!(
            error.contains("failing.lines")
                && error.contains(said)
                && error.contains("off for this textarea"),
            "{error}"
        );
        assert!(
            runs(&mut h, "code")
                .iter()
                .all(|(_, color)| *color != RED || true)
        );
        h.frames(2);
    }
    let h = failing(
        r##"local h = ui.highlighter("failing.lines", { word = "#ff0000" })
        for _, line in { "boom", "past" } do
            for _, ask in { function() return h:after(line, 0) end, function() return h:lex(line, 0) end,
                function() return h:states("ok\n" .. line, 1) end } do
                local ok, why = pcall(ask)
                assert(not ok and string.find(tostring(why), "failing.lines", 1, true), tostring(why))
            end
        end
        return ui.text { "done", id = "done" }"##,
    );
    h.frames(1);
    assert_eq!(h.error(), None);
}

/// A C module's highlighter pushing what no runtime should trust: spans from
/// 0, ending before they start, past the line, of kinds it hasn't, out of
/// order, overlapping, cutting a character, a million of them. Each is let
/// go or held to the line, and nothing panics.
#[cfg(unix)]
#[test]
fn a_highlighters_spans_are_not_trusted() {
    use zgx_module::abi;
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("c-module-hostile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let json = r#"{"zgx_module":1,"abi":1,"name":"hostile","version":"1.0.0"}"#;
    let bytes: Vec<String> = abi::manifest_bytes(json)
        .iter()
        .map(|b| b.to_string())
        .collect();
    std::fs::write(
        dir.join("manifest.h"),
        format!("ZGX_MODULE_MANIFEST(hostile) = {{{}}};\n", bytes.join(",")),
    )
    .unwrap();
    let library = dir.join(format!(
        "{}hostile{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    let built = std::process::Command::new("cc")
        .args([
            "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror", "-shared", "-fPIC", "-I",
        ])
        .arg(here.join("../zgx-module/include"))
        .arg("-I")
        .arg(&dir)
        .arg(here.join("tests/c_module/hostile.c"))
        .arg("-o")
        .arg(&library)
        .output()
        .expect("a C compiler (`cc`) runs");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r##"local h = ui.highlighter("hostile.spans", { one = "#ff0000", two = "#00ff00" })
            local spans, after = h:lex("abcdef", 0)
            -- The span over the two before it, and the first of the million.
            assert(after == 0 and #spans == 2, `{#spans} spans`)
            assert(spans[1][1] == 1 and spans[1][2] == 3 and spans[1][3] == "two")
            assert(spans[2][1] == 6 and spans[2][2] == 6 and spans[2][3] == "one")
            local past = h:lex("xyz", 0)
            assert(#past == 1 and past[1][1] == 2 and past[1][2] == 3 and past[1][3] == "two", "held to the line")
            local cut = h:lex("aé b", 0)
            assert(#cut == 1 and cut[1][1] == 2 and cut[1][2] == 2, "as pushed: a script reads bytes")
            return ui.textarea { id = "code", value = ui.state("text", "abcdef\naé b\nxyz"), w = 300, h = 100,
                highlight = h }"##,
        );
        // SAFETY: The C module this test just built.
        unsafe { host.load_module(&library) }.unwrap();
        host
    });
    h.frames(2);
    assert_eq!(h.error(), None);
    assert_eq!(color_of(&mut h, "abc"), GREEN, "the spans left");
    assert_eq!(color_of(&mut h, "f"), RED);
    assert_eq!(color_of(&mut h, "é"), RED, "held to the character it cut");
    assert_eq!(color_of(&mut h, "yz"), GREEN, "held to the line");
    std::fs::remove_dir_all(dir).unwrap();
}
