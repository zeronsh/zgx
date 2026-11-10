//! Native affine inset bindings: source changes run no script.
mod common;
use common::Harness;
use zgx_script::Value;

const RAIL: &str = r#"
local offset = ui.state("offset", 0)
local limit = ui.state("limit", 100)
local travel = ui.state("travel", 100)
local width = ui.state("width", 300)
local thumb_height = ui.state("thumb_height", 24)
return ui.column {
    id = "page", relative = true, h = 220,
    w = function() return width:get() end,
    ui.div { id = "rail", absolute = true, w = 8, h = 200, right = 0, top = 0,
        ui.div { id = "thumb", absolute = true, w = 4, h = function() return thumb_height:get() end,
            top = ui.map(offset, {
                from = {0, function() return limit:get() end},
                to = {4, function() return 4 + travel:get() end},
                clamp = true,
            }),
        },
    },
}
"#;
fn set(h: &Harness, name: &str, value: f64) {
    h.host.set_state(name, Value::Num(value)).unwrap();
}
#[test]
fn source_changes_clamp_natively_and_endpoint_changes_remap_an_unchanged_source() {
    let mut h = Harness::inline(RAIL);
    h.frames(2);
    let calls = h.calls();
    for (offset, expected) in [(0., 4.), (50., 54.), (100., 104.), (-10., 4.), (120., 104.)] {
        set(&h, "offset", offset);
        h.frame();
        assert_eq!(h.bounds("thumb").y, expected);
        assert_eq!(h.calls(), calls, "source changes must run no Luau");
    }
    set(&h, "limit", 200.);
    set(&h, "travel", 40.);
    h.frame();
    assert_eq!(h.bounds("thumb").y, 28.);
    let calls = h.calls();
    set(&h, "offset", 100.);
    h.frame();
    assert_eq!(h.bounds("thumb").y, 24.);
    assert_eq!(h.calls(), calls);
    set(&h, "limit", 0.);
    set(&h, "travel", 0.);
    h.frame();
    assert_eq!(h.bounds("thumb").y, 4.);
}
#[test]
fn mapped_insets_and_animated_ancestor_sizes_match_fresh_layout_in_the_same_frame() {
    let source = RAIL
        .replace("local offset =", "local flow_height = ui.state(\"flow_height\", 20)\nlocal offset =")
        .replace("    ui.div { id = \"rail\"", "    ui.div { id=\"flow\",h=function() return flow_height:get() end },\n    ui.div { id=\"after\",h=10 },\n    ui.div { id = \"rail\"");
    let mut h = Harness::inline(&source);
    for width in [320., 300., 310., 300.] {
        set(&h, "width", width);
        h.frame();
    }
    for (offset, width, height) in [(50., 400., 70.), (70., 260., 30.), (20., 350., 80.)] {
        // Queue the mapped inset first; neither sibling content nor the
        // ancestor's warmed pure-box path may disappear behind its stop.
        set(&h, "offset", offset);
        set(&h, "flow_height", height);
        set(&h, "width", width);
        h.frame();
        let mut fresh = Harness::inline(&source);
        set(&fresh, "width", width);
        set(&fresh, "flow_height", height);
        set(&fresh, "offset", offset);
        fresh.frame();
        for id in ["page", "flow", "after", "rail", "thumb"] {
            assert_eq!(h.bounds(id), fresh.bounds(id), "{id} at {offset}/{width}");
        }
    }
}

#[test]
fn another_style_change_keeps_the_current_mapped_inset_without_subscribing_its_lua() {
    let mut h = Harness::inline(RAIL);
    set(&h, "offset", 50.);
    h.frame();
    set(&h, "thumb_height", 30.);
    h.frame();
    assert_eq!((h.bounds("thumb").y, h.bounds("thumb").height), (54., 30.));
    let calls = h.calls();
    set(&h, "offset", 75.);
    h.frame();
    assert_eq!(h.bounds("thumb").y, 79.);
    assert_eq!(
        h.calls(),
        calls,
        "unrelated height function must not run on scroll"
    );
}
#[test]
fn geometry_endpoints_settle_in_the_resize_frame_without_a_source_change() {
    let mut h = Harness::inline(
        r#"
local offset = ui.state("offset", 50)
local height = ui.state("height", 100)
local track = ui.ref()
return ui.column { relative = true, w = 300, h = 250,
    ui.div { ref = track, absolute = true, w = 8,
        h = function() return height:get() end,
        ui.div { id = "thumb", absolute = true, w = 4, h = 24,
            top = ui.map(offset, {from={0,100},to={4,function() return (track.height or 0)-4 end},clamp=true}),
        },
    },
}
"#,
    );
    h.frame();
    assert_eq!(h.bounds("thumb").y, 50.);
    set(&h, "height", 200.);
    h.frame();
    assert_eq!(h.bounds("thumb").y, 100.);
    let calls = h.calls();
    set(&h, "offset", 75.);
    h.frame();
    assert_eq!(h.bounds("thumb").y, 148.);
    assert_eq!(h.calls(), calls);
}
#[test]
fn horizontal_mapping_and_computed_endpoints_use_the_same_native_path() {
    let mut h = Harness::inline(
        r#"
local offset = ui.state("offset", 50)
local base = ui.state("base", 100)
local limit = ui.computed(function() return base:get() end)
return ui.column { relative=true, w=300, h=220,
    ui.div { absolute=true, w=200, h=8,
        ui.div { id="thumb", absolute=true, w=24, h=4,
            left=ui.map(offset,{from={0,limit},to={4,104},clamp=true}),
        },
    },
}
"#,
    );
    h.frame();
    assert_eq!(h.bounds("thumb").x, 54.);
    let calls = h.calls();
    set(&h, "offset", 100.);
    h.frame();
    assert_eq!(h.bounds("thumb").x, 104.);
    assert_eq!(h.calls(), calls);
    set(&h, "base", 200.);
    h.frame();
    assert_eq!(h.bounds("thumb").x, 54.);
}
#[test]
fn late_absolute_and_relative_transitions_match_a_fresh_layout() {
    let source = RAIL
        .replace("local offset =", "local absolute = ui.state(\"absolute\", true)\nlocal relative = ui.state(\"relative\", true)\nlocal offset =")
        .replace("id = \"page\", relative = true", "id = \"page\", relative = function() return relative:get() end")
        .replace("id = \"thumb\", absolute = true", "id = \"thumb\", absolute = function() return absolute:get() end");
    let mut h = Harness::inline(&source);
    for (offset, absolute, relative) in [
        (50., false, true),
        (70., true, false),
        (20., true, true),
        (90., false, false),
    ] {
        set(&h, "offset", offset);
        h.host.set_state("absolute", Value::Bool(absolute)).unwrap();
        h.host.set_state("relative", Value::Bool(relative)).unwrap();
        h.frame();
        let mut fresh = Harness::inline(&source);
        fresh
            .host
            .set_state("absolute", Value::Bool(absolute))
            .unwrap();
        fresh
            .host
            .set_state("relative", Value::Bool(relative))
            .unwrap();
        set(&fresh, "offset", offset);
        fresh.frame();
        for id in ["page", "rail", "thumb"] {
            assert_eq!(
                h.bounds(id),
                fresh.bounds(id),
                "{id} at {offset}/{absolute}/{relative}"
            );
        }
    }
}

#[test]
fn mapped_thumb_lays_out_only_its_rail_for_short_and_long_pages() {
    let mut costs = Vec::new();
    for rows in [10, 400] {
        let source = RAIL
            .replace("return ui.column {", &format!("local rows = {{}} for i=1,{rows} do rows[i]=ui.div{{w=100,h=20,shrink=0}} end\nreturn ui.column {{"));
        let source = source.replace(
            "    ui.div { id = \"rail\"",
            "    ui.column { shrink=0, table.unpack(rows) },\n    ui.div { id = \"rail\"",
        );
        let h = Harness::inline(&source);
        h.frame();
        h.ui.scene.borrow_mut().flush();
        let calls = h.calls();
        let mut cost = 0;
        for offset in [25., 50., 75.] {
            set(&h, "offset", offset);
            h.frame();
            cost = cost.max(h.ui.scene.borrow_mut().flush().layout_nodes);
            assert_eq!(h.calls(), calls);
        }
        let ordinary_source = source.replace(
            r#"top = ui.map(offset, {
                from = {0, function() return limit:get() end},
                to = {4, function() return 4 + travel:get() end},
                clamp = true,
            })"#,
            "top = function() return 4 + travel:get() * math.clamp(offset:get(), 0, limit:get()) / limit:get() end",
        );
        assert_ne!(source, ordinary_source);
        let mut ordinary = Harness::inline(&ordinary_source);
        ordinary.frame();
        ordinary.ui.scene.borrow_mut().flush();
        let calls = ordinary.calls();
        set(&ordinary, "offset", 50.);
        ordinary.frame();
        let ordinary_cost = ordinary.ui.scene.borrow_mut().flush().layout_nodes;
        assert!(
            ordinary.calls() > calls,
            "ordinary top reruns its Lua function"
        );
        assert_eq!(ordinary.bounds("thumb").y, 54.);
        assert!(
            ordinary_cost > cost,
            "ordinary/mapped: {ordinary_cost}/{cost}"
        );
        eprintln!(
            "mapped rail rows={rows}: layout nodes native={cost}, function={ordinary_cost}; source Lua native=0"
        );
        costs.push(cost);
    }
    assert_eq!(
        costs[0], costs[1],
        "page-independent mapped rail: {costs:?}"
    );
    assert!(costs[0] <= 2, "only rail and thumb: {costs:?}");
}
#[test]
fn map_endpoint_effects_and_native_binding_end_with_the_owning_component() {
    let h = Harness::inline(
        r#"
local offset = ui.state("offset",0)
local limit = ui.state("limit",100)
local open = ui.state("open",true)
local Thumb = ui.component(function()
    return ui.div { id="thumb",absolute=true,w=4,h=24,
        top=ui.map(offset,{from={0,function() return limit:get() end},to={4,104},clamp=true}),
    }
end)
return ui.column {relative=true,w=300,h=220,
    ui.div {absolute=true,w=8,h=200,ui.show(open,Thumb {})},
}
"#,
    );
    h.frame();
    h.host.set_state("open", Value::Bool(false)).unwrap();
    h.frame();
    let calls = h.calls();
    set(&h, "limit", 200.);
    set(&h, "offset", 50.);
    h.frame();
    assert_eq!(h.calls(), calls, "disposed endpoint functions must not run");
}

#[test]
fn mapped_source_keeps_the_normal_interaction_style_cascade_without_lua_per_offset() {
    let source = RAIL.replace(
        "id = \"thumb\", absolute = true",
        "id = \"thumb\", hover = { top = 10 }, absolute = true",
    );
    let mut h = Harness::inline(&source);
    h.hover("thumb");
    h.frame();
    assert_eq!(h.bounds("thumb").y, 10.);
    h.ui.scene.borrow_mut().flush();
    let calls = h.calls();
    set(&h, "offset", 75.);
    h.frame();
    assert_eq!(
        h.bounds("thumb").y,
        10.,
        "hover override keeps its precedence"
    );
    assert_eq!(h.calls(), calls, "cached height function must not run");
    assert_eq!(
        h.ui.scene.borrow_mut().flush().layout_nodes,
        0,
        "an overridden source does no layout"
    );
}

#[test]
fn mapping_does_not_turn_a_contents_nodes_effective_style_into_its_authored_style() {
    let source = r#"
local offset=ui.state("offset",0)
local width=ui.state("width",40)
return ui.column {relative=true,w=300,h=220,
    ui.div {id="contents",display="contents",top=ui.map(offset,{from={0,100},to={0,50}}),
        ui.div {id="child",h=24,ml=7,mt=5,w=function() return width:get() end},
    },
}
"#;
    let mut h = Harness::inline(source);
    for (offset, width) in [(50., 70.), (75., 90.), (100., 50.)] {
        set(&h, "offset", offset);
        set(&h, "width", width);
        h.frame();
        let mut fresh = Harness::inline(source);
        set(&fresh, "offset", offset);
        set(&fresh, "width", width);
        fresh.frame();
        for id in ["contents", "child"] {
            assert_eq!(h.bounds(id), fresh.bounds(id), "{id} at {offset}/{width}");
        }
    }
}

#[test]
fn endpoint_functions_follow_their_own_dependencies_independently() {
    let mut h = Harness::inline(RAIL);
    h.frames(2);
    set(&h, "offset", 50.);
    h.frame();
    let calls = h.calls();
    set(&h, "limit", 200.);
    h.frame();
    assert_eq!(h.bounds("thumb").y, 29.);
    assert_eq!(h.calls() - calls, 1, "only the changed from endpoint runs");
    let calls = h.calls();
    set(&h, "travel", 60.);
    h.frame();
    assert_eq!(h.bounds("thumb").y, 19.);
    assert_eq!(h.calls() - calls, 1, "only the changed to endpoint runs");
}
