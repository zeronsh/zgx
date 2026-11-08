//! The machine beyond the window (`ui.clipboard`, `ui.open_files`,
//! `ui.save_file`, `ui.read_file`, `ui.write_file`) through a platform the
//! window provides: here a fake, as a desktop window provides the real one.
mod common;
use common::Harness;
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
use zgx::platform::{FilePicker, Pending, Platform, PlatformService};
use zgx_script::ScriptHost;

struct Fake {
    clipboard: RefCell<String>,
    image: RefCell<Option<std::sync::Arc<zgx::image::ImageData>>>,
    /// What the open dialog picks, and where the save dialog saves.
    opens: PathBuf,
    saves: PathBuf,
}
impl Platform for Fake {
    fn clipboard_text(&self) -> Option<String> {
        Some(self.clipboard.borrow().clone())
    }
    fn set_clipboard_text(&self, text: String) -> bool {
        *self.clipboard.borrow_mut() = text;
        true
    }
    fn clipboard_image(&self) -> Pending<Option<zgx::image::ImageData>> {
        // A copy sharing the pixels.
        let image = (self.image.borrow().as_ref())
            .map(|image| image.transformed(zgx::affine::Affine::IDENTITY));
        Box::pin(async move { image })
    }
    fn set_clipboard_image(&self, image: std::sync::Arc<zgx::image::ImageData>) -> Pending<bool> {
        *self.image.borrow_mut() = Some(image);
        Box::pin(async { true })
    }
    fn open_files(&self, _: FilePicker) -> Pending<Result<Vec<PathBuf>, String>> {
        let path = self.opens.clone();
        Box::pin(async move { Ok(vec![path]) })
    }
    fn save_file(&self, _: FilePicker) -> Pending<Result<Option<PathBuf>, String>> {
        let path = self.saves.clone();
        Box::pin(async move { Ok(Some(path)) })
    }
}

const APP: &str = r#"
local shown = ui.state("shown", "")
local kept = ui.state("kept", nil)
return ui.column {
    ui.text { id = "label", function() return shown:get() end },
    ui.button { id = "copy", w = 40, h = 20, "c", on_click = function()
        ui.clipboard.write("copied text")
        shown:set(ui.clipboard.read() or "")
    end },
    ui.button { id = "image", w = 40, h = 20, "i", on_click = function()
        ui.task(function()
            local before = ui.clipboard.read_image():await()
            local mask = ui.bitmap(3, 2, function(x, y) return x / 2 end)
            local written = ui.clipboard.write_image(mask):await()
            -- Kept in state, as a pasted image to show is.
            kept:set(ui.clipboard.read_image():await())
            shown:set(`{before == nil} {written} {kept:get().w}x{kept:get().h}`)
        end)
    end },
    ui.button { id = "open", w = 40, h = 20, "o", on_click = function()
        ui.task(function()
            local files = ui.open_files({ filters = { { "Text", { "txt" } } } }):await()
            shown:set(`{files[1].name}: {ui.read_file(files[1]):await()}`)
        end)
    end },
    ui.button { id = "save", w = 40, h = 20, "s", on_click = function()
        ui.task(function()
            local file = ui.save_file({ name = "out.txt" }):await()
            ui.write_file(file, "saved text"):await()
            shown:set(`wrote {file.name}`)
        end)
    end },
    ui.button { id = "sneak", w = 40, h = 20, "x", on_click = function()
        ui.read_file("/etc/hosts")
    end },
}
"#;

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("zgx-io-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn app(dir: &std::path::Path, code: &'static str, plugin: Option<&'static str>) -> Harness {
    let fake = Rc::new(Fake {
        clipboard: RefCell::new(String::new()),
        image: RefCell::new(None),
        opens: dir.join("in.txt"),
        saves: dir.join("out.txt"),
    });
    Harness::wrapped(
        move |runtime| {
            let host = ScriptHost::inline(runtime, "app", code);
            match plugin {
                Some(code) => host.plugin("peek.luau", code),
                None => host,
            }
        },
        move |view| zgx::compose::provide(PlatformService(fake), view),
    )
}
fn wait_for(h: &mut Harness, text: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        h.settle();
        h.ui.prepare_frame();
        if h.text("label") == text {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!(
        "label reads {:?}, not {text:?} ({:?})",
        h.text("label"),
        h.host.error().get()
    );
}

#[test]
fn the_clipboard_takes_and_gives_text() {
    let dir = dir("clipboard");
    let mut h = app(&dir, APP, None);
    h.click("copy");
    wait_for(&mut h, "copied text");
}

#[test]
fn the_clipboard_takes_and_gives_images() {
    let dir = dir("clipboard-image");
    let mut h = app(&dir, APP, None);
    h.click("image");
    wait_for(&mut h, "true true 3x2");
}

#[test]
fn a_picked_file_reads_and_a_saved_one_is_written() {
    let dir = dir("files");
    std::fs::write(dir.join("in.txt"), "hello from disk").unwrap();
    let mut h = app(&dir, APP, None);
    h.click("open");
    wait_for(&mut h, "in.txt: hello from disk");
    h.click("save");
    wait_for(&mut h, "wrote out.txt");
    assert_eq!(
        std::fs::read_to_string(dir.join("out.txt")).unwrap(),
        "saved text"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// A dialog the person never answers, telling when the host drops it.
struct Waiting(Rc<std::cell::Cell<bool>>);
impl Drop for Waiting {
    fn drop(&mut self) {
        self.0.set(true);
    }
}
struct Unanswered(Rc<std::cell::Cell<bool>>);
impl Platform for Unanswered {
    fn clipboard_text(&self) -> Option<String> {
        None
    }
    fn set_clipboard_text(&self, _: String) -> bool {
        false
    }
    fn open_files(&self, _: FilePicker) -> Pending<Result<Vec<PathBuf>, String>> {
        let waiting = Waiting(self.0.clone());
        Box::pin(async move {
            let _waiting = waiting;
            std::future::pending::<()>().await;
            Ok(Vec::new())
        })
    }
    fn save_file(&self, _: FilePicker) -> Pending<Result<Option<PathBuf>, String>> {
        Box::pin(async { Ok(None) })
    }
}

/// An app asking for a dialog its person never answers, by a promise
/// (`ask`, `cancel`) or a task awaiting one (`ask_task`, `cancel_task`).
fn unanswered(dropped: Rc<std::cell::Cell<bool>>) -> Harness {
    let platform = Rc::new(Unanswered(dropped));
    Harness::wrapped(
        |runtime| {
            ScriptHost::inline(
                runtime,
                "app",
                r#"local shown = ui.state("shown", "")
                local picking = nil
                local running = nil
                return ui.column {
                    ui.text { id = "label", function() return shown:get() end },
                    ui.button { id = "ask", w = 40, h = 20, "a", on_click = function()
                        picking = ui.open_files({})
                        shown:set("asked")
                    end },
                    ui.button { id = "cancel", w = 40, h = 20, "c", on_click = function()
                        picking:cancel()
                        shown:set("cancelled")
                    end },
                    ui.button { id = "ask_task", w = 40, h = 20, "t", on_click = function()
                        running = ui.task(function()
                            ui.open_files({}):await()
                            shown:set("answered")
                        end)
                        shown:set("asked")
                    end },
                    ui.button { id = "cancel_task", w = 40, h = 20, "k", on_click = function()
                        running:cancel()
                        shown:set("cancelled")
                    end },
                }"#,
            )
        },
        move |view| zgx::compose::provide(PlatformService(platform), view),
    )
}

#[test]
fn a_dialog_the_script_cancels_is_dropped_where_it_waits() {
    // A dropped request closes Linux's portal picker.
    let dropped = Rc::new(std::cell::Cell::new(false));
    let mut h = unanswered(dropped.clone());
    h.frames(1);
    h.click("ask");
    wait_for(&mut h, "asked");
    h.frames(2);
    assert!(!dropped.get(), "waiting on the person");
    h.click("cancel");
    wait_for(&mut h, "cancelled");
    h.frames(2);
    assert!(
        dropped.get(),
        "the dialog dropped once the script cancelled it"
    );
    assert_eq!(h.error(), None);
}

#[test]
fn a_task_cancelled_at_a_dialog_drops_the_dialog() {
    let dropped = Rc::new(std::cell::Cell::new(false));
    let mut h = unanswered(dropped.clone());
    h.frames(1);
    h.click("ask_task");
    wait_for(&mut h, "asked");
    h.frames(2);
    assert!(!dropped.get(), "waiting on the person");
    h.click("cancel_task");
    wait_for(&mut h, "cancelled");
    h.frames(2);
    assert!(dropped.get(), "the dialog the task awaited dropped with it");
    assert_eq!(h.text("label"), "cancelled", "the task never went on");
    assert_eq!(h.error(), None);
}

#[test]
fn a_file_the_user_did_not_pick_is_out_of_reach() {
    let dir = dir("sneak");
    let mut h = app(&dir, APP, None);
    h.click("sneak");
    let error = h.host.error().get().unwrap_or_default();
    assert!(error.contains("only files the user picked"), "{error}");
}

#[test]
fn a_plugin_needs_its_hosts_grant() {
    let dir = dir("plugin");
    let h = app(
        &dir,
        r#"return ui.text { "app", id = "label" }"#,
        Some(r#"ui.clipboard.read()"#),
    );
    let error = h.host.error().get().unwrap_or_default();
    assert!(error.contains("Capability::Clipboard"), "{error}");
    let h = app(
        &dir,
        r#"return ui.text { "app", id = "label" }"#,
        Some(r#"ui.clipboard.read_image()"#),
    );
    let error = h.host.error().get().unwrap_or_default();
    assert!(error.contains("Capability::Clipboard"), "{error}");
    let h = app(
        &dir,
        r#"return ui.text { "app", id = "label" }"#,
        Some(r#"ui.open_folder()"#),
    );
    let error = h.host.error().get().unwrap_or_default();
    assert!(error.contains("Capability::Folders"), "{error}");
}

#[test]
fn a_host_can_refuse_folders_to_every_script() {
    let mut h = Harness::with_desktop(|runtime| {
        ScriptHost::inline(
            runtime,
            "app",
            r#"local said = ui.state("said", "")
            return ui.column {
                ui.button { id = "open", w = 40, h = 20, "o", on_click = function()
                    ui.task(function()
                        local ok, failure = ui.try(function() return ui.open_folder():await() end)
                        said:set(if ok then "opened" else failure.kind)
                    end)
                end },
                ui.text { id = "label", "{said}" },
            }"#,
        )
        .refuse_folders()
    });
    h.desktop.clone().unwrap().pick_folder(std::env::temp_dir());
    h.click("open");
    wait_for(&mut h, "denied");
    assert!(h.desktop.clone().unwrap().asked().is_empty(), "no dialog");
}

/// A folder kept past the run (`remember`): a later launch on the same
/// storage has it again, until it is forgotten. A host can refuse, and a
/// plugin keeps none.
#[test]
fn a_remembered_folder_comes_back_on_a_later_launch_until_forgotten() {
    const KEEPS: &str = r#"local said = ui.state("said", "")
        return ui.column {
            ui.button { id = "open", w = 40, h = 20, "o", on_click = function()
                ui.task(function()
                    local folder = ui.open_folder({ remember = "library" }):await()
                    said:set(if folder then "picked " .. folder.name else "none")
                end)
            end },
            ui.text { id = "label", "{said}" },
        }"#;
    const COMES_BACK: &str = r#"local folder
        local said = ui.state("said", "")
        ui.task(function()
            folder = ui.remembered_folder("library"):await()
            said:set(if folder then "kept " .. folder.name else "nothing")
        end)
        return ui.column {
            ui.button { id = "forget", w = 40, h = 20, "f", on_click = function()
                if folder then folder:forget() end
                said:set("forgot")
            end },
            ui.text { id = "label", "{said}" },
        }"#;
    let storage = dir("remember-storage");
    let library = dir("remember-library");
    let name = library.file_name().unwrap().to_string_lossy().into_owned();
    let launch = |code: &'static str, forgetful: bool| {
        let storage = storage.clone();
        Harness::with_desktop(move |runtime| {
            let host = ScriptHost::inline(runtime, "app", code).storage(storage);
            if forgetful {
                host.refuse_remembered_folders()
            } else {
                host
            }
        })
    };
    let mut h = launch(KEEPS, false);
    h.desktop.clone().unwrap().pick_folder(&library);
    h.click("open");
    wait_for(&mut h, &format!("picked {name}"));
    drop(h);
    // The next launch has it, unasked; a host refusing keeps nothing.
    let mut h = launch(COMES_BACK, false);
    wait_for(&mut h, &format!("kept {name}"));
    wait_for(&mut launch(COMES_BACK, true), "nothing");
    h.click("forget");
    drop(h);
    wait_for(&mut launch(COMES_BACK, false), "nothing");

    // Kept where no script reads: the storage's `.folders`, by key in hex.
    let mut h = launch(KEEPS, false);
    h.desktop.clone().unwrap().pick_folder(&library);
    h.click("open");
    wait_for(&mut h, &format!("picked {name}"));
    assert!(storage.join(".folders/6c696272617279").is_file());

    // A plugin, granted folders, keeps none.
    let h = Harness::with_desktop(|runtime| {
        ScriptHost::inline(runtime, "app", r#"return ui.text { "app", id = "label" }"#)
            .plugin("keep.luau", r#"ui.remembered_folder("library")"#)
            .grant("keep.luau", zgx_script::Capability::Folders)
    });
    let error = h.host.error().get().unwrap_or_default();
    assert!(error.contains("a plugin keeps no folders"), "{error}");
}

#[test]
fn without_a_platform_it_says_so() {
    let mut h = Harness::new(|runtime| ScriptHost::inline(runtime, "app", APP));
    h.click("copy");
    let error = h.host.error().get().unwrap_or_default();
    assert!(error.contains("need a desktop window"), "{error}");
}

/// `ui.args` is what the app was launched with; `ui.on_open` is handed the
/// files and URLs the system opens with it, the files the script's to read
/// as a dialog's are, and a launch's waiting until the script takes them.
#[test]
fn launch_arguments_and_files_the_system_opens_reach_the_script() {
    let file = std::env::temp_dir().join(format!("zgx-on-open-{}.txt", std::process::id()));
    std::fs::write(&file, "opened by the system").unwrap();
    let (opened, given) = (file.clone(), file.clone());
    let mut h = Harness::with_desktop(move |runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"local args = ui.state("args", table.concat(ui.args, " "))
            local got = ui.state("got", "")
            ui.on_open(function(e)
                local urls = table.concat(e.urls, " ")
                for _, file in e.files do
                    ui.task(function()
                        got:set(`{file.name}: {ui.read_file(file):await()} {urls}`)
                    end)
                end
                if #e.files == 0 then got:set(urls) end
            end)
            return ui.column { ui.text { "{args}", id = "args" }, ui.text { "{got}", id = "got" } }"#,
        )
        .args(["--verbose", "notes.txt"]);
        // Asked before the script loads (a launch's file): it waits.
        host.open(vec![opened], vec!["zgx://launch".into()]);
        host
    });
    assert_eq!(h.text("args"), "--verbose notes.txt");
    let name = given.file_name().unwrap().to_string_lossy().into_owned();
    assert!(h.wait_until(|h| h.text("got").starts_with(&name)));
    assert_eq!(
        h.text("got"),
        format!("{name}: opened by the system zgx://launch")
    );
    h.host.open(vec![], vec!["https://example.org/a".into()]);
    h.settle();
    assert_eq!(h.text("got"), "https://example.org/a");
    let _ = std::fs::remove_file(&file);
}

/// Files copied in a file manager reach `on_paste`, and read as what the
/// person picked in a dialog does; a plugin without the grants hears none.
#[test]
fn files_on_the_clipboard_reach_on_paste_and_can_be_read() {
    let file = std::env::temp_dir().join(format!("zgx-pasted-{}.txt", std::process::id()));
    std::fs::write(&file, "pasted file").unwrap();
    let dir = std::env::temp_dir().join(format!("zgx-paste-plugin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("plugin.luau");
    std::fs::write(
        &entry,
        r#"local heard = ui.state("heard", "not yet")
        ui.contribute("bar", ui.column {
            ui.textarea { id = "plugin", value = ui.state("text", ""), w = 100, h = 30,
                on_paste = function(e)
                    heard:set(`{#e.files} files`)
                    return true
                end },
            ui.text { id = "plugin-heard", "{heard}" },
        })"#,
    )
    .unwrap();
    let mut h = Harness::with_desktop(move |runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"local got = ui.state("got", "")
            return ui.column {
                ui.textarea { id = "box", value = ui.state("text", ""), w = 200, h = 60,
                    on_paste = function(e)
                        for _, file in e.files do
                            ui.task(function()
                                got:set(`{file.name}: {ui.read_file(file):await()}`)
                            end)
                        end
                        return #e.files > 0
                    end },
                ui.slot("bar"),
                ui.text { id = "got", "{got}" },
            }"#,
        );
        host.set_plugins(vec![zgx_script::Plugin {
            name: "paste".into(),
            entry: entry.clone(),
            grants: vec![zgx_script::Capability::Clipboard],
        }]);
        host
    });
    h.frames(1);
    let paste = |h: &mut Harness, id: &str, text: Option<&str>| {
        let node = h.find(id).unwrap();
        let editor = h.ui.editor(node).unwrap();
        h.ui.paste_into(&editor, text);
        h.settle();
        h.frames(2);
    };
    // No files: as ever, the text goes in.
    paste(&mut h, "box", Some("plain"));
    let name = h.find("box").unwrap();
    let typed =
        h.ui.editor(name)
            .unwrap()
            .editor
            .borrow()
            .text()
            .to_string();
    assert_eq!(typed, "plain");
    // Files, with their name as the text as Finder puts it: taken, and read.
    h.desktop.as_ref().unwrap().copy_files(vec![file.clone()]);
    let copied = file.file_name().unwrap().to_string_lossy().into_owned();
    paste(&mut h, "box", Some(&copied));
    assert!(
        h.wait_until(|h| !h.text("got").is_empty()),
        "{:?}",
        h.error()
    );
    assert_eq!(h.text("got"), format!("{copied}: pasted file"));
    let typed =
        h.ui.editor(name)
            .unwrap()
            .editor
            .borrow()
            .text()
            .to_string();
    assert_eq!(typed, "plain", "taken: its name doesn't go in");
    // A plugin granted the clipboard but not files hears none.
    paste(&mut h, "plugin", None);
    assert_eq!(h.text("plugin-heard"), "0 files");
    assert_eq!(h.error(), None);
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A dialog asked for as a script loads (a task at its top level, before
/// its view mounts in a window) opens once the window is there; one that
/// can't open, or is refused, reports why rather than doing nothing.
#[test]
fn a_dialog_asked_for_as_the_script_loads_opens_or_says_why_not() {
    const ASKS: &str = r#"local _ = ui.task(function()
            local files = ui.open_files():await()
            ui.shared("said", ""):set("got " .. #files)
        end)"#;
    let shown = r#"local said = ui.shared("said", "")
        return ui.text { id = "label", "{said}" }"#;
    // The app's `said`, which a plugin may write.
    let exposed = |host: ScriptHost| {
        host.expose_shared("said", None);
        host
    };
    let settled = |h: &mut Harness| {
        for _ in 0..10 {
            h.frames(1);
            h.settle();
        }
    };
    // The app's own, and a plugin's granted files: the dialog opens.
    for plugin in [false, true] {
        let mut h = Harness::with_desktop(move |runtime| {
            let app = format!("{}\n{shown}", if plugin { "" } else { ASKS });
            let host = exposed(ScriptHost::inline(runtime, "app", &app));
            match plugin {
                true => host
                    .plugin("asks.luau", ASKS)
                    .grant("asks.luau", zgx_script::Capability::Files),
                false => host,
            }
        });
        settled(&mut h);
        assert_eq!(
            h.desktop.clone().unwrap().asked().len(),
            1,
            "plugin {plugin}"
        );
        assert_eq!(h.text("label"), "got 0", "nothing picked: cancelled");
        assert_eq!(h.host.error().get(), None);
    }
    // Without a window: the error shows.
    let mut h = Harness::new(|runtime| {
        exposed(ScriptHost::inline(
            runtime,
            "app",
            format!("{ASKS}\n{shown}"),
        ))
    });
    settled(&mut h);
    let error = h.host.error().get().unwrap_or_default();
    assert!(error.contains("no platform here"), "{error}");
    // Refused to a plugin, as it loads: the error shows, the load's success
    // doesn't clear it.
    let mut h = Harness::with_desktop(|runtime| {
        exposed(ScriptHost::inline(runtime, "app", shown)).plugin("asks.luau", ASKS)
    });
    settled(&mut h);
    let error = h.host.error().get().unwrap_or_default();
    assert!(error.contains("Capability::Files"), "{error}");
    assert!(h.desktop.clone().unwrap().asked().is_empty(), "no dialog");
}

/// A page's downloads are the script's (`on_download`): saved at a file
/// the user picked, through the save dialog (`d:ask`), refused at a path
/// the user didn't pick, or cancelled; how a saved one goes reaches its
/// callbacks.
#[test]
fn a_pages_downloads_are_the_scripts_to_save_or_not() {
    use zgx::{
        native_view::NativeContent,
        web_view::{DownloadRequest, DownloadResponder, DownloadStatus, DownloadStatusFn},
    };
    let dir = dir("downloads");
    let mut h = app(
        &dir,
        r#"
local shown = ui.state("shown", "")
local function note(s) shown:set(shown:get() .. s .. ";") end
return ui.column {
    ui.text { id = "label", function() return shown:get() end },
    ui.webview { id = "page", w = 100, h = 60, on_download = function(d)
        note(`{d.name} {d.mime}`)
        if d.name == "keep.txt" then
            ui.task(function()
                local file = ui.save_file({ name = d.name }):await()
                d:save_to(file, {
                    on_progress = function(got, total) note(`got {got}/{total}`) end,
                    on_finish = function(ok) note(`finished {ok}`) end,
                })
            end)
        elseif d.name == "ask.txt" then
            ui.task(function()
                local file = d:ask({ on_finish = function(ok, err) note(`failed {err}`) end }):await()
                note(`asked {file.name}`)
            end)
        elseif d.name == "sneak.txt" then
            note(`sneak {pcall(function() d:save_to("/etc/sneak.txt") end)}`)
            d:cancel()
        else
            d:cancel()
        end
    end },
}
"#,
        None,
    );
    h.ui.prepare_frame();
    let node = h.find("page").unwrap();
    let Some(NativeContent::Web(web)) = h.natives.content(node) else {
        panic!("no page")
    };
    let answers: Rc<RefCell<Vec<Option<PathBuf>>>> = Rc::default();
    let statuses: Rc<RefCell<Vec<DownloadStatusFn>>> = Rc::default();
    let start = |name: &str, mime: &str| {
        let (answers, statuses) = (answers.clone(), statuses.clone());
        web.download(
            DownloadRequest::new(format!("https://page.test/{name}"), name, mime),
            DownloadResponder::new(move |answer| {
                answers.borrow_mut().push(answer.map(|(path, status)| {
                    statuses.borrow_mut().push(status);
                    path
                }))
            }),
        );
    };
    let settle = |h: &mut Harness, until: &dyn Fn(&str) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !until(&h.text("label")) {
            assert!(
                Instant::now() < deadline,
                "label reads {:?} ({:?})",
                h.text("label"),
                h.host.error().get()
            );
            h.settle();
            h.ui.prepare_frame();
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    // Picked through the save dialog, then saved there; how it goes heard.
    start("keep.txt", "text/plain");
    settle(&mut h, &|label| {
        !answers.borrow().is_empty() && label.contains("keep.txt")
    });
    assert_eq!(answers.borrow()[0], Some(dir.join("out.txt")));
    (statuses.borrow_mut()[0])(DownloadStatus::Progress {
        received: 5,
        total: Some(9),
    });
    (statuses.borrow_mut()[0])(DownloadStatus::Finished);
    settle(&mut h, &|label| label.contains("finished true"));
    assert!(h.text("label").contains("got 5/9;"), "{}", h.text("label"));
    // `d:ask()`: the save dialog suggesting its name, and the file saved to.
    start("ask.txt", "text/plain");
    settle(&mut h, &|label| label.contains("asked out.txt"));
    assert_eq!(answers.borrow()[1], Some(dir.join("out.txt")));
    (statuses.borrow_mut()[1])(DownloadStatus::Failed("offline".into()));
    settle(&mut h, &|label| label.contains("failed offline"));
    // A path the user didn't pick: refused, and the script cancels.
    start("sneak.txt", "text/plain");
    settle(&mut h, &|label| label.contains("sneak false"));
    assert_eq!(answers.borrow()[2], None);
    // Cancelled.
    start("other.bin", "application/octet-stream");
    settle(&mut h, &|_| answers.borrow().len() == 4);
    assert_eq!(answers.borrow()[3], None);
    assert_eq!(h.host.error().get(), None);
}
