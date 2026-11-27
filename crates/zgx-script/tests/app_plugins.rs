//! A Luau app's scripts from its `zgx.json` alone (`app_config`): a script
//! beside the app's reaches its shared state, an action it defines
//! (`ui.actions`) and a target to filter, with nothing granted; `zgx test`
//! mounts it as `zgx run` would. A script that isn't there is an error as
//! it loads, naming it.
mod common;

use std::{path::PathBuf, process::Command};

const APP: &str = r##"
local tasks = ui.shared("tasks", {
    { id = 1, title = "Write", done = false },
    { id = 2, title = "Ship", done = true },
})
ui.actions {
    complete = function(id)
        local list = table.clone(tasks:peek())
        for i, task in list do
            if task.id == id then
                list[i] = { id = task.id, title = task.title, done = true }
            end
        end
        tasks:set(list)
    end,
}
return ui.column {
    ui.text { id = "title", "Tasks" },
    ui.each(tasks, function(task)
        return ui.text { id = `task-{task.id}`, task.title }
    end, { id = "tasks", key = function(task) return task.id end }),
}
"##;

const PLUGIN: &str = r##"
local tasks = ui.shared("tasks", {})
ui.contribute {
    target = "#title",
    after = ui.button { id = "finish", w = 60, h = 24, "Finish", on_click = function()
        ui.invoke("complete", tasks:peek()[1].id)
    end },
}
ui.filter {
    target = "#tasks",
    items = function(items)
        local open = {}
        for _, task in items do
            if not task.done then
                table.insert(open, task)
            end
        end
        return open
    end,
}
"##;

const TEST: &str = r##"
test("the plugin adds, filters and calls the app's action", function()
    local app = mount()
    expect(app:error()).to_be_nil()
    expect(app:has("#finish")).to_be(true)
    expect(app:has("#task-1")).to_be(true)
    expect(app:has("#task-2")).to_be(false)
    app:click("#finish")
    expect(app:state("tasks")[1].done).to_be(true)
    expect(app:has("#task-1")).to_be(false)
end)
"##;

const CONFIG: &str = r##"{
    "scripts": { "focus": "plugins/focus" }
}"##;

fn app(name: &str, config: &str) -> PathBuf {
    let dir = common::temp_dir(&format!("app-plugins-{name}"));
    std::fs::create_dir_all(dir.join("plugins/focus")).unwrap();
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::write(dir.join("app.luau"), APP).unwrap();
    std::fs::write(dir.join("plugins/focus/init.luau"), PLUGIN).unwrap();
    std::fs::write(dir.join("tests/app.test.luau"), TEST).unwrap();
    std::fs::write(dir.join("zgx.json"), config).unwrap();
    dir
}

#[test]
fn a_luau_apps_scripts_come_from_its_zgx_json() {
    let dir = app("scripts", CONFIG);
    let out = Command::new(env!("CARGO_BIN_EXE_zgx"))
        .arg("test")
        .arg(&dir)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0), "{text}");
    std::fs::remove_dir_all(dir).unwrap();
}

/// A script whose path is neither a file nor a folder with an `init.luau`
/// is an error as the app loads, naming the script.
#[test]
fn a_script_that_isnt_there_is_an_error_naming_it() {
    let dir = app(
        "missing",
        &CONFIG.replace("plugins/focus", "plugins/missing"),
    );
    let error = zgx_script::app_config::read(&dir).unwrap_err();
    assert!(error.contains("script `focus`:"), "{error}");
    std::fs::remove_dir_all(dir).unwrap();
}

/// A script beside the app's defines actions too, and the app calls them.
#[test]
fn a_script_beside_the_apps_defines_actions() {
    let dir = app("script-defines", CONFIG);
    std::fs::write(
        dir.join("plugins/focus/init.luau"),
        r#"
local pinged = ui.shared("pinged", false)
ui.actions { ping = function() pinged:set(true) end }
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("app.luau"),
        r#"
local pinged = ui.shared("pinged", false)
return ui.button { id = "ping", w = 60, h = 24, "Ping", on_click = function()
    ui.invoke("ping")
end }
"#,
    )
    .unwrap();
    let mut app = zgx_script::testing::TestApp::new(|runtime| {
        zgx_script::app_config::read(&dir)
            .unwrap()
            .apply(zgx_script::ScriptHost::new(runtime, dir.join("app.luau")))
    });
    assert_eq!(app.error(), None);
    app.click("ping");
    app.frames(1);
    assert_eq!(app.error(), None);
    let pinged = app
        .host
        .states()
        .iter()
        .find(|(name, _)| name == "pinged")
        .map(|(_, value)| value.to_json());
    assert_eq!(pinged.as_deref(), Some("true"), "the script's action ran");
    std::fs::remove_dir_all(dir).unwrap();
}
