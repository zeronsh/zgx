//! A host's top layer (`View::top_layer`, in a view composed at `render`):
//! above whatever a script opens, and live under it. A host view composed
//! beside the script's is above the script's own elements, whatever their
//! `z`, but under its portals, which is why the layer is needed.
mod common;
use std::{cell::Cell, rc::Rc};
use zgx::compose::{View, prelude::*};
use zgx_script::{ScriptHost, testing::TestApp};

const APP: &str = r##"
local modal = ui.state("modal", false)
local layer = ui.state("layer", false)
local fixed = ui.state("fixed", false)
local clicked = ui.state("clicked", 0)
ui.command { id = "modal", run = function() modal:set(true) end }
ui.command { id = "layer", run = function() layer:set(true) end }
ui.command { id = "fixed", run = function() fixed:set(true) end }
return ui.column { w_full = true, h_full = true,
    ui.button { id = "page", w = 60, h = 30, "Page", on_click = function() clicked:set(clicked:peek() + 1) end },
    ui.text { id = "clicked", "{clicked}" },
    ui.show(fixed, ui.div { id = "fixed-box", absolute = true, top = 0, left = 0, w = 400, h = 300,
        z = 2147483647, on_click = function() end }),
    ui.modal { open = modal, z = 2147483647,
        ui.div { id = "modal-box", w = 400, h = 300, z = 2147483647, on_click = function() end } },
    ui.layer { open = layer, z = 2147483647,
        ui.div { id = "layer-box", w = 400, h = 300, z = 2147483647, on_click = function() end } },
}
"##;

/// What the pointer finds at the middle of the host's button.
fn at_button(app: &TestApp) -> Option<zgx::scene::NodeId> {
    app.ui.scene.borrow().hit_test(200., 140.)
}

fn button_at_middle(clicks: Rc<Cell<u32>>) -> View {
    button()
        .id("host")
        .absolute()
        .left(150.)
        .top(120.)
        .w(100.)
        .h(40.)
        .on_click(move || clicks.set(clicks.get() + 1))
}

#[test]
fn a_host_view_beside_the_scripts_is_under_its_portals() {
    let mut app = TestApp::wrapped(
        |runtime| ScriptHost::inline(runtime, "app", APP),
        |view: View| {
            overlay()
                .w_full()
                .h_full()
                .child(view)
                .child(button_at_middle(Rc::default()))
        },
    );
    app.frames(3);
    let host = app.find("host");
    assert!(app.host.run_command("fixed"));
    app.frames(3);
    assert_eq!(at_button(&app), host, "over the script's elements, any z");
    assert!(app.host.run_command("layer"));
    app.frames(3);
    assert_ne!(at_button(&app), host, "under its layer");
}

#[test]
fn a_hosts_top_layer_fills_the_window_from_its_corner() {
    let mut app = TestApp::wrapped(
        |runtime| ScriptHost::inline(runtime, "app", APP),
        move |view: View| {
            component(move |cx| {
                let open = cx.state(true);
                overlay().w_full().h_full().child(view).child(
                    modal("Decisions", open)
                        .top_layer()
                        .child(div().id("filled").w_full().h_full()),
                )
            })
        },
    );
    app.frames(3);
    let filled = app.bounds("filled");
    let window = app.bounds("page");
    assert_eq!((filled.x, filled.y), (window.x, window.y));
    let root = app.ui.scene.borrow().bounds(app.ui.root());
    assert_eq!((filled.width, filled.height), (root.width, root.height));
    // Nor does what holds it reach past the window.
    let filled = app.find("filled").unwrap();
    let held = {
        let scene = app.ui.scene.borrow();
        let panel = scene.parent(filled).unwrap();
        let position = scene.parent(panel).unwrap();
        scene.bounds(position)
    };
    assert_eq!((held.width, held.height), (root.width, root.height));
}

#[test]
fn a_hosts_top_layer_is_above_what_a_script_opens_and_live_under_a_modal() {
    let clicks = Rc::new(Cell::new(0));
    let heard = clicks.clone();
    let mut app = TestApp::wrapped(
        |runtime| ScriptHost::inline(runtime, "app", APP),
        move |view: View| {
            component(move |cx| {
                let open = cx.state(true);
                overlay().w_full().h_full().child(view).child(
                    modal("Decisions", open)
                        .top_layer()
                        .child(button_at_middle(heard)),
                )
            })
        },
    );
    app.frames(3);
    let host = app.find("host");
    assert!(host.is_some());
    // Where it holds nothing, the pointer reaches what is under it.
    app.click("page");
    app.frames(2);
    assert_eq!(app.text("clicked"), "1", "the layer passes it through");
    // Each opened after it, the layer raising itself over everything as it
    // opens, and each asking for the highest `z` it can.
    for command in ["fixed", "layer", "modal"] {
        assert!(app.host.run_command(command));
        app.frames(5);
        assert_eq!(at_button(&app), host, "over {command}");
    }
    // Live under the modal's scope: a press reaches it.
    let before = app.ui.input.focus_scope();
    assert!(before.is_some(), "the modal holds focus");
    app.click("host");
    app.frames(2);
    assert_eq!(clicks.get(), 1);
    // And it closes nothing: the modal is still up.
    assert_eq!(app.ui.input.focus_scope(), before);
    assert!(app.find("modal-box").is_some());
}
