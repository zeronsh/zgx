//! `on_event` listeners, keymaps and focus groups in the DOM: the browser's
//! events routed to zgx's listeners in their phases, keys to keymaps first,
//! and arrows moving within a group.
#![cfg(target_arch = "wasm32")]

use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use zgx::{
    actions::{Action, KeyBinding, Keymap, Keystroke},
    compose::{button, column, radio, radio_group, row, text},
    input::{EventPhase, FocusGroup, InputEvent, Key, Modifiers},
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

fn all(host: &web_sys::Element, selector: &str) -> Vec<web_sys::HtmlElement> {
    let list = host.query_selector_all(selector).unwrap();
    (0..list.length())
        .map(|i| list.item(i).unwrap().dyn_into().unwrap())
        .collect()
}

fn active() -> Option<web_sys::Element> {
    web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .active_element()
}

/// A key pressed at `target`: whether nothing prevented its default.
fn key(target: &web_sys::Element, name: &str, control: bool) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key(name);
    init.set_ctrl_key(control);
    init.set_bubbles(true);
    init.set_cancelable(true);
    let event =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    target.dispatch_event(&event).unwrap()
}

fn pointer(target: &web_sys::Element, kind: &str) -> bool {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_client_x(12);
    init.set_client_y(34);
    let event = web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
    target.dispatch_event(&event).unwrap()
}

async fn settle() {
    let promise = js_sys::Promise::resolve(&wasm_bindgen::JsValue::UNDEFINED);
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

#[wasm_bindgen_test]
fn listeners_hear_each_phase_and_their_decisions_apply() {
    let runtime = Runtime::new();
    let heard: Rc<RefCell<Vec<String>>> = Rc::default();
    let (outer_log, inner_log) = (heard.clone(), heard.clone());
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column()
            .id("outer")
            .p(20.)
            .on_event(move |cx| {
                if let InputEvent::PointerDown { x, y, .. } = cx.event {
                    outer_log.borrow_mut().push(format!(
                        "outer {:?} {x},{y} self={}",
                        cx.phase,
                        cx.target == cx.current_target
                    ));
                }
                if let InputEvent::KeyDown {
                    key: Key::Escape, ..
                } = cx.event
                    && cx.phase == EventPhase::Bubble
                {
                    outer_log.borrow_mut().push("outer escape".into());
                }
            })
            .child(column().id("inner").focusable(true).h(20.).on_event(
                move |cx| match &cx.event {
                    InputEvent::PointerDown { .. } => {
                        inner_log.borrow_mut().push(format!("inner {:?}", cx.phase));
                    }
                    InputEvent::KeyDown {
                        key: Key::Character(c),
                        ..
                    } if c == "x" => {
                        cx.prevent_default();
                        cx.stop_propagation();
                    }
                    _ => {}
                },
            )),
    );
    let inner = find(&host, "[data-id=inner]");
    assert!(pointer(&inner, "pointerdown"));
    assert_eq!(
        *heard.borrow(),
        [
            "outer Capture 12,34 self=false",
            "inner Target",
            "outer Bubble 12,34 self=false",
        ]
    );
    // A listener's prevent_default is the event's; stop_propagation keeps
    // it from the outer element.
    heard.borrow_mut().clear();
    assert!(!key(&inner, "x", false));
    assert!(key(&inner, "Escape", false));
    assert_eq!(*heard.borrow(), ["outer escape"]);
    mounted.unmount();
    host.remove();
}

#[derive(Clone, Debug)]
struct Save;
#[derive(Clone, Debug)]
struct Find;

#[wasm_bindgen_test]
fn keymaps_take_keys_first_and_route_their_actions() {
    let runtime = Runtime::new();
    let done: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let (saved, found) = (done.clone(), done.clone());
    let control = Modifiers {
        control: true,
        ..Default::default()
    };
    let at = |c: &str| Keystroke::new(Key::Character(c.into()), control);
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column()
            .keymap(
                Keymap::new()
                    .bind(KeyBinding::new([at("s")], Action::new(Save)).unwrap())
                    .bind(KeyBinding::new([at("k"), at("f")], Action::new(Find)).unwrap()),
            )
            .on_action::<Save>(move |_, cx| {
                saved.borrow_mut().push("save");
                cx.prevent_default();
            })
            .on_action::<Find>(move |_, cx| {
                found.borrow_mut().push("find");
                cx.prevent_default();
            })
            .child(button().id("field").child(text("Focus here"))),
    );
    let field = find(&host, "[data-id=field]");
    field.focus().unwrap();
    // Matched: its action handled, the key's default prevented.
    assert!(!key(&field, "s", true));
    assert_eq!(*done.borrow(), ["save"]);
    // A sequence: its first key taken, its last runs the action.
    assert!(!key(&field, "k", true));
    assert_eq!(*done.borrow(), ["save"]);
    assert!(!key(&field, "f", true));
    assert_eq!(*done.borrow(), ["save", "find"]);
    // Bound to nothing: the key goes on.
    assert!(key(&field, "q", true));
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
async fn a_focus_group_is_one_tab_stop_moved_by_its_arrows() {
    let runtime = Runtime::new();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        row()
            .focus_group(FocusGroup::Horizontal)
            .child(button().child(text("One")))
            .child(button().child(text("Two")))
            .child(button().child(text("Three"))),
    );
    settle().await;
    let buttons = all(&host, "button");
    let tabs: Vec<i32> = buttons.iter().map(|b| b.tab_index()).collect();
    assert_eq!(tabs, [0, -1, -1]);
    buttons[0].focus().unwrap();
    assert!(!key(&buttons[0], "ArrowRight", false));
    assert_eq!(active().as_ref(), Some(buttons[1].unchecked_ref()));
    let tabs: Vec<i32> = buttons.iter().map(|b| b.tab_index()).collect();
    assert_eq!(tabs, [-1, 0, -1]);
    // Up and Down aren't this group's; End is.
    assert!(key(&buttons[1], "ArrowDown", false));
    assert!(!key(&buttons[1], "End", false));
    assert_eq!(active().as_ref(), Some(buttons[2].unchecked_ref()));
    // It doesn't wrap.
    assert!(key(&buttons[2], "ArrowRight", false));
    assert_eq!(active().as_ref(), Some(buttons[2].unchecked_ref()));
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
async fn a_radio_groups_arrows_wrap_and_choose() {
    let runtime = Runtime::new();
    let choice = runtime.signal(0_u8);
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        radio_group("Size")
            .child(radio("Small", choice.clone(), 0))
            .child(radio("Medium", choice.clone(), 1))
            .child(radio("Large", choice.clone(), 2)),
    );
    settle().await;
    let radios = all(&host, "[role=radio]");
    assert_eq!(radios.len(), 3);
    radios[0].focus().unwrap();
    assert!(!key(&radios[0], "ArrowUp", false));
    assert_eq!(active().as_ref(), Some(radios[2].unchecked_ref()));
    assert_eq!(choice.get(), 2);
    mounted.unmount();
    host.remove();
}

#[derive(Debug)]
struct Card(u32);

fn pointer_at(target: &web_sys::Element, kind: &str, x: f64, y: f64) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    init.set_is_primary(true);
    let event = web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
    target.dispatch_event(&event).unwrap();
}

#[wasm_bindgen_test]
fn a_drag_from_a_listener_drops_on_what_takes_it() {
    let runtime = Runtime::new();
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let (dropped, ended) = (log.clone(), log.clone());
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column()
            .gap(20.)
            .child(
                column()
                    .id("card")
                    .w(80.)
                    .h(30.)
                    .on_drag(|| Card(7))
                    .on_drag_end::<Card>(move |card, accepted| {
                        ended
                            .borrow_mut()
                            .push(format!("end {} {accepted}", card.0));
                    })
                    .child(text("Card")),
            )
            .child(
                column()
                    .id("bin")
                    .w(120.)
                    .h(60.)
                    .on_drop::<Card>(move |card, _| {
                        dropped.borrow_mut().push(format!("dropped {}", card.0));
                    })
                    .child(text("Bin")),
            ),
    );
    let (card, bin) = (find(&host, "[data-id=card]"), find(&host, "[data-id=bin]"));
    let (c, b) = (
        card.get_bounding_client_rect(),
        bin.get_bounding_client_rect(),
    );
    let (cx, cy) = (c.left() + 10., c.top() + 10.);
    pointer_at(&card, "pointerdown", cx, cy);
    // Past the threshold: the drag starts, a copy following the pointer.
    pointer_at(&card, "pointermove", cx + 12., cy + 12.);
    let ghosts = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .body()
        .unwrap()
        .last_element_child()
        .unwrap();
    assert_eq!(ghosts.text_content().unwrap(), "Card");
    let (bx, by) = (b.left() + 30., b.top() + 30.);
    pointer_at(&bin, "pointermove", bx, by);
    pointer_at(&bin, "pointerup", bx, by);
    assert_eq!(*log.borrow(), ["dropped 7", "end 7 true"]);
    // Dropped where nothing takes it: not accepted.
    log.borrow_mut().clear();
    pointer_at(&card, "pointerdown", cx, cy);
    pointer_at(&card, "pointermove", cx + 12., cy + 12.);
    let body = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .body()
        .unwrap();
    pointer_at(&body, "pointerup", c.right() + 300., cy);
    assert_eq!(*log.borrow(), ["end 7 false"]);
    mounted.unmount();
    host.remove();
}
