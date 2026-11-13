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
    // In view whatever earlier tests left on the page: it observes its
    // visibility.
    host.scroll_into_view();
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
    // Tall: gone, so the page's later tests start in view.
    _mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn alike_rows_keep_their_own_values_and_classes() {
    // Rows of one structure, some with another class set or another
    // attribute value: each keeps its own, stamped or not.
    let runtime = Runtime::new();
    let host = host();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        keyed(
            || (0..20).collect::<Vec<u32>>(),
            |k: u32, _| {
                let label = text(format!("row {k}")).accessible_label(format!("label {k}"));
                let label = if k.is_multiple_of(3) {
                    label.font_bold()
                } else {
                    label
                };
                row().p(2.).child(label)
            },
        ),
    );
    let list = first(&host);
    let window = web_sys::window().unwrap();
    let mut classes = std::collections::HashSet::new();
    for k in 0..20 {
        let cell = list
            .children()
            .item(k)
            .unwrap()
            .first_element_child()
            .unwrap();
        assert_eq!(cell.text_content().unwrap(), format!("row {k}"));
        assert_eq!(
            cell.get_attribute("aria-label").unwrap(),
            format!("label {k}")
        );
        let weight = window
            .get_computed_style(&cell)
            .unwrap()
            .unwrap()
            .get_property_value("font-weight")
            .unwrap();
        assert_eq!(
            weight == "700",
            k.is_multiple_of(3),
            "row {k} weighs {weight}"
        );
        classes.insert(cell.class_name());
    }
    assert_eq!(classes.len(), 2, "bold and plain cells, a class each");
}

/// A row of a shape picked by `seed` (nested rows and columns, texts,
/// buttons, a few styles and labels) holding row `nth`'s own texts and labels:
/// rows of one seed share a template and differ in their values.
fn random_row(seed: u32, nth: u32) -> zgx::compose::View {
    let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    let mut next = move |n: u32| {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x % n
    };
    fn styled(view: zgx::compose::View, pick: u32) -> zgx::compose::View {
        match pick {
            0 => view,
            1 => view.p(4.),
            2 => view.bg(Color(200, 40, 40, 255)),
            3 => view.w(40.),
            _ => view.font_bold().text_color(Color(10, 120, 200, 255)),
        }
    }
    let leaf = |next: &mut dyn FnMut(u32) -> u32| -> zgx::compose::View {
        let view = match next(4) {
            0 => text(format!("t{}-{nth}", next(5))),
            1 => button()
                .child(text(format!("b{}", next(3))))
                .on_click(|| {}),
            2 => text("label").accessible_label(format!("l{}-{}", next(4), nth % 3)),
            _ => column()
                .child(text("inner"))
                .child(text(format!("i{}-{}", next(3), nth % 2))),
        };
        let pick = next(5);
        styled(view, pick)
    };
    let mut parent = if next(2) == 0 { row() } else { column() };
    for _ in 0..1 + next(3) {
        parent = parent.child(leaf(&mut next));
    }
    let pick = next(5);
    styled(parent, pick)
}

#[wasm_bindgen_test]
fn stamped_rows_match_rows_written_in_full() {
    // Many rows of a few random shapes, mounted stamped from templates and
    // written in full: the same DOM, node for node.
    let keys = || (0..120).collect::<Vec<u32>>();
    let shapes = |k: u32| random_row(k % 17, k);
    let runtime = Runtime::new();
    let (stamped, full) = (host(), host());
    let a = zgx_dom::mount(&runtime, &stamped, keyed(keys, move |k: u32, _| shapes(k)));
    zgx_dom::__set_templates(false);
    let b = zgx_dom::mount(&runtime, &full, keyed(keys, move |k: u32, _| shapes(k)));
    zgx_dom::__set_templates(true);
    assert_eq!(stamped.inner_html(), full.inner_html());
    assert!(stamped.inner_html().len() > 2_000);
    // Tall: gone, so the page's later tests start in view.
    a.unmount();
    b.unmount();
    stamped.remove();
    full.remove();
}

#[wasm_bindgen_test]
fn a_rows_child_swapped_later_takes_its_handler_and_style() {
    // Nodes nothing wrote when a row was stamped get no id; what a row
    // later shows (a switch's new view) is a block of its own, with its
    // own: its handler runs and its style shows.
    let runtime = Runtime::new();
    let armed = runtime.signal(false);
    let clicks = runtime.signal(0);
    let host = host();
    let (read, count) = (armed.clone(), clicks.clone());
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        keyed(
            || (0..6).collect::<Vec<u32>>(),
            move |k: u32, _| {
                let (read, count) = (read.clone(), count.clone());
                row().p(1.).child(text(format!("r{k}"))).child(switch(
                    move || read.get(),
                    move |armed, _| {
                        let count = count.clone();
                        if armed {
                            button()
                                .bg(Color(0, 128, 0, 255))
                                .child(text("go"))
                                .on_click(move || {
                                    count.set(count.get() + 1);
                                })
                        } else {
                            text("idle")
                        }
                    },
                ))
            },
        ),
    );
    armed.set(true);
    zgx_dom::flush();
    let list = first(&host);
    let button: web_sys::HtmlElement = list
        .children()
        .item(4)
        .unwrap()
        .query_selector("button")
        .unwrap()
        .expect("the row's new button")
        .dyn_into()
        .unwrap();
    button.click();
    assert_eq!(clicks.get(), 1);
    let bg = web_sys::window()
        .unwrap()
        .get_computed_style(&button)
        .unwrap()
        .unwrap()
        .get_property_value("background-color")
        .unwrap();
    assert_eq!(bg, "rgb(0, 128, 0)");
}

#[wasm_bindgen_test]
fn stamped_rows_hold_ids_only_for_what_is_written() {
    // A row of seven nodes: its root (a reactive style), a static cell and
    // its text (the row's own value), a label with a handler and its bound
    // text, and a button with a handler holding a static text. Stamped from
    // a template, the static cell and the button's text take no id: five.
    let runtime = Runtime::new();
    let selected = runtime.signal(false);
    let host = host();
    let rows = 40_u32;
    let before = zgx_dom::live_nodes();
    let (sel, label_sel) = (selected.clone(), selected.clone());
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        keyed(
            move || (0..rows).collect::<Vec<u32>>(),
            move |k: u32, _| {
                let (sel, label_sel) = (sel.clone(), label_sel.clone());
                row()
                    .reactive_style(move || {
                        if sel.get() {
                            Styles::new().bg(Color(120, 40, 40, 255))
                        } else {
                            Styles::new()
                        }
                    })
                    .child(text(k.to_string()).w(40.))
                    .child(
                        text_signal(move || format!("{k} {}", label_sel.get()))
                            .grow()
                            .on_click(|| {}),
                    )
                    .child(text("x").w(20.).on_click(|| {}))
            },
        ),
    );
    // The host, the typography root and the list; the first row written in
    // full (no template yet: seven); the template itself (seven, kept for
    // the page); the second row, the template's own values (four: its cell
    // text needs no write); then five a row.
    let ids = zgx_dom::live_nodes() - before;
    assert_eq!(ids, 3 + 7 + 7 + 4 + (rows - 2) * 5, "{ids} ids");
    // And what they hold still updates: every row's label, and the style.
    selected.set(true);
    zgx_dom::flush();
    let list = first(&host);
    let last = list.last_element_child().unwrap();
    assert_eq!(
        last.children().item(1).unwrap().text_content().unwrap(),
        format!("{} true", rows - 1)
    );
    assert!(
        last.get_attribute("style")
            .unwrap()
            .contains("background-color")
    );
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn virtual_list_rows_lie_inside_the_list() {
    // Its rows are positioned in its scrolled content, at their offsets,
    // and the list is as tall as the view says.
    let runtime = Runtime::new();
    let offset = runtime.signal(0_f32);
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().p(40.).child(
            virtual_list(
                offset.clone(),
                20.,
                0,
                || 1_000,
                |i| i,
                |_, key: usize, _| text(format!("row {key}")),
            )
            .h(100.),
        ),
    );
    let list = first(&host).first_element_child().unwrap();
    let rect = list.get_bounding_client_rect();
    assert_eq!(rect.height(), 100.);
    let space = list.first_element_child().unwrap();
    let rows = space.children();
    assert!(rows.length() > 0);
    for i in 0..rows.length() {
        let row = rows.item(i).unwrap().get_bounding_client_rect();
        assert!(
            row.top() >= rect.top() - 0.5 && row.left() >= rect.left() - 0.5,
            "row {i} at ({}, {}) lies outside the list at ({}, {})",
            row.left(),
            row.top(),
            rect.left(),
            rect.top()
        );
    }
    let first_row = rows.item(0).unwrap().get_bounding_client_rect();
    assert_eq!(first_row.top(), rect.top());
    // Gone, so it pushes no later test's host out of view.
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn rows_observing_their_visibility_mount_whole() {
    let runtime = Runtime::new();
    let rows = runtime.signal((0..4).collect::<Vec<u32>>());
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(keyed(
            move || rows.get(),
            |n: u32, _| {
                row()
                    .child(text(format!("Row {n}")))
                    .child(text("seen"))
                    .on_mount(|node| {
                        let _ = node.observe_visibility();
                    })
            },
        )),
    );
    // A row's block replayed (or stamped) whole, its observer's op among
    // the others.
    let list = first(&host).first_element_child().unwrap();
    assert_eq!(
        texts(&list),
        ["Row 0seen", "Row 1seen", "Row 2seen", "Row 3seen"]
    );
    mounted.unmount();
    host.remove();
}
