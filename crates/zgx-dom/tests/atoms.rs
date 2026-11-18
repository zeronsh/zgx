//! A text area with atoms on the DOM (`View::atoms_when`, `atom_wash`): a
//! contenteditable over its model, its atoms chips, its edits the model's
//! (`beforeinput`, an input method's end, undo); one without atoms is the
//! browser's own field, as before.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use zgx::{
    compose::{column, text_area},
    reactive::{Runtime, Signal},
    scene::Color,
    style::Styled,
    text_edit::Atom,
};

wasm_bindgen_test_configure!(run_in_browser);

fn host() -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
}

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// The text area, mounted with `text`, one atom over `atom` washed.
fn note(
    text: &str,
    atom: Option<(usize, usize)>,
) -> (zgx_dom::Mounted, web_sys::HtmlElement, Signal<String>) {
    let runtime = Runtime::new();
    let value = runtime.signal(text.to_owned());
    let host = host();
    let mut area = text_area("Message", value.clone()).id("message").w(300.);
    if let Some((from, to)) = atom {
        area = area
            .atoms_when(move |editor| {
                let id = editor.new_atom_id();
                vec![Atom::new(id, from..to)]
            })
            .atom_wash(Color(200, 220, 255, 255));
    }
    let mounted = zgx_dom::mount(&runtime, &host, column().child(area));
    zgx_dom::flush();
    let el = host
        .query_selector(".ze")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    (mounted, el, value)
}

/// The caret at `at` UTF-16 units into `el`'s text (a text node's).
fn caret_at_end(el: &web_sys::HtmlElement) {
    el.focus().unwrap();
    let range = document().create_range().unwrap();
    range.select_node_contents(el).unwrap();
    range.collapse_with_to_start(false);
    let selection = document().get_selection().unwrap().unwrap();
    selection.remove_all_ranges().unwrap();
    selection.add_range(&range).unwrap();
}

fn before_input(el: &web_sys::HtmlElement, kind: &str, data: Option<&str>) {
    let init = web_sys::InputEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_input_type(kind);
    init.set_data(data);
    let event = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
    el.dispatch_event(&event).unwrap();
    zgx_dom::flush();
}

fn chips(el: &web_sys::HtmlElement) -> Vec<String> {
    let list = el.query_selector_all("[data-atom]").unwrap();
    (0..list.length())
        .map(|i| list.item(i).unwrap().text_content().unwrap_or_default())
        .collect()
}

#[wasm_bindgen_test]
fn a_text_area_without_atoms_is_still_the_browsers_field() {
    let (_mounted, el, _) = note("plain", None);
    assert_eq!(el.tag_name(), "TEXTAREA");
    assert!(el.get_attribute("contenteditable").is_none());
}

#[wasm_bindgen_test]
fn its_atoms_show_as_chips_in_its_text() {
    let (_mounted, el, _) = note("ask Ada now", Some((4, 7)));
    assert_eq!(el.tag_name(), "DIV");
    assert_eq!(el.get_attribute("contenteditable").as_deref(), Some("true"));
    assert_eq!(el.get_attribute("role").as_deref(), Some("textbox"));
    assert_eq!(el.text_content().unwrap(), "ask Ada now");
    assert_eq!(chips(&el), ["Ada"]);
    let chip: web_sys::HtmlElement = el
        .query_selector("[data-atom]")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    assert_eq!(
        chip.get_attribute("contenteditable").as_deref(),
        Some("false")
    );
    assert!(
        chip.style()
            .get_property_value("background")
            .unwrap()
            .contains("rgb"),
        "washed"
    );
}

#[wasm_bindgen_test]
fn typing_edits_the_model_and_the_text_follows() {
    let (_mounted, el, value) = note("ask Ada now", Some((4, 7)));
    caret_at_end(&el);
    before_input(&el, "insertText", Some("!"));
    assert_eq!(value.get(), "ask Ada now!");
    assert_eq!(el.text_content().unwrap(), "ask Ada now!");
    assert_eq!(chips(&el), ["Ada"], "the chip kept");
}

#[wasm_bindgen_test]
fn backspace_after_a_chip_deletes_it_whole_and_undo_brings_it_back() {
    let (_mounted, el, value) = note("ask Ada", Some((4, 7)));
    caret_at_end(&el);
    before_input(&el, "deleteContentBackward", None);
    assert_eq!(value.get(), "ask ");
    assert!(chips(&el).is_empty());
    before_input(&el, "historyUndo", None);
    assert_eq!(value.get(), "ask Ada");
    assert_eq!(chips(&el), ["Ada"], "back, a chip again");
}

#[wasm_bindgen_test]
fn an_input_methods_text_is_the_models_once_committed() {
    let (_mounted, el, value) = note("ask Ada ", Some((4, 7)));
    caret_at_end(&el);
    let compose = |kind: &str, data: &str| {
        let init = web_sys::CompositionEventInit::new();
        init.set_bubbles(true);
        init.set_data(data);
        let event = web_sys::CompositionEvent::new_with_event_init_dict(kind, &init).unwrap();
        el.dispatch_event(&event).unwrap();
        zgx_dom::flush();
    };
    compose("compositionstart", "");
    // The browser writes the composing text into the element itself.
    el.append_child(&document().create_text_node("にほん"))
        .unwrap();
    before_input(&el, "insertCompositionText", Some("にほん"));
    assert_eq!(value.get(), "ask Ada ", "not the model's while composing");
    compose("compositionend", "日本");
    assert_eq!(value.get(), "ask Ada 日本");
    assert_eq!(
        el.text_content().unwrap(),
        "ask Ada 日本",
        "shown once, as the model has it"
    );
    assert_eq!(chips(&el), ["Ada"]);
}

#[wasm_bindgen_test]
fn a_value_set_from_outside_shows_and_keeps_atoms_off_it() {
    let (_mounted, el, value) = note("ask Ada now", Some((4, 7)));
    value.set("hello".into());
    zgx_dom::flush();
    assert_eq!(el.text_content().unwrap(), "hello");
}
