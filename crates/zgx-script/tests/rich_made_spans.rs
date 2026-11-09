//! `ui.rich { spans = function() … end }`: runs a function makes, made
//! again in place as what it reads changes. The paragraph keeps its node
//! (and so its selection and its text's layout), and its runs are as the
//! same runs listed would be: links, clicks, icons, hover and fades alike.

use std::time::Duration;

use zgx::{
    rich_text::TextRun,
    scene::{NodeId, NodeKind},
    semantics::Role,
};
use zgx_script::{ScriptHost, Value, testing::TestApp};

/// The paragraph under `id`: its node, text and runs.
fn paragraph(app: &mut TestApp, id: &str) -> (NodeId, String, Vec<TextRun>) {
    let node = app
        .find(id)
        .unwrap_or_else(|| panic!("no {id}:\n{}", app.tree()));
    let scene = app.ui.scene.borrow();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if let NodeKind::RichText { text } = scene.kind(n) {
            return (n, text.text().to_owned(), text.runs().to_vec());
        }
        stack.extend(scene.children(n).iter().rev().copied());
    }
    panic!("no paragraph under {id}");
}

/// Whether an icon is drawn under `id`.
fn has_icon(app: &mut TestApp, id: &str) -> bool {
    let node = app.find(id).unwrap();
    let scene = app.ui.scene.borrow();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if matches!(scene.kind(n), NodeKind::Svg(_)) {
            return true;
        }
        stack.extend(scene.children(n).iter().copied());
    }
    false
}

#[test]
fn a_paragraph_of_made_runs_keeps_its_node_as_they_change() {
    let mut app = TestApp::inline(
        r##"local words = ui.state("words", 1)
        return ui.column { w = 300, ui.rich { id = "p", wrap = true,
            spans = function()
                local runs = {}
                for i = 1, words:get() do
                    runs[i] = ui.span { if i == 1 then "One " else "more ", literal = true, bold = i == 1 }
                end
                return runs
            end } }"##,
    );
    app.frames(2);
    assert_eq!(app.error(), None);
    let (node, text, runs) = paragraph(&mut app, "p");
    assert_eq!(text, "One ");
    assert_eq!(runs.len(), 1);
    let element = app.find("p").unwrap();
    for n in 2..=6 {
        app.host.set_state("words", Value::Num(n as f64)).unwrap();
        app.frames(1);
        let (now, text, runs) = paragraph(&mut app, "p");
        assert_eq!(now, node, "the same paragraph after {n} runs");
        assert_eq!(app.find("p"), Some(element), "and the same element");
        assert_eq!(text, format!("One {}", "more ".repeat(n - 1)));
        assert_eq!(runs.len(), n, "a run each: {runs:?}");
        assert_eq!(runs[0].font.weight, 700);
        assert_eq!(runs[n - 1].font.weight, 400);
    }
    assert_eq!(app.error(), None);
}

/// The runs `body` makes, listed and made by a function, are the same.
#[test]
fn made_runs_are_the_runs_listed() {
    let body = r##"
        ui.span { "Read ", literal = true },
        ui.span { "the docs", literal = true, href = "https://zgx.zeron.sh", color = "#ff0000" },
        "{count} plain, ",
        { "bold", bold = true },
        ui.span { " code", literal = true, font = "mono", bg = "#202020", padding = { 3, 1 }, radius = 4 },
        ui.span { " struck", literal = true, strike = true, underline_style = "wavy" },
        ui.span { " sized", literal = true, text_size = 20, italic = true, color = function() return "#00ff00" end },
    "##;
    let listed = TestApp::inline(&format!(
        r##"local count = ui.state("count", 3)
        return ui.column {{ w = 400, ui.rich {{ id = "p", wrap = true, {body} }} }}"##
    ));
    let made = TestApp::inline(&format!(
        r##"local count = ui.state("count", 3)
        return ui.column {{ w = 400, ui.rich {{ id = "p", wrap = true,
            spans = function() return {{ {body} }} end }} }}"##
    ));
    let mut all = [listed, made];
    let [a, b] = all.each_mut().map(|app| {
        app.frames(2);
        assert_eq!(app.error(), None);
        let (_, text, runs) = paragraph(app, "p");
        let links = (app.ui.semantics.borrow().iter())
            .filter(|(_, s)| s.role == Role::Link)
            .count();
        (text, runs, links, app.bounds("p"))
    });
    assert_eq!(a, b);
    assert_eq!(a.2, 1, "one link");
    // A state a made run's template reads is read as a listed one's is.
    for app in &mut all {
        app.host.set_state("count", Value::Num(4.)).unwrap();
        app.frames(1);
    }
    let [a, b] = all.each_mut().map(|app| paragraph(app, "p").1);
    assert_eq!(a, b);
    assert!(a.contains("4 plain"), "{a}");
}

#[test]
fn a_made_link_opens_where_it_links_now_and_its_click_runs() {
    let mut app = TestApp::with_desktop(|runtime| {
        ScriptHost::inline(
            runtime,
            "app",
            r##"local to = ui.state("to", "https://a.test/")
            local seen = ui.state("seen", "")
            return ui.column { w = 300,
                ui.rich { id = "para", w = 120, spans = function()
                    return { ui.span { "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", literal = true, href = to:get(),
                        on_click = function() seen:set(seen:peek() .. "c") end } }
                end },
                ui.text { "{seen}", id = "seen" } }"##,
        )
    });
    let desktop = app.desktop.clone().unwrap();
    app.ui.prepare_frame();
    let (node, ..) = paragraph(&mut app, "para");
    app.click("para");
    app.settle();
    assert_eq!(desktop.opened(), ["https://a.test/"]);
    (app.host
        .set_state("to", Value::Str("https://b.test/".into())))
    .unwrap();
    app.frames(1);
    assert_eq!(paragraph(&mut app, "para").0, node, "the same paragraph");
    app.click("para");
    app.settle();
    assert_eq!(desktop.opened(), ["https://a.test/", "https://b.test/"]);
    assert_eq!(app.text("seen"), "cc", "its on_click ran each time");
    let links = (app.ui.semantics.borrow().iter())
        .filter(|(_, s)| s.role == Role::Link)
        .count();
    assert_eq!(links, 1);
    assert_eq!(app.error(), None);
}

#[test]
fn a_made_links_hover_underlines_it() {
    let mut app = TestApp::inline(
        r##"return ui.column { w = 300, ui.rich { id = "p", w = 200, spans = function()
            return { ui.span { "aaaaaaaaaaaaaaaaaaaaaaaaaa", literal = true, on_click = function() end,
                hover = { underline = true, color = "#ff0000" } } }
        end } }"##,
    );
    app.frames(2);
    assert!(paragraph(&mut app, "p").2[0].underline.is_none(), "at rest");
    app.hover("p");
    app.frames(2);
    let runs = paragraph(&mut app, "p").2;
    assert!(runs[0].underline.is_some(), "under the pointer: {runs:?}");
    assert_eq!(runs[0].color, zgx::scene::Color(255, 0, 0, 255));
}

#[test]
fn a_made_runs_icon_comes_and_goes_in_the_same_paragraph() {
    let svg = r#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 12 12'><rect width='12' height='12'/></svg>"#;
    let mut app = TestApp::inline(&format!(
        r##"local iconed = ui.state("iconed", false)
        return ui.column {{ w = 300, ui.rich {{ id = "p", spans = function()
            return {{ ui.span {{ "src/main.rs", literal = true,
                icon = if iconed:get() then {{ code = "{svg}", size = 12, gap = 4 }} else nil }} }}
        end }} }}"##
    ));
    app.frames(2);
    let (node, ..) = paragraph(&mut app, "p");
    assert!(!has_icon(&mut app, "p"));
    app.host.set_state("iconed", Value::Bool(true)).unwrap();
    app.frames(2);
    assert!(has_icon(&mut app, "p"), "the icon drawn:\n{}", app.tree());
    assert_eq!(paragraph(&mut app, "p").0, node);
    app.host.set_state("iconed", Value::Bool(false)).unwrap();
    app.frames(2);
    assert!(!has_icon(&mut app, "p"));
    assert_eq!(paragraph(&mut app, "p").0, node);
}

fn paced(app: &TestApp, frames: usize) {
    for _ in 0..frames {
        std::thread::sleep(Duration::from_millis(16));
        app.frames(1);
    }
}

/// A run made again with the start it had goes on fading where it was: its
/// fade is the time's, not the paragraph's making.
#[test]
fn a_made_runs_fade_goes_on_as_runs_are_added() {
    let mut app = TestApp::inline(
        r##"local chunks = ui.state("chunks", 1)
        local texts, starts = { "Fading in. ", "More." }, {}
        return ui.column { w = 400, ui.rich { id = "p", wrap = true, spans = function()
            local runs = {}
            for i = 1, chunks:get() do
                -- Each chunk's start, the time it came: kept as runs are made again.
                starts[i] = starts[i] or ui.now()
                runs[i] = ui.span { texts[i], literal = true, color = "#ff0000",
                    fade_in = { start = starts[i], ms = 400, power = 1.6 } }
            end
            return runs
        end } }"##,
    );
    app.ui.scene.borrow_mut().resize(600., 200.);
    app.frames(1);
    assert_eq!(app.error(), None);
    let alpha = |app: &mut TestApp, i: usize| paragraph(app, "p").2[i].color.3;
    let first = alpha(&mut app, 0);
    assert!(first < 0x60, "starts near clear: {first}");
    let calls = app.calls();
    paced(&app, 10);
    let between = alpha(&mut app, 0);
    assert!(between > first && between < 0xff, "on its way: {between}");
    assert_eq!(app.calls(), calls, "no script runs while it fades");
    // A chunk more: the first goes on from where it was.
    app.host.set_state("chunks", Value::Num(2.)).unwrap();
    app.frames(1);
    let (_, text, runs) = paragraph(&mut app, "p");
    assert_eq!(text, "Fading in. More.");
    assert!(
        runs[0].color.3 >= between,
        "not faded in again: {} after {between}",
        runs[0].color.3
    );
    assert!(runs[1].color.3 < runs[0].color.3, "the new one fades in");
    paced(&app, 40);
    let runs = paragraph(&mut app, "p").2;
    assert!(runs.iter().all(|r| r.color.3 == 0xff), "{runs:?}");
    app.frames(2);
    assert!(!app.clock.wants_frame(), "asks for no frames once done");
}

#[test]
fn an_error_in_the_function_is_reported_and_the_paragraph_recovers() {
    let mut app = TestApp::inline(
        r##"local broken = ui.state("broken", false)
        return ui.column { w = 300, ui.rich { id = "p", spans = function()
            if broken:get() then error("no runs today") end
            return { "Fine" }
        end } }"##,
    );
    app.frames(2);
    assert_eq!(app.error(), None);
    let (node, text, _) = paragraph(&mut app, "p");
    assert_eq!(text, "Fine");
    app.host.set_state("broken", Value::Bool(true)).unwrap();
    app.frames(1);
    let error = app.error().expect("reported");
    assert!(error.contains("no runs today"), "{error}");
    app.host.set_state("broken", Value::Bool(false)).unwrap();
    app.frames(1);
    let (now, text, _) = paragraph(&mut app, "p");
    assert_eq!((now, text.as_str()), (node, "Fine"), "made again, in place");
}

#[test]
fn spans_is_a_function_and_takes_no_listed_runs_beside_it() {
    for (script, said) in [
        (
            r#"return ui.rich { "a", spans = function() return {} end }"#,
            "not both",
        ),
        (r#"return ui.rich { spans = { "a" } }"#, "is a function"),
        (
            r#"return ui.rich { spans = function() return 3 end }"#,
            "returns a list",
        ),
    ] {
        let error = match TestApp::try_new(|runtime| ScriptHost::inline(runtime, "app", script)) {
            Err(error) => error,
            Ok(app) => {
                app.frames(1);
                app.error().unwrap_or_default()
            }
        };
        assert!(error.contains(said), "{script}: {error}");
    }
}
