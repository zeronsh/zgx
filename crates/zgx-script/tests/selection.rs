//! A script styles display text selections as CSS's `::selection` does.
mod common;
use common::Harness;
use zgx::scene::{Color, NodeKind};

#[test]
fn selection_props_and_theme_tokens_style_a_nodes_highlight() {
    let mut h = Harness::inline(
        r##"local tint = ui.state("tint", "#ff0000")
        return ui.column {
            ui.column {
                selectable = true,
                selection_color = "#0000ff80",
                selected_text_color = function() return tint:get() end,
                selection_radius = 5,
                ui.text { "Props", id = "props" },
            },
            ui.column {
                selectable = true,
                theme = { ["selection.text"] = "#00ff00", ["selection.radius"] = 3 },
                ui.text { "Themed", id = "themed" },
            },
        }"##,
    );
    h.settle();
    h.ui.prepare_frame();
    let text_in = |h: &mut Harness, id: &str| {
        let node = h.find(id).unwrap();
        let scene = h.ui.scene.borrow();
        if matches!(scene.kind(node), NodeKind::Text { .. }) {
            node
        } else {
            scene.children(node)[0]
        }
    };
    let props = text_in(&mut h, "props");
    assert!(h.ui.select_all_text(props));
    let highlight = h.ui.scene.borrow().text_highlight(props).unwrap().clone();
    assert_eq!(highlight.color, Color(0, 0, 255, 128));
    assert_eq!(highlight.text_color, Some(Color(255, 0, 0, 255)));
    assert_eq!(highlight.radius, 5.);
    let themed = text_in(&mut h, "themed");
    assert!(h.ui.select_all_text(themed));
    let highlight = h.ui.scene.borrow().text_highlight(themed).unwrap().clone();
    assert_eq!(highlight.text_color, Some(Color(0, 255, 0, 255)));
    assert_eq!(highlight.radius, 3.);
}

/// `selection_radius` is the subtree's `$selection.radius`: it follows a
/// state, and keeps a `theme` given beside it in either order.
#[test]
fn a_selection_radius_follows_state_beside_a_theme() {
    let mut h = Harness::inline(
        r##"local round = ui.state("round", 2)
        return ui.column {
            ui.column {
                selectable = true,
                selection_radius = round,
                theme = { ["selection.text"] = "#00ff00" },
                ui.text { "First", id = "first" },
            },
            ui.column {
                theme = { ["selection.text"] = "#00ff00" },
                selectable = true,
                selection_radius = 7,
                ui.text { "Second", id = "second" },
            },
        }"##,
    );
    h.settle();
    h.ui.prepare_frame();
    let look = |h: &mut Harness, id: &str| {
        let node = h.find(id).unwrap();
        let text = {
            let scene = h.ui.scene.borrow();
            if matches!(scene.kind(node), NodeKind::Text { .. }) {
                node
            } else {
                scene.children(node)[0]
            }
        };
        assert!(h.ui.select_all_text(text));
        h.ui.prepare_frame();
        let highlight = h.ui.scene.borrow().text_highlight(text).unwrap().clone();
        (highlight.radius, highlight.text_color)
    };
    let green = Some(Color(0, 255, 0, 255));
    assert_eq!(look(&mut h, "first"), (2., green));
    assert_eq!(look(&mut h, "second"), (7., green));
    h.host
        .set_state("round", zgx_script::Value::Num(5.))
        .unwrap();
    h.settle();
    assert_eq!(look(&mut h, "first"), (5., green), "follows the state");
    assert_eq!(h.error(), None);
}

/// `on_copy` on a selectable region says what a copy out of it puts on
/// the clipboard: it hears the selection's text, HTML and the ids of the
/// text it spans in order (`false` for text without one); nil keeps the
/// copy as it was.
#[test]
fn on_copy_decides_what_a_regions_copy_writes() {
    let mut h = Harness::inline(
        r##"local plain = ui.state("plain", false)
        local heard = ui.state("heard", "")
        return ui.column {
            selectable = true,
            id = "region",
            on_copy = function(request)
                local ids = {}
                for _, leaf in request.leaves do
                    table.insert(ids, if leaf then leaf else "-")
                end
                heard:set(table.concat(ids, " "))
                if plain:get() then
                    return nil
                end
                return { text = "[1] " .. request.text, html = "<cite>" .. request.text .. "</cite>" }
            end,
            ui.text { "Alpha", id = "a" },
            ui.row { ui.text { "Beta", id = "b" }, ui.column { ui.text { "Gamma" } } },
            ui.text { "Delta", id = "d" },
        }"##,
    );
    h.settle();
    h.ui.prepare_frame();
    let region = h.find("region").unwrap();
    assert!(h.ui.select_all_text(region));
    let copy = |h: &mut Harness| {
        let mut written = None;
        assert!(h.ui.copy_selection_from(|text, html| {
            written = Some((text, html));
            true
        }));
        h.settle();
        written.unwrap()
    };
    let selected = h.ui.selected_text();
    assert!(selected.starts_with("Alpha") && selected.ends_with("Delta"));
    let (text, html) = copy(&mut h);
    assert_eq!(text, format!("[1] {selected}"));
    assert_eq!(html, Some(format!("<cite>{selected}</cite>")));
    let heard = h
        .host
        .states()
        .into_iter()
        .find(|(name, _)| name == "heard");
    assert!(
        matches!(&heard, Some((_, zgx_script::Value::Str(ids))) if &**ids == "a b - d"),
        "{heard:?}"
    );
    h.host
        .set_state("plain", zgx_script::Value::Bool(true))
        .unwrap();
    h.settle();
    let (text, html) = (h.ui.selected_text(), h.ui.selected_html());
    assert_eq!(copy(&mut h), (text, Some(html)), "nil keeps the copy");
    assert_eq!(h.error(), None);
}

/// `on_copy` hears the rows of each `ui.list` the selection spans, from 1,
/// mounted or not: a Select All over five hundred rows, a few mounted,
/// names them all, by the list's id.
#[test]
fn on_copy_hears_the_rows_of_the_lists_it_spans() {
    let mut h = Harness::inline(
        r##"local heard = ui.state("heard", "")
        return ui.column {
            selectable = true,
            id = "region",
            on_copy = function(request)
                local said = {}
                for _, rows in request.rows do
                    table.insert(said, `{rows.list} {rows.first} {rows.last}`)
                end
                heard:set(table.concat(said, ", "))
                return nil
            end,
            ui.text { "Header" },
            ui.list {
                id = "items",
                count = 500,
                height = 20,
                h = 100,
                row = function(i) return ui.text { "Row " .. i, literal = true } end,
            },
        }"##,
    );
    h.settle();
    h.ui.prepare_frame();
    let region = h.find("region").unwrap();
    assert!(h.ui.select_all_text(region));
    h.ui.prepare_frame();
    assert!(h.ui.copy_selection_from(|_, _| true));
    h.settle();
    let heard = h
        .host
        .states()
        .into_iter()
        .find(|(name, _)| name == "heard");
    assert!(
        matches!(&heard, Some((_, zgx_script::Value::Str(said))) if &**said == "items 1 500"),
        "{heard:?}"
    );
    assert_eq!(h.error(), None);
}

/// A `ui.list` that is itself the selectable region (Anara's transcript):
/// the copy request names its rows by the list's own id, not `false`.
#[test]
fn a_list_that_is_the_region_names_its_rows_by_its_own_id() {
    let mut h = Harness::inline(
        r##"local heard = ui.state("heard", "")
        return ui.column {
            ui.list {
                id = "transcript:scale",
                selectable = true,
                count = 180,
                height = 20,
                h = 100,
                on_copy = function(request)
                    local said = {}
                    for _, rows in request.rows do
                        table.insert(said, `{rows.list} {rows.first} {rows.last}`)
                    end
                    heard:set(table.concat(said, ", "))
                    return nil
                end,
                row = function(i) return ui.text { "Row " .. i, literal = true } end,
            },
        }"##,
    );
    h.settle();
    h.ui.prepare_frame();
    let list = h.find("transcript:scale").unwrap();
    assert!(h.ui.select_all_text(list));
    h.ui.prepare_frame();
    assert!(h.ui.copy_selection_from(|_, _| true));
    h.settle();
    let heard = h
        .host
        .states()
        .into_iter()
        .find(|(name, _)| name == "heard");
    assert!(
        matches!(&heard, Some((_, zgx_script::Value::Str(said))) if &**said == "transcript:scale 1 180"),
        "{heard:?}"
    );
    assert_eq!(h.error(), None);
}

/// `zgx inspect node` on a paragraph with a citation in it says what a
/// reader hears: its text, and the citation as its button where it stands.
#[test]
fn inspect_says_what_a_reader_hears_of_a_paragraph_with_a_citation() {
    let h = Harness::inline(
        r##"return ui.column { w = 400,
            ui.rich {
                ui.span { "step " },
                ui.span { "1", label = "Citation 1", on_click = function() end },
                ui.span { "." },
            },
        }"##,
    );
    h.settle();
    h.frames(3);
    let paragraph = {
        let semantics = h.ui.semantics.borrow();
        semantics
            .iter()
            .map(|(id, _)| id)
            .find(|id| !semantics.inline_controls(*id).is_empty())
            .expect("the paragraph, with its citation")
    };
    let fields = zgx_script::driver::node(&h.ui, None, paragraph);
    let reads = fields
        .iter()
        .find(|(k, _)| k == "reads")
        .map(|(_, v)| v.as_str());
    assert_eq!(reads, Some(r#""step " [button "Citation 1"] ".""#));
    let value = fields
        .iter()
        .find(|(k, _)| k == "value")
        .map(|(_, v)| v.as_str());
    assert_eq!(value, Some("step 1."), "its value is as zgx keeps it");
}
