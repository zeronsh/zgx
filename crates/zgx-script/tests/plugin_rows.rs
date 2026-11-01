//! What plugins add before or after a list row's root: drawn a row each,
//! keyed with the row (it follows a reorder and goes with a removal), and
//! reading what the row provides, live.
mod common;
use common::Harness;
use zgx_script::{Plugin, ScriptHost};

fn with(test: &str, app: &'static str, code: &str) -> Harness {
    let dir = std::env::temp_dir().join(format!("zgx-rows-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(&entry, code).unwrap();
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", app);
        host.set_plugins(vec![Plugin {
            name: "marks".into(),
            entry,
            grants: vec![],
        }]);
        host
    })
}

const APP: &str = r##"
local items = ui.state("items", {
    { id = "a", title = "Alpha" },
    { id = "b", title = "Beta" },
    { id = "c", title = "Gamma" },
})
ui.command { id = "reverse", run = function()
    items:set({ items:get()[3], items:get()[2], items:get()[1] })
end }
ui.command { id = "drop-b", run = function()
    items:set({ items:get()[1], items:get()[3] })
end }
ui.command { id = "rename-a", run = function()
    local list = items:get()
    items:set({ list[1], { id = list[2].id, title = "Alpha!" } })
end }
return ui.column {
    ui.each(items, function(item, _, row)
        return ui.row {
            class = "session",
            provide = { row = row },
            ui.text { function() return row:get().title end },
        }
    end, { key = function(item) return item.id end }),
}
"##;

const PLUGIN: &str = r##"
local Badge = ui.component(function()
    local row = ui.use_context("row")
    return ui.text { function() return `[{row:get().title}]` end }
end)
ui.contribute { target = ".session", after = Badge {} }
"##;

fn shown(h: &mut Harness) -> Vec<String> {
    h.frames(1);
    h.texts()
}

#[test]
fn a_row_contribution_follows_its_row_and_reads_it_live() {
    let mut h = with("rows", APP, PLUGIN);
    assert_eq!(
        shown(&mut h),
        ["Alpha", "[Alpha]", "Beta", "[Beta]", "Gamma", "[Gamma]"],
        "after each row's root, reading what the row provides"
    );
    assert_eq!(h.error(), None);
    assert_eq!(h.host.contributions()[0].matches, 3);
    assert!(h.host.run_command("reverse"));
    assert_eq!(
        shown(&mut h),
        ["Gamma", "[Gamma]", "Beta", "[Beta]", "Alpha", "[Alpha]"],
        "keyed with its row: it moves with it"
    );
    assert!(h.host.run_command("drop-b"));
    assert_eq!(
        shown(&mut h),
        ["Gamma", "[Gamma]", "Alpha", "[Alpha]"],
        "a removed row takes it along"
    );
    assert_eq!(h.host.contributions()[0].matches, 2);
    assert!(h.host.run_command("rename-a"));
    assert_eq!(
        shown(&mut h),
        ["Gamma", "[Gamma]", "Alpha!", "[Alpha!]"],
        "the row's state, read live"
    );
}
