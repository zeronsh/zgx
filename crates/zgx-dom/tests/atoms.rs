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

/// The browser's selection from `from` in `a` to `to` in `b` (text nodes).
fn select_nodes(a: &web_sys::Node, from: u32, b: &web_sys::Node, to: u32) {
    let selection = document().get_selection().unwrap().unwrap();
    selection.set_base_and_extent(a, from, b, to).unwrap();
}

#[wasm_bindgen_test]
fn a_selection_into_a_chip_takes_it_whole() {
    let (_mounted, el, value) = note("ask Ada now", Some((4, 7)));
    el.focus().unwrap();
    let head = el.first_child().unwrap();
    let label = el
        .query_selector("[data-atom]")
        .unwrap()
        .unwrap()
        .first_child()
        .unwrap();
    select_nodes(&head, 2, &label, 1);
    before_input(&el, "deleteContentBackward", None);
    assert_eq!(value.get(), "as now", "the chip went whole");
    assert!(chips(&el).is_empty());
}

fn compose(el: &web_sys::HtmlElement, kind: &str, data: &str) {
    let init = web_sys::CompositionEventInit::new();
    init.set_bubbles(true);
    init.set_data(data);
    let event = web_sys::CompositionEvent::new_with_event_init_dict(kind, &init).unwrap();
    el.dispatch_event(&event).unwrap();
    zgx_dom::flush();
}

#[wasm_bindgen_test]
fn an_input_method_right_beside_a_chip_leaves_it_whole() {
    let (_mounted, el, value) = note("ask Ada", Some((4, 7)));
    caret_at_end(&el);
    compose(&el, "compositionstart", "");
    el.append_child(&document().create_text_node("にほん"))
        .unwrap();
    compose(&el, "compositionend", "日本");
    assert_eq!(value.get(), "ask Ada日本");
    assert_eq!(chips(&el), ["Ada"]);
    assert_eq!(caret(&el), 9, "the caret after what was committed");
    // Right before it: the end of the text before the chip.
    let head = el.first_child().unwrap();
    select_nodes(&head, 4, &head, 4);
    compose(&el, "compositionstart", "");
    compose(&el, "compositionend", "火");
    assert_eq!(value.get(), "ask 火Ada日本");
    assert_eq!(chips(&el), ["Ada"]);
    assert_eq!(caret(&el), 5);
}

/// Wait for the browser's tasks (a `selectionchange` is one).
async fn tick() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// The browser's selection in `el`'s text, UTF-16 units, anchor and focus.
fn selected(el: &web_sys::HtmlElement) -> (u32, u32) {
    let selection = document().get_selection().unwrap().unwrap();
    let at = |node: web_sys::Node, offset: u32| {
        let range = document().create_range().unwrap();
        range.set_start(el, 0).unwrap();
        range.set_end(&node, offset).unwrap();
        String::from(range.to_string()).encode_utf16().count() as u32
    };
    (
        at(selection.anchor_node().unwrap(), selection.anchor_offset()),
        at(selection.focus_node().unwrap(), selection.focus_offset()),
    )
}

#[wasm_bindgen_test]
async fn a_selection_shown_into_a_chip_takes_it_whole_and_a_caret_its_nearer_edge() {
    let (_mounted, el, _) = note("ask Ada now", Some((4, 7)));
    el.focus().unwrap();
    let head = el.first_child().unwrap();
    let label = el
        .query_selector("[data-atom]")
        .unwrap()
        .unwrap()
        .first_child()
        .unwrap();
    select_nodes(&head, 2, &label, 1);
    tick().await;
    assert_eq!(selected(&el), (2, 7), "the chip whole");
    select_nodes(&label, 2, &head, 1);
    tick().await;
    assert_eq!(selected(&el), (7, 1), "backwards too");
    select_nodes(&label, 1, &label, 1);
    tick().await;
    assert_eq!(selected(&el), (4, 4), "a caret at the nearer edge");
    let chip: web_sys::HtmlElement = el
        .query_selector("[data-atom]")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    assert_eq!(
        chip.style().get_property_value("user-select").unwrap(),
        "all",
        "a drag or a double click selects it whole"
    );
}

/// The clipboard type a text area's atoms go on as.
const ATOMS_TYPE: &str = "application/x-zgx-atoms";

/// A `copy` (or `cut`) of `el`'s whole text, as the browser sends a
/// person's ⌘C: its data, as the page wrote it.
fn copy_all(el: &web_sys::HtmlElement, kind: &str) -> web_sys::DataTransfer {
    el.focus().unwrap();
    let range = document().create_range().unwrap();
    range.select_node_contents(el).unwrap();
    let selection = document().get_selection().unwrap().unwrap();
    selection.remove_all_ranges().unwrap();
    selection.add_range(&range).unwrap();
    let data = web_sys::DataTransfer::new().unwrap();
    let init = web_sys::ClipboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_clipboard_data(Some(&data));
    let event = web_sys::ClipboardEvent::new_with_event_init_dict(kind, &init).unwrap();
    el.dispatch_event(&event).unwrap();
    zgx_dom::flush();
    data
}

/// A paste of `data` at the end of `el`.
fn paste(el: &web_sys::HtmlElement, data: &web_sys::DataTransfer) {
    caret_at_end(el);
    let init = web_sys::InputEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_input_type("insertFromPaste");
    init.set_data_transfer(Some(data));
    let event = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
    el.dispatch_event(&event).unwrap();
    zgx_dom::flush();
}

#[wasm_bindgen_test]
fn a_copy_puts_its_atoms_beside_the_text() {
    let (_mounted, el, _) = note("ask Ada now", Some((4, 7)));
    let data = copy_all(&el, "copy");
    assert_eq!(data.get_data("text/plain").unwrap(), "ask Ada now");
    let json = data.get_data(ATOMS_TYPE).unwrap();
    assert!(json.contains(r#""from":4,"to":7"#), "{json}");
}

#[wasm_bindgen_test]
fn a_paste_of_a_copy_makes_its_atoms_chips_and_a_cut_takes_them() {
    let (_mounted, from, from_value) = note("ask Ada now", Some((4, 7)));
    let (_other, to, to_value) = note("hi ", Some((0, 2)));
    let data = copy_all(&from, "cut");
    assert_eq!(from_value.get(), "", "the cut took it, chip and all");
    assert!(chips(&from).is_empty());
    paste(&to, &data);
    assert_eq!(to_value.get(), "hi ask Ada now");
    assert_eq!(chips(&to), ["hi", "Ada"], "the pasted atom a chip");
    before_input(&to, "historyUndo", None);
    assert_eq!(to_value.get(), "hi ");
    assert_eq!(chips(&to), ["hi"], "one undo takes the paste and its chip");
}

#[wasm_bindgen_test]
fn a_chip_is_read_as_one_unit_by_its_text_or_its_name() {
    let runtime = Runtime::new();
    let value = runtime.signal(String::from("ask Ada and Bo"));
    let host = host();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        text_area("Message", value).atoms_when(|editor| {
            let mut named = Atom::new(editor.new_atom_id(), 12..14);
            named.name = Some(std::sync::Arc::from("Bo Jensen"));
            vec![Atom::new(editor.new_atom_id(), 4..7), named]
        }),
    );
    zgx_dom::flush();
    let list = host.query_selector_all("[data-atom]").unwrap();
    let chip = |i| {
        list.item(i)
            .unwrap()
            .dyn_into::<web_sys::Element>()
            .unwrap()
    };
    for i in 0..2 {
        assert_eq!(chip(i).get_attribute("role").as_deref(), Some("img"));
    }
    assert_eq!(chip(0).get_attribute("aria-label").as_deref(), Some("Ada"));
    assert_eq!(
        chip(1).get_attribute("aria-label").as_deref(),
        Some("Bo Jensen")
    );
}

/// A small SVG, as an atom's icon.
const DOT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="8"/></svg>"#;

#[wasm_bindgen_test]
fn its_chips_take_their_padding_corners_and_icons() {
    use zgx::widgets::{AtomIcon, AtomStyle};
    let icon = |tint: Option<Color>, well: Option<Color>| {
        let svg = zgx::svg::SvgData::new(DOT.as_bytes().to_vec()).unwrap();
        AtomIcon {
            svg: std::sync::Arc::new(match tint {
                Some(tint) => svg.tinted(tint),
                None => svg,
            }),
            size: 12.,
            gap: 3.,
            bg: well,
            radius: 6.,
            pad: 2.,
        }
    };
    let runtime = Runtime::new();
    let value = runtime.signal(String::from("ask Ada and Bo"));
    let host = host();
    let plain = icon(None, None);
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        text_area("Message", value)
            .atoms_when(move |editor| {
                let mut own = Atom::new(editor.new_atom_id(), 12..14);
                own.style = Some(std::sync::Arc::new(AtomStyle {
                    icon: Some(plain.clone()),
                    radius: Some(2.),
                    ..Default::default()
                }));
                vec![Atom::new(editor.new_atom_id(), 4..7), own]
            })
            .atom_wash(Color(200, 220, 255, 255))
            .atom_style(AtomStyle {
                padding: Some([6., 4., 1.]),
                radius: Some(9.),
                icon: Some(icon(
                    Some(Color(255, 0, 0, 255)),
                    Some(Color(0, 0, 255, 255)),
                )),
                ..Default::default()
            }),
    );
    zgx_dom::flush();
    let list = host.query_selector_all("[data-atom]").unwrap();
    let chip = |i| {
        list.item(i)
            .unwrap()
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap()
    };
    let style = |el: &web_sys::Element, name: &str| {
        web_sys::window()
            .unwrap()
            .get_computed_style(el)
            .unwrap()
            .unwrap()
            .get_property_value(name)
            .unwrap()
    };
    // The text area's look: its padding and corners.
    let first = chip(0);
    assert_eq!(
        first.text_content().unwrap(),
        "Ada",
        "the label alone is its text"
    );
    assert_eq!(style(&first, "padding-left"), "6px");
    assert_eq!(style(&first, "padding-right"), "4px");
    assert_eq!(style(&first, "padding-top"), "1px");
    assert_eq!(style(&first, "border-top-left-radius"), "9px");
    // Its icon: tinted, so a mask over the tint, in a blue well.
    let well = first.first_element_child().expect("a well");
    assert_eq!(well.get_attribute("aria-hidden").as_deref(), Some("true"));
    assert_eq!(style(&well, "background-color"), "rgb(0, 0, 255)");
    assert_eq!(style(&well, "padding-left"), "2px");
    assert_eq!(style(&well, "margin-right"), "3px");
    let picture = well.first_element_child().expect("the icon");
    assert_eq!(style(&picture, "width"), "12px");
    assert_eq!(style(&picture, "background-color"), "rgb(255, 0, 0)");
    assert!(style(&picture, "mask-image").contains("data:image/svg+xml;base64,"));
    // An atom's own look over the text area's: its icon untinted, no well,
    // its own corners, the text area's padding.
    let second = chip(1);
    assert_eq!(second.text_content().unwrap(), "Bo");
    assert_eq!(style(&second, "border-top-left-radius"), "2px");
    assert_eq!(style(&second, "padding-left"), "6px");
    let picture = second.first_element_child().expect("its icon");
    assert_eq!(style(&picture, "margin-right"), "3px");
    assert!(style(&picture, "background-image").contains("data:image/svg+xml;base64,"));
}
