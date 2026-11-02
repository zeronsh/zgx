//! A layout that never settles (a size read back into itself) is an error
//! a test sees, its last layout kept, as a window shows it: not a panic.
mod common;
use common::Harness;

#[test]
fn a_layout_that_never_settles_is_an_error_not_a_panic() {
    let mut h = Harness::inline(
        r#"
local r = ui.ref()
return ui.column {
    ui.div { ref = r, h = 20, w = function() return (r.width or 100) + 1 end },
    ui.text { id = "after", "still here" },
}
"#,
    );
    h.frames(3);
    let error = h.error().unwrap_or_default();
    assert!(error.contains("did not settle"), "{error:?}");
    // The app goes on: what it shows answers.
    assert_eq!(h.text("after"), "still here");
}
