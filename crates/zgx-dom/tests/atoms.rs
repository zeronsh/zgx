//! A text area with atoms on the DOM (`View::atoms_when`, `atom_wash`): a
//! contenteditable over its model, its atoms chips, its edits the model's
//! (`beforeinput`, an input method's end, undo); one without atoms is the
//! browser's own field, as before.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use zgx::{
    compose::{column, keyed, text_area, text_input},
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

/// A `beforeinput` of `kind` whose target range (`getTargetRanges`) is
/// `from..to` UTF-16 units into `node`, a text node of the field's.
fn before_input_at(
    el: &web_sys::HtmlElement,
    kind: &str,
    data: Option<&str>,
    node: &web_sys::Node,
    from: u32,
    to: u32,
) {
    let init = web_sys::InputEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_input_type(kind);
    init.set_data(data);
    let range =
        web_sys::StaticRange::new(&web_sys::StaticRangeInit::new(node, to, node, from)).unwrap();
    init.set_target_ranges(&js_sys::Array::of1(&range));
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
fn a_value_set_from_outside_shows_and_takes_the_atoms_it_replaced() {
    let (_mounted, el, value) = note("ask Ada now", Some((4, 7)));
    value.set("hello".into());
    zgx_dom::flush();
    assert_eq!(el.text_content().unwrap(), "hello");
    assert!(chips(&el).is_empty(), "its text replaced, its atom gone");
}

/// The caret's place in `el`'s text, in UTF-16 units, as the browser has it.
fn caret(el: &web_sys::HtmlElement) -> u32 {
    let selection = document().get_selection().unwrap().unwrap();
    let range = document().create_range().unwrap();
    range.set_start(el, 0).unwrap();
    range
        .set_end(&selection.anchor_node().unwrap(), selection.anchor_offset())
        .unwrap();
    String::from(range.to_string()).encode_utf16().count() as u32
}

#[wasm_bindgen_test]
fn its_box_is_a_box_it_takes_the_focus_and_its_caret_is_the_models() {
    let (_mounted, el, value) = note("ask Ada now", Some((4, 7)));
    let display = web_sys::window()
        .unwrap()
        .get_computed_style(&el)
        .unwrap()
        .unwrap()
        .get_property_value("display")
        .unwrap();
    assert_ne!(display, "contents", "a box of its own");
    el.focus().unwrap();
    assert_eq!(document().active_element().as_ref(), Some(el.as_ref()));
    caret_at_end(&el);
    before_input(&el, "insertText", Some("!"));
    assert_eq!(value.get(), "ask Ada now!");
    assert_eq!(
        caret(&el),
        12,
        "the caret after what was typed, as the model has it"
    );
    // Past the chip: the caret after it, not in it.
    before_input(&el, "deleteContentBackward", None);
    let at = document().create_range().unwrap();
    at.set_start(&el, 0).unwrap();
    at.set_end(&el, 0).unwrap();
    let selection = document().get_selection().unwrap().unwrap();
    selection.remove_all_ranges().unwrap();
    selection.add_range(&at).unwrap();
    before_input(&el, "insertText", Some("x"));
    assert_eq!(value.get(), "xask Ada now");
    assert_eq!(caret(&el), 1);
}

#[wasm_bindgen_test]
fn its_placeholder_shows_while_it_is_empty() {
    let runtime = Runtime::new();
    let value = runtime.signal(String::new());
    let host = host();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        text_area("Message", value)
            .placeholder("Ask anything")
            .atoms_when(|_| Vec::new())
            .atom_wash(Color(200, 220, 255, 255)),
    );
    zgx_dom::flush();
    let el = host.query_selector(".ze").unwrap().unwrap();
    assert_eq!(el.get_attribute("contenteditable").as_deref(), Some("true"));
    assert_eq!(
        el.get_attribute("data-placeholder").as_deref(),
        Some("Ask anything")
    );
}

#[wasm_bindgen_test]
fn a_single_line_field_takes_no_line_breaks() {
    let runtime = Runtime::new();
    let value = runtime.signal(String::from("ask"));
    let host = host();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        text_input("Name", value.clone())
            .atoms_when(|_| Vec::new())
            .atom_wash(Color(200, 220, 255, 255)),
    );
    zgx_dom::flush();
    let el: web_sys::HtmlElement = host
        .query_selector(".ze")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    caret_at_end(&el);
    before_input(&el, "insertFromPaste", Some(" two\nlines"));
    assert!(!value.get().contains('\n'), "joined: {:?}", value.get());
    before_input(&el, "insertParagraph", None);
    assert!(!value.get().contains('\n'));
}

#[wasm_bindgen_test]
fn text_areas_with_atoms_in_a_list_each_show_theirs() {
    let runtime = Runtime::new();
    let host = host();
    let rows = runtime.clone();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        keyed(
            || vec![1, 2],
            move |id: i32, _| {
                let value = rows.signal(format!("row {id} Ada"));
                text_area("Row", value)
                    .atoms_when(|editor| {
                        let id = editor.new_atom_id();
                        let at = editor.text().find("Ada").unwrap();
                        vec![Atom::new(id, at..at + 3)]
                    })
                    .atom_wash(Color(200, 220, 255, 255))
            },
        ),
    );
    zgx_dom::flush();
    let list = host.query_selector_all("[data-atom]").unwrap();
    assert_eq!(list.length(), 2, "a chip in each row");
    let fields = host.query_selector_all(".ze").unwrap();
    assert_eq!(fields.length(), 2);
}

#[wasm_bindgen_test]
fn a_replacement_takes_its_target_range_and_a_delete_the_models() {
    let (_mounted, el, value) = note("ask Ada teh end", Some((4, 7)));
    caret_at_end(&el);
    // The text after the chip, " teh end": a spelling correction names
    // "teh", the caret elsewhere.
    let tail = el.last_child().unwrap();
    assert_eq!(tail.text_content().unwrap(), " teh end");
    before_input_at(&el, "insertReplacementText", Some("the"), &tail, 1, 4);
    assert_eq!(value.get(), "ask Ada the end", "the word, not at the caret");
    assert_eq!(chips(&el), ["Ada"]);
    // A delete is the model's at the caret, whatever range the browser
    // names: Backspace after the chip takes it whole.
    let tail = el.last_child().unwrap();
    let at = document().create_range().unwrap();
    at.set_start(&tail, 0).unwrap();
    at.set_end(&tail, 0).unwrap();
    let selection = document().get_selection().unwrap().unwrap();
    selection.remove_all_ranges().unwrap();
    selection.add_range(&at).unwrap();
    before_input_at(&el, "deleteContentBackward", None, &tail, 4, 8);
    assert_eq!(value.get(), "ask  the end");
    assert!(chips(&el).is_empty(), "the chip, whole");
}
