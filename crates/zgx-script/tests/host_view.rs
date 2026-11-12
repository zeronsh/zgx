//! `ui.host_view { name }`: a view the host makes in Rust, mounted in the script's own tree, so it lays
//! out like its elements and sits under its portals.
mod common;
use common::Harness;
use std::{cell::Cell, rc::Rc};
use zgx::compose::prelude::*;

const APP: &str = r##"
local layer = ui.state("layer", false)
ui.command { id = "layer", run = function() layer:set(true) end }
return ui.column { w_full = true, h_full = true,
    ui.text { id = "before", "before" },
    ui.host_view { name = "probe", id = "slot", w = 120, h = 40 },
    ui.host_view { name = "missing", id = "gone", w = 50, h = 20 },
    ui.layer { open = layer, z = 2147483647,
        ui.div { id = "layer-box", w = 400, h = 300, on_click = function() end } },
}
"##;

fn app(clicks: Rc<Cell<u32>>) -> Harness {
    zgx_script::host_view("probe", move || {
        let clicks = clicks.clone();
        div()
            .id("probe-view")
            .on_click(move || clicks.set(clicks.get() + 1))
    });
    let h = Harness::inline(APP);
    h.frames(3);
    h
}

#[test]
fn the_registered_view_is_laid_out_in_the_scripts_tree() {
    let mut h = app(Rc::default());
    assert_eq!(h.error(), None);
    let slot = h.bounds("slot");
    assert_eq!((slot.width, slot.height), (120., 40.));
    assert!(
        h.find("probe-view").is_none(),
        "the element's id names the view's node"
    );
    assert!(slot.y >= h.bounds("before").y + h.bounds("before").height);
}

#[test]
fn the_view_takes_the_pointer_where_nothing_covers_it() {
    let clicks = Rc::new(Cell::new(0));
    let mut h = app(clicks.clone());
    let slot = h.bounds("slot");
    let hit = h.ui.scene.borrow().hit_test(slot.x + 10., slot.y + 10.);
    assert_eq!(hit, h.find("slot"));
}

#[test]
fn a_layer_the_script_opens_covers_the_view() {
    let mut h = app(Rc::default());
    let slot = h.bounds("slot");
    assert!(h.host.run_command("layer"));
    h.frames(3);
    let hit = h.ui.scene.borrow().hit_test(slot.x + 10., slot.y + 10.);
    assert_ne!(hit, h.find("slot"), "under the script's popups");
    assert!(hit.is_some());
}

#[test]
fn an_unregistered_name_is_an_empty_box_of_its_size() {
    let mut h = app(Rc::default());
    assert_eq!(h.error(), None);
    let gone = h.bounds("gone");
    assert_eq!((gone.width, gone.height), (50., 20.));
}
