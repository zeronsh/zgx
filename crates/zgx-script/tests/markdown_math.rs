//! A markdown view's `on_math`: TeX between dollars drawn as pictures in the line.
mod common;
use common::Harness;
use zgx_script::ScriptHost;

const BOX: &str = r#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'><rect width='10' height='10'/></svg>"#;

fn app(text: &str, on_math: &str) -> Harness {
    let code = format!(
        r##"local asked = ui.state("asked", "")
return ui.column {{
    ui.markdown_view {{ id = "md", w = 300, text_size = 16, literal = true, {text:?}, {on_math} }},
    ui.text {{ id = "asked", function() return asked:get() end }},
}}"##
    );
    let code: &'static str = Box::leak(code.into_boxed_str());
    let mut h = Harness::new(move |runtime| ScriptHost::inline(runtime, "app", code));
    h.ui.prepare_frame();
    h.settle();
    h.ui.prepare_frame();
    h
}

fn drawing(h: f32) -> String {
    format!(
        r#"on_math = function(tex, display)
            asked:set(asked:get() .. tex .. (if display then "!" else "") .. ";")
            return {{ svg = "{BOX}", w = 1, h = {h}, d = 0.5 }}
        end,"#
    )
}

#[test]
fn inline_math_is_drawn_in_the_line_and_reads_as_its_tex() {
    let mut h = app("a $x^2$ b", &drawing(1.));
    assert_eq!(h.text("asked"), "x^2;");
    let tree = h.tree();
    assert!(tree.contains("\"a x^2 b\""), "{tree}");
}

#[test]
fn a_tall_formula_grows_its_line() {
    let short = app("a $x$ b", &drawing(0.5)).bounds("md").height;
    let tall = app("a $x$ b", &drawing(4.)).bounds("md").height;
    assert!(tall >= short + 40., "short {short}, tall {tall}");
}

#[test]
fn display_math_alone_is_its_own_block() {
    let mut h = app("before\n\n$$\\frac{a}{b}$$\n\nafter", &drawing(2.));
    assert_eq!(h.text("asked"), "\\frac{a}{b}!;");
    let tree = h.tree();
    assert!(tree.contains("before") && tree.contains("after"), "{tree}");
}

#[test]
fn math_it_cannot_draw_shows_as_code() {
    let mut h = app("a $\\nope$ b", "on_math = function() return nil end,");
    assert!(h.tree().contains("\\nope"), "{}", h.tree());
}

#[test]
fn without_on_math_dollars_are_text() {
    let mut h = app("costs $5 and $6", "");
    assert!(h.tree().contains("costs $5 and $6"), "{}", h.tree());
}

#[test]
fn a_long_text_shows_its_start_at_once_and_the_rest_over_frames() {
    let text: String = (0..1_000).map(|i| format!("Paragraph {i}.\n\n")).collect();
    let code = format!(
        "return ui.column {{ ui.markdown_view {{ id = \"md\", w = 300, literal = true, {text:?} }} }}"
    );
    let code: &'static str = Box::leak(code.into_boxed_str());
    let mut h = Harness::new(move |runtime| ScriptHost::inline(runtime, "app", code));
    h.ui.prepare_frame();
    let first = h.tree();
    assert!(first.contains("Paragraph 0."), "{first}");
    let start = h.bounds("md").height;
    h.frames(60);
    let all = h.bounds("md").height;
    assert!(
        start > 600. && start * 8. < all,
        "first frame {start}, all {all}"
    );
}

#[test]
fn find_opened_while_a_long_text_builds_counts_what_comes_after() {
    let text: String = (0..1_000).map(|i| format!("Paragraph {i}, a needle.\n\n")).collect();
    let code = format!("return ui.column {{ ui.markdown_view {{ id = \"md\", w = 300, literal = true, {text:?} }} }}");
    let code: &'static str = Box::leak(code.into_boxed_str());
    let mut h = Harness::new(move |runtime| ScriptHost::inline(runtime, "app", code));
    h.ui.prepare_frame();
    h.ui.find_text("needle");
    let early = h.ui.find_status().count;
    assert!(early > 0 && early < 1_000, "{early}");
    for _ in 0..8 {
        h.frames(10);
        std::thread::sleep(std::time::Duration::from_millis(260));
        h.frames(1);
    }
    assert_eq!(h.ui.find_status().count, 1_000);
    assert_eq!(h.ui.find_status().current, Some(0));
}
