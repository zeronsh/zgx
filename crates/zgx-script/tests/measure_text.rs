//! `ui.measure_text`: the points a text takes in a style, as the app lays
//! text out, with nothing mounted (a column fitted to its widest cell).
mod common;
use common::Harness;

/// What `code` (an expression) gives, as text, from a loaded script.
fn eval(code: &str) -> String {
    let mut h = Harness::inline(&format!(
        "local v = {code}\nreturn ui.text {{ id = 'out', tostring(v) }}"
    ));
    h.text("out")
}
fn num(code: &str) -> f64 {
    eval(code)
        .parse()
        .unwrap_or_else(|_| panic!("{code}: {}", eval(code)))
}

#[test]
fn a_text_measures_wider_as_it_grows_and_as_its_size_does() {
    let short = num("ui.measure_text('Name').w");
    let long = num("ui.measure_text('Name and surname').w");
    let big = num("ui.measure_text('Name', { size = 28 }).w");
    let h = num("ui.measure_text('Name').h");
    assert!(short > 0. && h > 0., "{short} x {h}");
    assert!(long > short * 2., "{long} vs {short}");
    assert!(big > short * 1.5, "{big} vs {short}");
}

#[test]
fn its_size_is_a_ui_texts_unless_given() {
    assert_eq!(
        num("ui.measure_text('Revenue').w"),
        num("ui.measure_text('Revenue', { size = 14 }).w")
    );
}

#[test]
fn max_w_wraps_it_to_a_taller_paragraph_no_wider_than_that() {
    let text = "'one two three four five six seven eight nine ten'";
    let line = num(&format!("ui.measure_text({text}).h"));
    let width = num(&format!("ui.measure_text({text}, {{ max_w = 80 }}).w"));
    let height = num(&format!("ui.measure_text({text}, {{ max_w = 80 }}).h"));
    assert!(width <= 80., "{width}");
    assert!(height >= line * 3., "{height} vs one line's {line}");
    // Wider than the text: one line, as unwrapped.
    assert_eq!(
        num(&format!("ui.measure_text({text}, {{ max_w = 5000 }}).h")),
        line
    );
}

#[test]
fn a_bold_or_monospaced_text_measures_in_that_font() {
    let plain = num("ui.measure_text('iiiiiiii').w");
    let mono = num("ui.measure_text('iiiiiiii', { font = 'mono' }).w");
    assert!(mono > plain, "{mono} vs {plain}");
}

#[test]
fn a_max_w_that_is_no_width_is_an_error() {
    let h = zgx_script::testing::TestApp::try_new(|runtime| {
        zgx_script::ScriptHost::inline(
            runtime,
            "app",
            "ui.measure_text('x', { max_w = 0 })\nreturn ui.column {}\n",
        )
    });
    let error = h.err().expect("it fails to load");
    assert!(
        error.contains("app:1:") && error.contains("`max_w` is a number of points above 0"),
        "{error}"
    );
}
