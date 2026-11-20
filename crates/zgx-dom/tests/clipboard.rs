//! The browser's clipboard behind zgx's `Platform`: what a view writes, and
//! what a paste brings to an action a keymap binds to ⌘V or Ctrl+V.
#![cfg(target_arch = "wasm32")]

use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use zgx::{
    actions::{Action, KeyBinding, Keymap, Keystroke},
    compose::{button, column, component, text},
    input::{Key, Modifiers},
    platform::PlatformService,
    reactive::Runtime,
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

#[derive(Debug)]
struct Paste;

/// What the clipboard held as read: its text and its HTML.
type Reads = Rc<RefCell<Vec<(Option<String>, Option<String>)>>>;

#[wasm_bindgen_test]
fn a_view_writes_text_and_html_and_reads_them_back() {
    let runtime = Runtime::new();
    let read: Reads = Rc::default();
    let log = read.clone();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        component(move |cx| {
            let platform = cx.service::<PlatformService>();
            let log = log.clone();
            button().id("copy").child(text("Copy")).on_click(move || {
                platform
                    .0
                    .set_clipboard_rich_text("bold".into(), "<b>bold</b>".into());
                log.borrow_mut()
                    .push((platform.0.clipboard_text(), platform.0.clipboard_html()));
            })
        }),
    );
    find(&host, "[data-id=copy]").click();
    assert_eq!(
        *read.borrow(),
        [(Some("bold".into()), Some("<b>bold</b>".into()))]
    );
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn a_paste_key_bound_by_a_keymap_reads_what_is_pasted() {
    let runtime = Runtime::new();
    let pasted: Reads = Rc::default();
    let log = pasted.clone();
    let control = Modifiers {
        control: true,
        ..Default::default()
    };
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        component(move |cx| {
            let platform = cx.service::<PlatformService>();
            let log = log.clone();
            column()
                .keymap(
                    Keymap::new().bind(
                        KeyBinding::new(
                            [Keystroke::new(Key::Character("v".into()), control)],
                            Action::new(Paste),
                        )
                        .unwrap(),
                    ),
                )
                .on_action::<Paste>(move |_, cx| {
                    log.borrow_mut()
                        .push((platform.0.clipboard_text(), platform.0.clipboard_html()));
                    cx.prevent_default();
                })
                .child(button().id("field").child(text("Paste here")))
        }),
    );
    let field = find(&host, "[data-id=field]");
    field.focus().unwrap();
    // The key itself: nothing runs yet (the page can't read the clipboard).
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("v");
    init.set_ctrl_key(true);
    init.set_bubbles(true);
    init.set_cancelable(true);
    let key = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    assert!(field.dispatch_event(&key).unwrap());
    assert!(pasted.borrow().is_empty());
    // The paste it makes: the action runs, reading what is pasted, and
    // the paste goes no further.
    let data = web_sys::DataTransfer::new().unwrap();
    data.set_data("text/plain", "pasted").unwrap();
    data.set_data("text/html", "<i>pasted</i>").unwrap();
    let init = web_sys::ClipboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_clipboard_data(Some(&data));
    let paste = web_sys::ClipboardEvent::new_with_event_init_dict("paste", &init).unwrap();
    assert!(!field.dispatch_event(&paste).unwrap());
    assert_eq!(
        *pasted.borrow(),
        [(Some("pasted".into()), Some("<i>pasted</i>".into()))]
    );
    mounted.unmount();
    host.remove();
}

/// What the clipboard held as read: its types, and the value of one.
type TypeReads = Rc<RefCell<Vec<(Vec<String>, Option<String>)>>>;

/// A paste's types beyond text and HTML (a page's copy that set its own)
/// read back as it brings them; a write with types copies them where it
/// can (a gesture's `execCommand("copy")`), and without one its text still
/// goes, as a plain write's does.
#[wasm_bindgen_test]
fn a_pastes_types_read_back_and_a_typed_write_keeps_its_text() {
    let runtime = Runtime::new();
    let read: TypeReads = Rc::default();
    let log = read.clone();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        component(move |cx| {
            let platform = cx.service::<PlatformService>();
            let (log, written) = (log.clone(), platform.clone());
            column()
                .child(button().id("read").child(text("Read")).on_click(move || {
                    let types = platform.0.clipboard_types();
                    let notes = platform.0.clipboard_typed("application/x-notes");
                    log.borrow_mut().push((types, notes));
                }))
                .child(button().id("write").child(text("Write")).on_click(move || {
                    let types = [("application/x-notes".to_owned(), "written".to_owned())];
                    assert!(written.0.set_clipboard_typed("t".into(), None, &types));
                    assert_eq!(written.0.clipboard_text().as_deref(), Some("t"));
                    let typed = written.0.clipboard_typed("application/x-notes");
                    // Copied, the types read back; refused, none are left.
                    assert!(
                        matches!(typed.as_deref(), None | Some("written")),
                        "{typed:?}"
                    );
                }))
        }),
    );
    let data = web_sys::DataTransfer::new().unwrap();
    data.set_data("text/plain", "pasted").unwrap();
    data.set_data("application/x-notes", "from a page 😀\n\n")
        .unwrap();
    let init = web_sys::ClipboardEventInit::new();
    init.set_bubbles(true);
    init.set_clipboard_data(Some(&data));
    let paste = web_sys::ClipboardEvent::new_with_event_init_dict("paste", &init).unwrap();
    find(&host, "[data-id=read]")
        .dispatch_event(&paste)
        .unwrap();
    find(&host, "[data-id=read]").click();
    assert_eq!(
        *read.borrow(),
        [(
            vec!["application/x-notes".to_owned()],
            Some("from a page 😀\n\n".to_owned())
        )]
    );
    find(&host, "[data-id=write]").click();
    mounted.unmount();
    host.remove();
}
