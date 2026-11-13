//! Rich text (`rich_text`, `rich_text_signal`) as the browser's inline
//! text: text nodes and styled `<span>`s and `<a>`s in one paragraph.
#![cfg(target_arch = "wasm32")]

use std::{cell::Cell, rc::Rc, sync::Arc};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use zgx::{
    compose::{
        SpanHover, SpanIcon, TextFade, column, fade_now, rich_text, rich_text_signal, text_span,
    },
    reactive::Runtime,
    rich_text::Decoration,
    scene::Color,
    svg::SvgData,
};

wasm_bindgen_test_configure!(run_in_browser);

fn host() -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
}

/// The paragraph: the column's first child.
fn paragraph(host: &web_sys::Element) -> web_sys::Element {
    let root = host.first_element_child().unwrap();
    let column = root.first_element_child().unwrap();
    column.first_element_child().unwrap()
}

fn style(element: &web_sys::Element, property: &str) -> String {
    web_sys::window()
        .unwrap()
        .get_computed_style(element)
        .unwrap()
        .unwrap()
        .get_property_value(property)
        .unwrap()
}

fn element(node: web_sys::Node) -> web_sys::HtmlElement {
    node.dyn_into().unwrap()
}

#[wasm_bindgen_test]
fn spans_are_text_nodes_spans_and_links() {
    let runtime = Runtime::new();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(
            rich_text()
                .child(text_span("Plain "))
                .child(text_span("bold").font_bold())
                .child(text_span(" and "))
                .child(
                    text_span("struck")
                        .strikethrough(Decoration::new(1.))
                        .background(Color(255, 240, 0, 255))
                        .padding(4., 0.),
                )
                .child(text_span(" "))
                .child(text_span("a link").href("https://example.com/"))
                .child(text_span(" "))
                .child(text_span("nowhere").href("javascript:alert(1)")),
        ),
    );
    let p = paragraph(&host);
    assert_eq!(
        p.text_content().unwrap(),
        "Plain bold and struck a link nowhere"
    );
    let nodes = p.child_nodes();
    // A span with nothing of its own is its text alone.
    assert_eq!(nodes.item(0).unwrap().node_type(), web_sys::Node::TEXT_NODE);
    let bold = element(nodes.item(1).unwrap());
    assert_eq!(bold.tag_name(), "SPAN");
    assert_eq!(style(&bold, "font-weight"), "700");
    let struck = element(nodes.item(3).unwrap());
    assert!(style(&struck, "text-decoration-line").contains("line-through"));
    assert_eq!(style(&struck, "background-color"), "rgb(255, 240, 0)");
    assert_eq!(style(&struck, "padding-left"), "4px");
    // A web link: an <a> opening in a tab of its own, underlined.
    let link = element(nodes.item(5).unwrap());
    assert_eq!(link.tag_name(), "A");
    assert_eq!(
        link.get_attribute("href").as_deref(),
        Some("https://example.com/")
    );
    assert_eq!(link.get_attribute("target").as_deref(), Some("_blank"));
    assert!(style(&link, "text-decoration-line").contains("underline"));
    // A scheme the desktop wouldn't open isn't given to the browser.
    let other = element(nodes.item(7).unwrap());
    assert_eq!(other.tag_name(), "A");
    assert_eq!(other.get_attribute("href"), None);
    assert_eq!(other.get_attribute("role").as_deref(), Some("link"));
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn streamed_spans_keep_the_nodes_before_them() {
    let runtime = Runtime::new();
    let words = runtime.signal(vec!["Hello".to_owned(), " there".to_owned()]);
    let read = words.clone();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(rich_text_signal(move || {
            read.get()
                .into_iter()
                .enumerate()
                .map(|(i, word)| match i % 2 {
                    0 => text_span(word),
                    _ => text_span(word).italic(true),
                })
                .collect()
        })),
    );
    let p = paragraph(&host);
    let first = p.child_nodes().item(0).unwrap();
    let second = p.child_nodes().item(1).unwrap();
    words.update(|w| w.push(", world".into()));
    zgx_dom::flush();
    assert_eq!(p.text_content().unwrap(), "Hello there, world");
    assert!(p.child_nodes().item(0).unwrap().is_same_node(Some(&first)));
    assert!(p.child_nodes().item(1).unwrap().is_same_node(Some(&second)));
    assert_eq!(p.child_nodes().length(), 3);
    // A change in the middle makes it and what follows again.
    words.update(|w| w[1] = " you".into());
    zgx_dom::flush();
    assert_eq!(p.text_content().unwrap(), "Hello you, world");
    assert!(p.child_nodes().item(0).unwrap().is_same_node(Some(&first)));
    assert!(!p.child_nodes().item(1).unwrap().is_same_node(Some(&second)));
    let live = zgx_dom::live_nodes();
    words.set(Vec::new());
    zgx_dom::flush();
    assert_eq!(p.child_nodes().length(), 0);
    // Three spans let go: a text node, an <em>-like span and its text, a text node.
    assert_eq!(live - zgx_dom::live_nodes(), 4);
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn a_links_click_runs_and_may_keep_its_href_closed() {
    let runtime = Runtime::new();
    let (clicks, handled) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (plain, kept) = (clicks.clone(), handled.clone());
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(
            rich_text()
                .child(text_span("Run").on_click(move || plain.set(plain.get() + 1)))
                .child(text_span(" "))
                .child(
                    text_span("Docs")
                        .href("https://example.com/docs")
                        .on_click_handled(move |_| {
                            kept.set(kept.get() + 1);
                            true
                        }),
                ),
        ),
    );
    let p = paragraph(&host);
    let run = element(p.child_nodes().item(0).unwrap());
    assert_eq!(run.get_attribute("tabindex").as_deref(), Some("0"));
    run.click();
    assert_eq!(clicks.get(), 1);
    // Enter presses a link with no href, as it does a link.
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("Enter");
    init.set_bubbles(true);
    init.set_cancelable(true);
    let enter =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    run.dispatch_event(&enter).unwrap();
    assert_eq!(clicks.get(), 2);
    // Handled, its href doesn't open: the click's default is prevented.
    let docs = element(p.child_nodes().item(2).unwrap());
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    let click = web_sys::MouseEvent::new_with_mouse_event_init_dict("click", &init).unwrap();
    let not_prevented = docs.dispatch_event(&click).unwrap();
    assert_eq!(handled.get(), 1);
    assert!(!not_prevented);
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn hover_styles_icons_and_fades_are_css() {
    let runtime = Runtime::new();
    let svg = SvgData::new(
        Arc::<[u8]>::from(
            &br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10"/></svg>"#[..],
        ),
    )
    .unwrap();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(
            rich_text()
                .child(text_span("hover").on_click(|| {}).hover(SpanHover {
                    underline: Some(Decoration::new(1.)),
                    text_color: None,
                }))
                .child(text_span("file.rs").icon(SpanIcon {
                    svg: Arc::new(svg),
                    size: 12.,
                    gap: 4.,
                    tint: Some(Color(200, 0, 0, 255).into()),
                    bg: None,
                    radius: 0.,
                    pad: 0.,
                }))
                .child(text_span("new").fade_in(TextFade {
                    start: fade_now(),
                    ms: 400.,
                    power: 3.,
                })),
        ),
    );
    let p = paragraph(&host);
    let nodes = p.child_nodes();
    // A hover style in place of the underline at rest.
    let hover = element(nodes.item(0).unwrap());
    assert!(!style(&hover, "text-decoration-line").contains("underline"));
    let class = hover.class_name();
    let sheets = js_sys::eval(&format!(
        "[...document.styleSheets].flatMap(s => [...s.cssRules]).map(r => r.cssText).filter(t => t.startsWith('.{class}:hover')).join()"
    ))
    .unwrap()
    .as_string()
    .unwrap();
    assert!(sheets.contains("underline"), "{sheets}");
    // An icon before the text: its own box, the size given, tinted.
    let chip = element(nodes.item(1).unwrap());
    let icon = element(chip.first_child().unwrap());
    assert_eq!(style(&icon, "width"), "12px");
    assert_eq!(style(&icon, "margin-right"), "4px");
    assert_eq!(style(&icon, "background-color"), "rgb(200, 0, 0)");
    assert_eq!(chip.text_content().unwrap(), "file.rs");
    // A fade: a CSS animation, no script per frame.
    let fading = element(nodes.item(2).unwrap());
    assert_eq!(style(&fading, "animation-name"), "zf");
    mounted.unmount();
    host.remove();
}
