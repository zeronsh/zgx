//! Overlays as the browser's own: a modal as a `<dialog>`, popovers, menus
//! and tooltips in its top layer, their closing reported to their signals.
#![cfg(target_arch = "wasm32")]

use std::{cell::Cell, rc::Rc, time::Duration};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;
use zgx::{
    compose::{
        Tooltip, View, button, column, keyed, menu, menu_item, modal, popover, submenu, text,
    },
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

fn find(host: &web_sys::Element, selector: &str) -> web_sys::HtmlElement {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
        .dyn_into()
        .unwrap()
}

fn active() -> Option<web_sys::Element> {
    web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .active_element()
}

fn call(target: &JsValue, method: &str) {
    let f: js_sys::Function = js_sys::Reflect::get(target, &method.into())
        .unwrap()
        .dyn_into()
        .unwrap();
    f.call0(target).unwrap();
}

fn showing(panel: &web_sys::Element) -> bool {
    panel.matches(":popover-open").unwrap()
}

/// Let the browser's queued events (toggle, close) and timers run.
async fn settle(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// Wait (up to a second) for `done`: what the browser reports of an
/// overlay (toggle, close) comes in a task of its own.
async fn until(mut done: impl FnMut() -> bool) {
    for _ in 0..100 {
        if done() {
            return;
        }
        settle(10).await;
    }
    panic!("not reported in a second");
}

fn pointer(target: &web_sys::Element, kind: &str) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    let event = web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
    target.dispatch_event(&event).unwrap();
}

fn key(target: &web_sys::Element, name: &str) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key(name);
    init.set_bubbles(true);
    let event =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    target.dispatch_event(&event).unwrap();
}

#[wasm_bindgen_test]
async fn a_modal_is_a_dialog_shown_while_open() {
    let runtime = Runtime::new();
    let open = runtime.signal(false);
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(modal("Settings", open.clone()).child(text("Inside"))),
    );
    let dialog: web_sys::HtmlElement = find(&host, "dialog");
    assert_eq!(
        dialog.get_attribute("aria-label").as_deref(),
        Some("Settings")
    );
    assert!(!dialog.has_attribute("open"));
    assert_eq!(dialog.text_content().unwrap(), "Inside");
    open.set(true);
    zgx_dom::flush();
    assert!(dialog.matches(":modal").unwrap());
    // Closed by the browser (Escape, `close()`): its signal follows.
    call(&dialog, "close");
    until(|| !open.get()).await;
    open.set(true);
    zgx_dom::flush();
    assert!(dialog.matches(":modal").unwrap());
    open.set(false);
    zgx_dom::flush();
    assert!(!dialog.has_attribute("open"));
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
async fn a_popover_sits_below_its_anchor_and_its_anchor_closes_it() {
    let runtime = Runtime::new();
    let open = runtime.signal(false);
    let toggle = open.clone();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(
            popover(
                "Details",
                open.clone(),
                button().child(text("More")).on_click(move || {
                    toggle.update(|o| *o = !*o);
                }),
            )
            .child(text("Body")),
        ),
    );
    let anchor = find(&host, "button");
    let panel = find(&host, "[popover]");
    assert_eq!(panel.get_attribute("role").as_deref(), Some("dialog"));
    assert!(!showing(&panel));
    anchor.click();
    assert!(open.get());
    assert!(showing(&panel));
    until(|| anchor.get_attribute("aria-expanded").as_deref() == Some("true")).await;
    // Below it, at its start, 4 px off (`Placement`'s default).
    let (a, p) = (
        anchor.get_bounding_client_rect(),
        panel.get_bounding_client_rect(),
    );
    assert!(
        (p.top() - (a.bottom() + 4.)).abs() < 1.,
        "{} {}",
        p.top(),
        a.bottom()
    );
    assert!((p.left() - a.left()).abs() < 1.);
    // A press on its anchor closes it; the click it ends in opens nothing.
    pointer(&anchor, "pointerdown");
    anchor.click();
    assert!(!showing(&panel));
    until(|| !open.get()).await;
    settle(20).await;
    assert!(!showing(&panel), "the click opened nothing");
    // Closed by the browser (a click outside, Escape): its signal follows.
    open.set(true);
    zgx_dom::flush();
    assert!(showing(&panel));
    call(&panel, "hidePopover");
    until(|| !open.get()).await;
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
async fn menus_open_from_their_button_and_close_as_an_item_is_chosen() {
    let runtime = Runtime::new();
    let (open, recent) = (runtime.signal(false), runtime.signal(false));
    let chosen = Rc::new(Cell::new(""));
    let (new, file) = (chosen.clone(), chosen.clone());
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(
            menu("File", open.clone(), button().child(text("File")))
                .child(menu_item("New").on_click(move || new.set("new")))
                .child(
                    submenu("Recent", recent.clone())
                        .child(menu_item("notes.txt").on_click(move || file.set("notes"))),
                ),
        ),
    );
    let anchor = find(&host, "button");
    let panel = find(&host, "[role=menu]");
    assert_eq!(
        anchor.get_attribute("aria-haspopup").as_deref(),
        Some("menu")
    );
    // A bare button opening a menu: a press opens it, focus on its first item.
    anchor.click();
    assert!(open.get());
    assert!(showing(&panel));
    let items = panel.query_selector_all("[role=menuitem]").unwrap();
    let item = |i: u32| -> web_sys::HtmlElement { items.item(i).unwrap().dyn_into().unwrap() };
    assert_eq!(item(0).text_content().unwrap(), "New");
    assert_eq!(active().as_ref(), Some(item(0).unchecked_ref()));
    // The arrows move among its own items, wrapping.
    key(&item(0), "ArrowDown");
    assert_eq!(active().as_ref(), Some(item(1).unchecked_ref()));
    key(&item(1), "ArrowDown");
    assert_eq!(active().as_ref(), Some(item(0).unchecked_ref()));
    key(&item(0), "ArrowUp");
    let trigger = item(1);
    assert!(trigger.text_content().unwrap().starts_with("Recent"));
    assert_eq!(
        trigger.get_attribute("aria-haspopup").as_deref(),
        Some("menu")
    );
    // Right opens the submenu, focus on its first item; Left closes it.
    key(&trigger, "ArrowRight");
    let sub = find(&panel, "[role=menu]");
    assert!(showing(&sub));
    until(|| recent.get()).await;
    assert!(showing(&panel), "its menu stays open under it");
    let notes = item(2);
    assert_eq!(notes.text_content().unwrap(), "notes.txt");
    assert_eq!(active().as_ref(), Some(notes.unchecked_ref()));
    key(&notes, "ArrowLeft");
    assert!(!showing(&sub));
    until(|| !recent.get()).await;
    assert_eq!(active().as_ref(), Some(trigger.unchecked_ref()));
    // Choosing an item closes every menu it is in, then runs its click,
    // and focus goes back to the menu's button.
    key(&trigger, "ArrowRight");
    until(|| recent.get()).await;
    notes.click();
    assert_eq!(chosen.get(), "notes");
    assert!(!open.get() && !recent.get());
    assert!(!showing(&panel) && !showing(&sub));
    settle(0).await;
    assert_eq!(active().as_ref(), Some(anchor.unchecked_ref()));
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
async fn a_tooltip_shows_as_the_pointer_rests_and_goes_on_a_press() {
    let runtime = Runtime::new();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().pt(120.).child(
            button()
                .child(text("Save"))
                .tooltip(Tooltip::from("Saves the file").delay(Duration::from_millis(20))),
        ),
    );
    let anchor = find(&host, "button");
    let tip = find(&host, "[role=tooltip]");
    assert_eq!(
        anchor.get_attribute("aria-description").as_deref(),
        Some("Saves the file")
    );
    assert_eq!(tip.get_attribute("aria-hidden").as_deref(), Some("true"));
    pointer(&anchor, "pointerover");
    assert!(!showing(&tip), "not before its delay");
    settle(60).await;
    assert!(showing(&tip));
    assert_eq!(tip.text_content().unwrap(), "Saves the file");
    // Above it, centred (a tooltip's default).
    let (a, t) = (
        anchor.get_bounding_client_rect(),
        tip.get_bounding_client_rect(),
    );
    assert!(t.bottom() <= a.top() + 0.5);
    pointer(&anchor, "pointerdown");
    assert!(!showing(&tip));
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn overlays_in_keyed_rows_each_keep_their_own() {
    let runtime = Runtime::new();
    let rows = runtime.signal(vec![1_u32, 2, 3]);
    let opens: Vec<_> = (0..4).map(|_| runtime.signal(false)).collect();
    let row_opens = opens.clone();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        keyed(
            move || rows.get(),
            move |n: u32, _| -> View {
                popover(
                    format!("Row {n}"),
                    row_opens[n as usize].clone(),
                    button().child(text(format!("Row {n}"))),
                )
                .child(text(format!("About {n}")))
            },
        ),
    );
    let panels = host.query_selector_all("[popover]").unwrap();
    assert_eq!(panels.length(), 3);
    opens[2].set(true);
    opens[3].set(true);
    zgx_dom::flush();
    let third: web_sys::Element = panels.item(2).unwrap().dyn_into().unwrap();
    assert_eq!(third.text_content().unwrap(), "About 3");
    assert!(showing(&third));
    let second: web_sys::Element = panels.item(1).unwrap().dyn_into().unwrap();
    // Showing the third (an auto popover) closed the second's.
    assert!(!showing(&second));
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
async fn a_popovers_arrow_points_at_its_anchor() {
    let runtime = Runtime::new();
    let open = runtime.signal(false);
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().pt(20.).child(
            popover("Info", open.clone(), button().w(60.).child(text("i")))
                .arrow(8.)
                .child(text("Body")),
        ),
    );
    open.set(true);
    zgx_dom::flush();
    let (anchor, panel) = (find(&host, "button"), find(&host, "[popover]"));
    let (a, p) = (
        anchor.get_bounding_client_rect(),
        panel.get_bounding_client_rect(),
    );
    // Below the anchor, 4 (the offset) and 8 (the arrow's room) off.
    assert!(
        (p.top() - (a.bottom() + 12.)).abs() < 1.,
        "{} {}",
        p.top(),
        a.bottom()
    );
    // Its arrow: at the top edge, under the anchor's centre, the panel's
    // own background.
    let arrow = web_sys::window()
        .unwrap()
        .get_computed_style_with_pseudo_elt(&panel, "::before")
        .unwrap()
        .unwrap();
    let px = |v: String| v.trim_end_matches("px").parse::<f64>().unwrap();
    assert_eq!(px(arrow.get_property_value("top").unwrap()), 0.);
    let tip = p.left() + px(arrow.get_property_value("left").unwrap());
    assert!((tip - (a.left() + a.width() / 2.)).abs() < 1., "{tip}");
    let panel_style = web_sys::window()
        .unwrap()
        .get_computed_style(&panel)
        .unwrap()
        .unwrap();
    assert_eq!(
        arrow.get_property_value("background-color").unwrap(),
        panel_style.get_property_value("background-color").unwrap()
    );
    // Not clipped by the panel: it shows past its edge.
    assert_eq!(
        panel_style.get_property_value("overflow").unwrap(),
        "visible"
    );
    open.set(false);
    zgx_dom::flush();
    mounted.unmount();
    host.remove();
}
