//! zgx views mounted as DOM, in a headless browser
//! (`wasm-pack test --headless --chrome`, or `cargo test --target
//! wasm32-unknown-unknown` with wasm-bindgen-test-runner).
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use zgx::{
    compose::{
        button, checkbox, column, keyed, row, scroll, switch, text, text_input, text_signal,
        virtual_list,
    },
    reactive::Runtime,
    scene::Color,
    style::{Styled, Styles},
};

wasm_bindgen_test_configure!(run_in_browser);

fn host() -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
}

/// The mounted view's own element (inside the root that carries the
/// theme's typography).
fn first(host: &web_sys::Element) -> web_sys::Element {
    host.first_element_child()
        .unwrap()
        .first_element_child()
        .unwrap()
}

fn texts(parent: &web_sys::Element) -> Vec<String> {
    let children = parent.children();
    (0..children.length())
        .map(|i| children.item(i).unwrap().text_content().unwrap_or_default())
        .collect()
}

#[wasm_bindgen_test]
fn bound_text_updates_its_node_alone() {
    let runtime = Runtime::new();
    let count = runtime.signal(0);
    let shown = count.clone();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column()
            .child(text("Count"))
            .child(text_signal(move || format!("{} clicks", shown.get()))),
    );
    let view = first(&host);
    assert_eq!(view.text_content().unwrap(), "Count0 clicks");
    let nodes = view.child_nodes();
    let bound = nodes.item(1).unwrap();
    count.set(3);
    zgx_dom::flush();
    assert_eq!(view.text_content().unwrap(), "Count3 clicks");
    // The same text node, its value written.
    assert!(
        view.child_nodes()
            .item(1)
            .unwrap()
            .is_same_node(Some(&bound))
    );
    let live = zgx_dom::live_nodes();
    mounted.unmount();
    assert_eq!(host.child_element_count(), 0);
    // The typography root, the column, its two text nodes and the host
    // element, let go.
    assert_eq!(live - zgx_dom::live_nodes(), 5);
}

#[wasm_bindgen_test]
fn styles_become_shared_classes_and_css() {
    let runtime = Runtime::new();
    let host = host();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        row()
            .gap(8.)
            .p(4.)
            .child(column().w(40.).bg(Color(255, 0, 0, 255)).child(text("a")))
            .child(column().w(40.).bg(Color(255, 0, 0, 255)).child(text("b"))),
    );
    let view = first(&host);
    let a = view.children().item(0).unwrap();
    let b = view.children().item(1).unwrap();
    assert_eq!(a.class_name(), b.class_name(), "alike styles share a class");
    let style = web_sys::window()
        .unwrap()
        .get_computed_style(&a)
        .unwrap()
        .unwrap();
    assert_eq!(style.get_property_value("width").unwrap(), "40px");
    assert_eq!(
        style.get_property_value("background-color").unwrap(),
        "rgb(255, 0, 0)"
    );
    let row_style = web_sys::window()
        .unwrap()
        .get_computed_style(&view)
        .unwrap()
        .unwrap();
    assert_eq!(row_style.get_property_value("display").unwrap(), "flex");
    assert_eq!(
        row_style.get_property_value("flex-direction").unwrap(),
        "row"
    );
    assert_eq!(row_style.get_property_value("column-gap").unwrap(), "8px");
    assert_eq!(row_style.get_property_value("padding-top").unwrap(), "4px");
}

#[wasm_bindgen_test]
fn clicks_reach_handlers_through_the_root() {
    let runtime = Runtime::new();
    let count = runtime.signal(0);
    let (write, shown) = (count.clone(), count.clone());
    let host = host();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        column()
            .child(button().child(text("Add")).on_click(move || {
                write.set(write.get() + 1);
            }))
            .child(text_signal(move || shown.get().to_string())),
    );
    let view = first(&host);
    let button: web_sys::HtmlElement = view.children().item(0).unwrap().dyn_into().unwrap();
    assert_eq!(button.tag_name(), "BUTTON");
    button.click();
    button.click();
    assert_eq!(count.get(), 2);
    assert_eq!(view.text_content().unwrap(), "Add2");
}

#[wasm_bindgen_test]
fn keyed_rows_reorder_and_clear() {
    let runtime = Runtime::new();
    let items = runtime.signal(vec![1, 2, 3, 4, 5]);
    let read = items.clone();
    let host = host();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        keyed(move || read.get(), |id: i32, _| text(id.to_string()).p(1.)),
    );
    let list = first(&host);
    assert_eq!(texts(&list), ["1", "2", "3", "4", "5"]);
    let second = list.children().item(1).unwrap();
    // A swap moves two rows; the others keep their nodes.
    items.set(vec![1, 4, 3, 2, 5]);
    zgx_dom::flush();
    assert_eq!(texts(&list), ["1", "4", "3", "2", "5"]);
    assert!(list.children().item(3).unwrap().is_same_node(Some(&second)));
    items.set(vec![5, 1, 3]);
    zgx_dom::flush();
    assert_eq!(texts(&list), ["5", "1", "3"]);
    items.set(vec![5, 1, 3, 6, 7]);
    zgx_dom::flush();
    assert_eq!(texts(&list), ["5", "1", "3", "6", "7"]);
    items.set(vec![0, 5, 1, 3, 6, 7]);
    zgx_dom::flush();
    assert_eq!(texts(&list), ["0", "5", "1", "3", "6", "7"]);
    let before = zgx_dom::live_nodes();
    items.set(vec![]);
    zgx_dom::flush();
    assert_eq!(list.child_element_count(), 0);
    // Each row was a span and its text: all freed.
    assert_eq!(before - zgx_dom::live_nodes(), 12);
}

#[wasm_bindgen_test]
fn switch_replaces_its_view() {
    let runtime = Runtime::new();
    let open = runtime.signal(false);
    let read = open.clone();
    let host = host();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(switch(
            move || read.get(),
            |open, _| {
                if open {
                    text("open").p(2.)
                } else {
                    text("closed").p(2.)
                }
            },
        )),
    );
    let view = first(&host);
    assert_eq!(view.text_content().unwrap(), "closed");
    open.set(true);
    zgx_dom::flush();
    assert_eq!(view.text_content().unwrap(), "open");
}

#[wasm_bindgen_test]
fn editors_bind_both_ways() {
    let runtime = Runtime::new();
    let value = runtime.signal(String::from("hi"));
    let host = host();
    let _mounted = zgx_dom::mount(&runtime, &host, text_input("Name", value.clone()));
    let input: web_sys::HtmlInputElement = first(&host).dyn_into().unwrap();
    assert_eq!(input.value(), "hi");
    assert_eq!(input.get_attribute("aria-label").unwrap(), "Name");
    input.set_value("hello");
    let event = web_sys::Event::new_with_event_init_dict("input", &{
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        init
    })
    .unwrap();
    input.dispatch_event(&event).unwrap();
    assert_eq!(value.get(), "hello");
    value.set("bye".into());
    zgx_dom::flush();
    assert_eq!(input.value(), "bye");
}

#[wasm_bindgen_test]
fn checkboxes_bind_both_ways() {
    let runtime = Runtime::new();
    let on = runtime.signal(false);
    let host = host();
    let _mounted = zgx_dom::mount(&runtime, &host, checkbox("Wrap", on.clone()));
    let label = first(&host);
    let input: web_sys::HtmlInputElement = label.first_element_child().unwrap().dyn_into().unwrap();
    assert!(!input.checked());
    input.click();
    assert!(on.get());
    on.set(false);
    zgx_dom::flush();
    assert!(!input.checked());
    assert_eq!(label.text_content().unwrap(), "Wrap");
}

#[wasm_bindgen_test]
fn scroll_offsets_write_the_viewport() {
    let runtime = Runtime::new();
    let offset = runtime.signal(0_f32);
    let host = host();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        scroll(offset.clone())
            .h(100.)
            .children((0..50).map(|i| text(format!("line {i}")).h(20.))),
    );
    let viewport = first(&host);
    offset.set(200.);
    zgx_dom::flush();
    assert_eq!(viewport.scroll_top(), 200);
}

#[wasm_bindgen_test]
fn virtual_lists_mount_what_shows() {
    let runtime = Runtime::new();
    let offset = runtime.signal(0_f32);
    let host = host();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        virtual_list(
            offset.clone(),
            20.,
            2,
            || 10_000,
            |i| i,
            |_, key: usize, _| text(format!("row {key}")),
        )
        .h(100.),
    );
    let list = first(&host);
    let space = list.first_element_child().unwrap();
    assert_eq!(
        space.get_attribute("style").unwrap(),
        "height: 200000px; flex-shrink: 0;"
    );
    // Before the viewport is measured, a row's worth plus overscan.
    assert!(space.child_element_count() <= 8);
    offset.set(10_000.);
    zgx_dom::flush();
    assert!(space.text_content().unwrap().contains("row 500"));
}

#[wasm_bindgen_test]
fn roles_and_labels_reach_aria() {
    use zgx::semantics::Role;
    let runtime = Runtime::new();
    let host = host();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        column()
            .role(Role::Toolbar)
            .accessible_label("Tools")
            .child(text("x")),
    );
    let view = first(&host);
    assert_eq!(view.get_attribute("role").unwrap(), "toolbar");
    assert_eq!(view.get_attribute("aria-label").unwrap(), "Tools");
}

#[wasm_bindgen_test]
fn clocks_and_tracing_read_the_browser_clock() {
    // std's clock panics in a browser; zgx's reads `performance.now()`.
    let start = zgx::clock::now();
    let wall = zgx::clock::system_now();
    assert!(wall > zgx::clock::system_now() - std::time::Duration::from_secs(1));
    zgx::trace::set_enabled(true);
    let runtime = Runtime::new();
    let count = runtime.signal(0);
    let read = count.clone();
    let host = host();
    let _mounted = zgx_dom::mount(&runtime, &host, text_signal(move || read.get().to_string()));
    count.set(1);
    zgx_dom::flush();
    zgx::trace::end_frame(Vec::new());
    let frames = zgx::trace::frames(1);
    zgx::trace::set_enabled(false);
    assert!(
        frames.last().is_some_and(|frame| !frame.runs.is_empty()),
        "the effect's run was traced, timed by the browser's clock"
    );
    assert!(zgx::clock::since(start) < std::time::Duration::from_secs(60));
    assert_eq!(first_text(&host), "1");
}

fn first_text(host: &web_sys::Element) -> String {
    host.first_element_child()
        .unwrap()
        .text_content()
        .unwrap_or_default()
}

/// Wait `ms` on the browser's clock (observers report between tasks).
async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

#[wasm_bindgen_test]
async fn observers_report_sizes_and_visibility() {
    use std::{cell::RefCell, rc::Rc};
    use zgx::reactive::Signal;
    type Seen = Option<(Signal<(f32, f32)>, Signal<zgx::scene::Rect>, Signal<bool>)>;
    let runtime = Runtime::new();
    let seen: Rc<RefCell<Seen>> = Rc::default();
    let into = seen.clone();
    let width = runtime.signal(120_f32);
    let read = width.clone();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column()
            .h(40.)
            .reactive_style(move || Styles::new().w(read.get()))
            .on_mount(move |node| {
                *into.borrow_mut() = Some((
                    node.observe_size().unwrap(),
                    node.observe_bounds().unwrap(),
                    node.observe_visibility().unwrap(),
                ));
            }),
    );
    sleep(50).await;
    let (size, bounds, visible) = seen.borrow().clone().unwrap();
    assert_eq!(size.get(), (120., 40.));
    assert_eq!(bounds.get().width, 120.);
    assert!(visible.get());
    width.set(200.);
    zgx_dom::flush();
    sleep(50).await;
    assert_eq!(size.get(), (200., 40.));
    mounted.unmount();
}

#[wasm_bindgen_test]
fn absolute_children_position_against_their_parent() {
    let runtime = Runtime::new();
    let host = host();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().p(30.).child(
            column()
                .w(100.)
                .h(50.)
                .child(text("x").absolute().top(5.).left(7.)),
        ),
    );
    let holder = first(&host).first_element_child().unwrap();
    let badge = holder.first_element_child().unwrap();
    let (a, b) = (
        holder.get_bounding_client_rect(),
        badge.get_bounding_client_rect(),
    );
    assert_eq!((b.left() - a.left(), b.top() - a.top()), (7., 5.));
}

#[wasm_bindgen_test]
fn a_long_frame_keeps_each_rows_strings() {
    // Past a few thousand words a frame applies what it has as it goes:
    // rows stamped from templates, and one of a new shape replayed with its
    // attribute, all keep their strings.
    let runtime = Runtime::new();
    let host = host();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        keyed(
            || (0..1500).collect::<Vec<u32>>(),
            |key: u32, _| {
                if key < 1499 {
                    text(format!("row {key}")).p(1.)
                } else {
                    button()
                        .accessible_label("the last row")
                        .child(text("last"))
                }
            },
        ),
    );
    let list = first(&host);
    assert_eq!(list.child_element_count(), 1500);
    let last = list.last_element_child().unwrap();
    assert_eq!(last.get_attribute("aria-label").unwrap(), "the last row");
    assert_eq!(last.text_content().unwrap(), "last");
    assert_eq!(
        list.children().item(700).unwrap().text_content().unwrap(),
        "row 700"
    );
}
