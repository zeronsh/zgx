//! `container` and `queries` as scripts write them: styles that follow the
//! size of the container above, without script running as it changes.
mod common;

use common::Harness;

fn width(h: &mut Harness, id: &str) -> f32 {
    h.frames(1);
    let node = h.find(id).unwrap();
    h.ui.scene.borrow().bounds(node).width
}

#[test]
fn a_query_follows_its_containers_width() {
    let mut h = Harness::inline(
        r##"local wide = ui.state("wide", 1000)
        return ui.column {
            ui.button { id = "narrow", on_click = function() wide:set(450) end, "narrow" },
            ui.button { id = "tiny", on_click = function() wide:set(300) end, "tiny" },
            ui.row { container = true, w = wide,
                ui.div { id = "side", h = 20, w = 0,
                    queries = {
                        { min_width = 600, style = { w = 200 } },
                        { max_width = 300, style = { w = 50 } },
                    },
                },
            },
        }"##,
    );
    assert_eq!(width(&mut h, "side"), 200.);
    h.click("narrow");
    assert_eq!(width(&mut h, "side"), 0.);
    h.click("tiny");
    assert_eq!(width(&mut h, "side"), 50.);
    assert_eq!(h.error(), None);
}

#[test]
fn a_query_with_an_unknown_key_says_what_it_takes() {
    let code = r##"return ui.row { container = true, w = 500,
        ui.div { queries = { { min_w = 600, style = { w = 200 } } } },
    }"##;
    let error = zgx_script::testing::TestApp::try_new(|runtime| {
        zgx_script::ScriptHost::inline(runtime, "app", code)
    })
    .err()
    .unwrap_or_default();
    assert!(error.contains("min_width"), "{error}");
}
