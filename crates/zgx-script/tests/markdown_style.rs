//! `ui.markdown_view { style = …, on_link, link_menu, autolink }`: its
//! prose drawn as the script's own `ui.rich` and `ui.span` would draw it,
//! each kind of run with its style's options, its links' clicks and menus
//! the script's.

use zgx::{
    rich_text::TextRun,
    scene::{NodeId, NodeKind},
};
use zgx_script::{ScriptHost, Value, testing::TestApp};

/// The paragraphs under `id`, in order: each one's node, text and runs.
fn paragraphs(app: &mut TestApp, id: &str) -> Vec<(NodeId, String, Vec<TextRun>)> {
    let node = app
        .find(id)
        .unwrap_or_else(|| panic!("no {id}:\n{}", app.tree()));
    let scene = app.ui.scene.borrow();
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if let NodeKind::RichText { text } = scene.kind(n) {
            out.push((n, text.text().to_owned(), text.runs().to_vec()));
        }
        stack.extend(scene.children(n).iter().rev().copied());
    }
    out
}

/// A style naming every kind of run, as an app's own look would.
const STYLE: &str = r##"{
    gap = 14,
    paragraph = { text_size = 15, line_height = 24, color = "#dddddd" },
    strong = { weight = 600 },
    em = { italic = true, color = "#aaaaff" },
    strike = { strike = true },
    code = { font = "mono", bg = "#303030", color = "#ffcc00", padding = { 4, 1 }, radius = 4.5, text_size = 13 },
    link = { color = "#33aaff", underline = true, underline_offset = 4, hover = { underline = true, color = "#ff0000" } },
}"##;

#[test]
fn a_styled_paragraphs_runs_are_the_spans_a_script_would_make() {
    let mut app = TestApp::inline(&format!(
        r##"return ui.column {{ w = 400,
            ui.markdown_view {{ id = "md", w_full = true, style = {STYLE},
                "Plain **bold** *em* ~~gone~~ `code` and **`bold code`** [a link](https://a.test/)." }},
            ui.rich {{ id = "p", wrap = true, text_size = 15, line_height = 24, color = "#dddddd",
                ui.span {{ "Plain ", literal = true }},
                ui.span {{ "bold", literal = true, weight = 600 }},
                ui.span {{ " ", literal = true }},
                ui.span {{ "em", literal = true, italic = true, color = "#aaaaff" }},
                ui.span {{ " ", literal = true }},
                ui.span {{ "gone", literal = true, strike = true }},
                ui.span {{ " ", literal = true }},
                ui.span {{ "code", literal = true, font = "mono", bg = "#303030", color = "#ffcc00",
                    padding = {{ 4, 1 }}, radius = 4.5, text_size = 13 }},
                ui.span {{ " and ", literal = true }},
                ui.span {{ "bold code", literal = true, weight = 600, font = "mono", bg = "#303030",
                    color = "#ffcc00", padding = {{ 4, 1 }}, radius = 4.5, text_size = 13 }},
                ui.span {{ " ", literal = true }},
                ui.span {{ "a link", literal = true, href = "https://a.test/", color = "#33aaff",
                    underline = true, underline_offset = 4, hover = {{ underline = true, color = "#ff0000" }} }},
                ui.span {{ ".", literal = true }},
            }},
        }}"##
    ));
    app.frames(2);
    assert_eq!(app.error(), None);
    let viewed = paragraphs(&mut app, "md");
    let written = paragraphs(&mut app, "p");
    assert_eq!(viewed.len(), 1, "one paragraph: {viewed:?}");
    assert_eq!(viewed[0].1, written[0].1);
    assert_eq!(viewed[0].2, written[0].2, "the same runs");
    // Laid out alike: the paragraph's size and line height are its style's.
    let size = |app: &mut TestApp, node: NodeId| {
        let b = app.ui.scene.borrow().bounds(node);
        (b.width, b.height)
    };
    assert_eq!(size(&mut app, viewed[0].0), size(&mut app, written[0].0));
}

#[test]
fn a_styles_gap_parts_the_blocks_and_its_paragraph_options_take_tokens() {
    let mut app = TestApp::inline(
        r##"return ui.column { w = 400,
            ui.markdown_view { id = "md", w_full = true,
                style = { gap = 30, paragraph = { text_size = "$font.size.lg", color = "$text.secondary" } },
                "One.\n\nTwo." },
            ui.rich { id = "p", text_size = "$font.size.lg", color = "$text.secondary", "One." },
        }"##,
    );
    app.frames(2);
    assert_eq!(app.error(), None);
    let viewed = paragraphs(&mut app, "md");
    assert_eq!(viewed.len(), 2);
    let written = paragraphs(&mut app, "p");
    assert_eq!(viewed[0].2, written[0].2, "the theme's size and ink");
    let scene = app.ui.scene.borrow();
    let (a, b) = (scene.bounds(viewed[0].0), scene.bounds(viewed[1].0));
    assert_eq!(b.y - (a.y + a.height), 30.);
}

#[test]
fn a_run_of_several_kinds_takes_each_kinds_options_the_later_over_the_earlier() {
    // strong, em, strike, code, link: a bold link in code takes bold's
    // weight, code's font and background, and the link's colour over code's.
    let mut app = TestApp::inline(
        r##"return ui.column { w = 400, ui.markdown_view { id = "md", w_full = true,
            style = { strong = { weight = 800, color = "#111111" }, code = { font = "mono", color = "#222222", bg = "#303030" },
                link = { color = "#333333" } },
            "**[`x`](https://a.test/)** **y**" } }"##,
    );
    app.frames(2);
    assert_eq!(app.error(), None);
    let runs = &paragraphs(&mut app, "md")[0].2;
    assert_eq!(runs[0].font.weight, 800);
    assert_eq!(runs[0].color, zgx::scene::Color(0x33, 0x33, 0x33, 255));
    assert!(runs[0].background.is_some());
    let last = runs.last().unwrap();
    assert_eq!(
        (last.font.weight, last.color),
        (800, zgx::scene::Color(0x11, 0x11, 0x11, 255))
    );
}

#[test]
fn a_style_with_a_name_it_does_not_know_is_an_error() {
    let app = TestApp::try_new(|runtime| {
        ScriptHost::inline(
            runtime,
            "app",
            r##"return ui.markdown_view { style = { bold = { weight = 600 } }, "**x**" }"##,
        )
    });
    let error = match app {
        Ok(app) => {
            app.frames(1);
            app.error().unwrap_or_default()
        }
        Err(error) => error,
    };
    assert!(error.contains("style has no `bold`"), "{error}");
}

#[test]
fn a_links_click_is_on_links_and_a_web_address_opens_unless_it_says_handled() {
    let mut app = TestApp::with_desktop(|runtime| {
        ScriptHost::inline(
            runtime,
            "app",
            r##"local seen = ui.state("seen", "")
            local handled = ui.state("handled", false)
            return ui.column { w = 600,
                ui.markdown_view { id = "web", style = {}, "[aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa](https://a.test/)",
                    on_link = function(target, keys)
                        seen:set(seen:peek() .. target .. (if keys.shift then "+" else "") .. ";")
                        return handled:peek()
                    end },
                ui.markdown_view { id = "file", style = {}, "[aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa](src/main.rs)",
                    on_link = function(target) seen:set(seen:peek() .. target .. ";") end },
            }"##,
        )
    });
    let desktop = app.desktop.clone().unwrap();
    app.frames(2);
    app.click("web");
    app.settle();
    assert_eq!(
        desktop.opened(),
        ["https://a.test/"],
        "not handled: it opens"
    );
    app.host.set_state("handled", Value::Bool(true)).unwrap();
    app.click("web");
    app.settle();
    assert_eq!(desktop.opened().len(), 1, "handled: it stays shut");
    // Another target is the script's alone: no address to open.
    app.click("file");
    app.settle();
    assert_eq!(desktop.opened().len(), 1);
    assert_eq!(
        app.host
            .states()
            .into_iter()
            .find(|(name, _)| name == "seen")
            .map(|(_, v)| v.to_string())
            .unwrap(),
        "https://a.test/;https://a.test/;src/main.rs;"
    );
    assert_eq!(app.error(), None);
}

#[test]
fn a_links_menu_is_link_menus_told_its_target() {
    let mut app = TestApp::inline(
        r##"local asked = ui.state("asked", "")
        return ui.column { w = 600,
            ui.markdown_view { id = "md", style = {}, "[aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa](zeron-chat:7)",
                link_menu = function(target, e)
                    asked:set(target)
                    return { ui.text { id = "entry", "Open the chat" } }
                end },
        }"##,
    );
    app.frames(2);
    let b = app.bounds("md");
    zgx_script::driver::right_click_at(&mut app.ui, (b.x + 20., b.y + b.height / 2.));
    app.frames(3);
    assert_eq!(
        app.host
            .states()
            .into_iter()
            .find(|(name, _)| name == "asked")
            .map(|(_, v)| v.to_string())
            .unwrap(),
        "zeron-chat:7"
    );
    assert!(
        app.texts().iter().any(|t| t == "Open the chat"),
        "{:?}",
        app.texts()
    );
    assert_eq!(app.error(), None);
}

#[test]
fn bare_web_addresses_are_links_with_autolink() {
    let mut app = TestApp::inline(
        r##"local style = { link = { color = "#33aaff" } }
        return ui.column { w = 600,
            ui.markdown_view { id = "on", autolink = true, style = style,
                "See https://a.test/x. Or (https://b.test/y) and `https://c.test/` and http://a, **https://d.test/**" },
            ui.markdown_view { id = "off", style = style, "See https://a.test/x." },
        }"##,
    );
    app.frames(2);
    assert_eq!(app.error(), None);
    let links = |app: &mut TestApp, id: &str| -> Vec<(String, u16)> {
        let (_, text, runs) = paragraphs(app, id).remove(0);
        runs.iter()
            .filter(|r| r.color == zgx::scene::Color(0x33, 0xaa, 0xff, 255))
            .map(|r| (text[r.range.clone()].to_string(), r.font.weight))
            .collect()
    };
    assert_eq!(
        links(&mut app, "on"),
        [
            ("https://a.test/x".to_string(), 400),
            ("https://b.test/y".to_string(), 400),
            ("https://d.test/".to_string(), 700),
        ],
        "punctuation and an unopened bracket left out; not in code; bold kept"
    );
    assert!(links(&mut app, "off").is_empty());
}

#[test]
fn a_styled_paragraph_being_written_keeps_its_node() {
    let mut app = TestApp::inline(&format!(
        r##"local reply = ui.state("reply", "Settled.\n\nGrowing")
        ui.command {{ id = "more", run = function() reply:set(reply:peek() .. " **more** `x`") end }}
        return ui.column {{ w = 400, ui.markdown_view {{ "{{reply}}", id = "md", w_full = true, style = {STYLE} }} }}"##
    ));
    app.frames(2);
    let before = paragraphs(&mut app, "md");
    for n in 1..=4 {
        assert!(app.host.run_command("more"));
        app.frames(1);
        let now = paragraphs(&mut app, "md");
        assert_eq!(now[0].0, before[0].0, "the settled paragraph kept");
        assert_eq!(now[1].0, before[1].0, "the growing one kept, after {n}");
        assert_eq!(now[1].1, format!("Growing{}", " more x".repeat(n)));
        assert!(
            now[1].2.iter().any(|r| r.font.weight == 600),
            "its bold run"
        );
    }
    assert_eq!(app.error(), None);
}

#[test]
fn a_spans_text_size_takes_a_token() {
    let mut app = TestApp::inline(
        r##"return ui.column { w = 400,
            ui.rich { id = "token", ui.span { "x", text_size = "$font.size.xl" } },
            ui.rich { id = "number", ui.span { "x", text_size = ui.theme.get("font.size.xl") } },
        }"##,
    );
    app.frames(2);
    assert_eq!(app.error(), None);
    let token = paragraphs(&mut app, "token").remove(0).2;
    let number = paragraphs(&mut app, "number").remove(0).2;
    assert_eq!(token, number);
    assert!(token[0].font_size > 16., "{token:?}");
}

#[test]
fn a_styles_options_are_not_animated() {
    let app = TestApp::try_new(|runtime| {
        ScriptHost::inline(
            runtime,
            "app",
            r##"local size = ui.state("size", 15)
            return ui.markdown_view { style = { paragraph = { text_size = ui.tween(size) } }, "x" }"##,
        )
    });
    let error = match app {
        Ok(app) => {
            app.frames(1);
            app.error().unwrap_or_default()
        }
        Err(error) => error,
    };
    assert!(error.contains("not animations"), "{error}");
}
