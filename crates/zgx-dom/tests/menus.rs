//! Context menus (`View::context_menu`, `context_menu_with_span`,
//! `TextSpan::context_menu`) in the DOM: a menu of the entries built for a
//! secondary click, at the pointer, the browser's own kept when none are.
#![cfg(target_arch = "wasm32")]

use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use zgx::{
    compose::{View, column, keyed, menu_item, rich_text, text, text_span},
    reactive::Runtime,
    style::Styled,
};

wasm_bindgen_test_configure!(run_in_browser);

fn host() -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
}

fn find(root: &web_sys::Element, selector: &str) -> web_sys::HtmlElement {
    root.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
        .dyn_into()
        .unwrap()
}

/// A secondary click at the middle of `target`: whether the browser's own
/// menu would show (nothing prevented it).
fn secondary_click(target: &web_sys::Element) -> (bool, f64, f64) {
    let r = target.get_bounding_client_rect();
    let (x, y) = (r.left() + r.width() / 2., r.top() + r.height() / 2.);
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    init.set_button(2);
    let event = web_sys::MouseEvent::new_with_mouse_event_init_dict("contextmenu", &init).unwrap();
    (target.dispatch_event(&event).unwrap(), x.floor(), y.floor())
}

fn open_menu() -> Option<web_sys::Element> {
    web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .query_selector("[role=menu]:popover-open")
        .unwrap()
}

#[wasm_bindgen_test]
fn a_views_context_menu_opens_at_the_pointer() {
    let runtime = Runtime::new();
    let chosen: Rc<RefCell<Vec<String>>> = Rc::default();
    let (log, seen) = (chosen.clone(), chosen.clone());
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(
            column()
                .id("target")
                .w(200.)
                .h(60.)
                .context_menu(move |at| {
                    seen.borrow_mut()
                        .push(format!("asked {:.0},{:.0}", at.local_x, at.local_y));
                    let log = log.clone();
                    vec![
                        menu_item("Copy").on_click(move || log.borrow_mut().push("copy".into())),
                        menu_item("Paste"),
                    ]
                })
                .child(text("Right-click me")),
        ),
    );
    let target = find(&host, "[data-id=target]");
    let (browser_menu, x, y) = secondary_click(&target);
    assert!(!browser_menu, "its menu takes the place of the browser's");
    assert_eq!(chosen.borrow()[0], "asked 100,30");
    let menu = open_menu().expect("a menu showing");
    assert_eq!(menu.text_content().unwrap(), "CopyPaste");
    let r = menu.get_bounding_client_rect();
    assert!(
        (r.left() - x).abs() <= 1. && (r.top() - y).abs() <= 1.,
        "at the pointer"
    );
    // Choosing an entry closes it and runs its click.
    find(&menu, "[role=menuitem]").click();
    assert_eq!(chosen.borrow()[1], "copy");
    assert!(open_menu().is_none());
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn no_entries_leave_the_browsers_menu() {
    let runtime = Runtime::new();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(
            column()
                .id("target")
                .h(20.)
                .context_menu(|_| Vec::new())
                .child(text("Nothing")),
        ),
    );
    let (browser_menu, _, _) = secondary_click(&find(&host, "[data-id=target]"));
    assert!(browser_menu);
    assert!(open_menu().is_none());
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn spans_have_menus_of_their_own_and_a_paragraph_knows_which() {
    let runtime = Runtime::new();
    let asked: Rc<RefCell<Vec<String>>> = Rc::default();
    let log = asked.clone();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(
            View::from(
                rich_text()
                    .child(text_span("Plain "))
                    .child(
                        text_span("own")
                            .font_bold()
                            .context_menu(|_| vec![menu_item("Span's own")]),
                    )
                    .child(text_span(" and "))
                    .child(text_span("a link").href("https://example.com/")),
            )
            .context_menu_with_span(move |_, span| {
                log.borrow_mut().push(match span {
                    Some(span) => format!("{} {:?} {}", span.index, span.href, span.text),
                    None => "none".into(),
                });
                vec![menu_item("Paragraph's")]
            }),
        ),
    );
    let paragraph = find(&host, "a").parent_element().unwrap();
    let own = paragraph.children().item(0).unwrap();
    secondary_click(&own);
    assert_eq!(open_menu().unwrap().text_content().unwrap(), "Span's own");
    assert!(asked.borrow().is_empty());
    secondary_click(&find(&host, "a"));
    assert_eq!(open_menu().unwrap().text_content().unwrap(), "Paragraph's");
    assert_eq!(
        asked.borrow()[0],
        r#"3 Some("https://example.com/") a link"#
    );
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn rows_stamped_from_a_template_keep_their_context_menus() {
    let runtime = Runtime::new();
    let rows = runtime.signal(vec![1_u32, 2, 3, 4]);
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(keyed(
            move || rows.get(),
            |n: u32, _| {
                column()
                    .id(format!("row{n}"))
                    .h(20.)
                    .context_menu(move |_| vec![menu_item(format!("Row {n}'s"))])
                    .child(text(format!("Row {n}")))
            },
        )),
    );
    // Rows after the first two are stamped from a template.
    let (browser_menu, _, _) = secondary_click(&find(&host, "[data-id=row4]"));
    assert!(!browser_menu);
    assert_eq!(open_menu().unwrap().text_content().unwrap(), "Row 4's");
    mounted.unmount();
    host.remove();
}
