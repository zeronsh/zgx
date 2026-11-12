//! A markdown view's `on_link`: told of its links before they open.
mod common;
use common::Harness;
use zgx_script::ScriptHost;

fn app(on_link: &str) -> String {
    format!(
        r##"local told = ui.state("told", "")
return ui.column {{
    ui.markdown_view {{
        id = "md",
        w = 160,
        "[aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa](https://anara.com/cite/doc1?page=3)",
        {on_link}
    }},
    ui.text {{ id = "told", function() return told:get() end }},
}}"##
    )
}

fn click_link(code: String) -> (Vec<String>, String) {
    let code: &'static str = Box::leak(code.into_boxed_str());
    let mut h = Harness::with_desktop(move |runtime| ScriptHost::inline(runtime, "app", code));
    let desktop = h.desktop.clone().unwrap();
    h.ui.prepare_frame();
    h.click("md");
    h.settle();
    (desktop.opened(), h.text("told"))
}

#[test]
fn a_link_on_link_handles_stays_unopened() {
    let (opened, told) = click_link(app(
        r#"on_link = function(url) told:set(url) return true end,"#,
    ));
    assert!(opened.is_empty(), "{opened:?}");
    assert_eq!(told, "https://anara.com/cite/doc1?page=3");
}

#[test]
fn a_link_on_link_passes_on_opens() {
    let (opened, told) = click_link(app(r#"on_link = function(url) told:set(url) end,"#));
    assert_eq!(opened, ["https://anara.com/cite/doc1?page=3"]);
    assert_eq!(told, "https://anara.com/cite/doc1?page=3");
}

#[test]
fn without_on_link_a_link_opens() {
    let (opened, _) = click_link(app(""));
    assert_eq!(opened, ["https://anara.com/cite/doc1?page=3"]);
}

#[test]
fn the_open_link_item_asks_on_link_too() {
    let code: &'static str = Box::leak(
        app(r#"on_link = function(url) told:set(url) return true end,"#).into_boxed_str(),
    );
    let mut h = Harness::with_desktop(move |runtime| ScriptHost::inline(runtime, "app", code));
    let desktop = h.desktop.clone().unwrap();
    h.ui.prepare_frame();
    let bounds = h.bounds("md");
    zgx_script::driver::right_click_at(&mut h.ui, (bounds.x + 20., bounds.y + 8.));
    h.settle();
    h.ui.prepare_frame();
    h.click("link-open");
    h.settle();
    assert!(desktop.opened().is_empty(), "{:?}", desktop.opened());
    assert_eq!(h.text("told"), "https://anara.com/cite/doc1?page=3");
}
