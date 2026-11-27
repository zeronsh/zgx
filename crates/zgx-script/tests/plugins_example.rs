//! The plugins example (examples/plugins), run headless with its host and
//! both plugins, each a script of its own: what each adds, filters and
//! replaces shows, the host's actions change the tasks both the app and the
//! plugins show, and a script reaches all of the app (no element, state or
//! action is kept from it).
mod common;

#[path = "../examples/support/plugins_host.rs"]
mod plugins_host;

use common::Harness;
use std::path::Path;
use zgx_script::Script;

fn app() -> Harness {
    let h = Harness::new(plugins_host::host);
    h.ui.scene.borrow_mut().resize(560., 480.);
    h.frames(2);
    assert_eq!(h.error(), None);
    h
}

#[test]
fn both_plugins_load_and_add_where_they_target() {
    let mut h = app();
    assert!(h.find("stamp").is_some(), "before the title");
    assert!(
        h.find("focus").is_some(),
        "after the declared hook toolbar.add"
    );
    assert!(h.find("clear").is_some(), "the app's own button");
    let texts = h.texts();
    assert!(texts.contains(&"Write the guide".to_string()), "{texts:?}");
    assert!(
        texts.contains(&"done".to_string()),
        "the overriding row module's badge"
    );
}

#[test]
fn the_focus_filter_follows_the_shared_state_it_reads() {
    let mut h = app();
    assert!(h.texts().contains(&"Review the plugins".to_string()));
    assert!(
        h.texts().contains(&"Focus (2)".to_string()),
        "it reads the tasks: {:?}",
        h.texts()
    );
    h.click("focus");
    h.frames(1);
    let texts = h.texts();
    assert!(
        !texts.contains(&"Review the plugins".to_string()),
        "done tasks hidden: {texts:?}"
    );
    assert!(texts.contains(&"Show all".to_string()), "{texts:?}");
    h.click("focus");
    h.frames(1);
    assert!(h.texts().contains(&"Review the plugins".to_string()));
}

#[test]
fn the_hosts_actions_change_what_everyone_shows() {
    let mut h = app();
    h.click("done-1");
    h.frames(1);
    assert!(h.find("done-1").is_none(), "completed");
    h.click("clear");
    h.frames(1);
    let texts = h.texts();
    assert!(!texts.contains(&"Write the guide".to_string()), "{texts:?}");
    assert!(texts.contains(&"Ship it".to_string()), "{texts:?}");
}

#[test]
fn the_empty_state_is_the_plugins() {
    let mut h = app();
    h.click("done-1");
    h.click("done-3");
    h.click("clear");
    h.frames(1);
    assert!(h.find("empty").is_none(), "replaced");
    assert!(
        h.texts()
            .contains(&"All clear: add a task above.".to_string()),
        "{:?}",
        h.texts()
    );
}

/// Beside the example's two plugins, a script adds after the app's Clear
/// button, writes the host's `tasks` and calls its `add`: none of it is
/// kept from a script.
#[test]
fn a_script_reaches_every_element_state_and_action() {
    let mut h = Harness::new(|runtime| {
        let host = plugins_host::host(runtime);
        let plugins = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plugins/plugins");
        host.set_scripts(vec![
            Script::file("focus", plugins.join("focus/init.luau")),
            Script::file("stamps", plugins.join("stamps/init.luau")),
            Script::code(
                "extra",
                r##"
local tasks = ui.shared("tasks", {})
ui.contribute {
    target = "#clear",
    after = ui.column {
        ui.button { id = "extra-add", "Add one", on_click = function()
            ui.invoke("add", "From a script")
        end },
        ui.button { id = "wipe", "Wipe", on_click = function()
            tasks:set({ { id = 9, title = "Only this", done = false } })
        end },
    },
}
"##,
            ),
        ]);
        host
    });
    h.ui.scene.borrow_mut().resize(560., 480.);
    h.frames(2);
    assert_eq!(h.error(), None);
    assert!(h.find("stamp").is_some(), "the plugins still load");
    h.click("extra-add");
    h.frames(1);
    assert_eq!(h.error(), None);
    assert!(
        h.texts().contains(&"From a script".to_string()),
        "the host's add ran: {:?}",
        h.texts()
    );
    h.click("wipe");
    h.frames(1);
    assert_eq!(h.error(), None);
    let texts = h.texts();
    assert!(!texts.contains(&"Write the guide".to_string()), "{texts:?}");
    assert!(
        texts.contains(&"Only this".to_string()),
        "written, the list is the script's: {texts:?}"
    );
}
