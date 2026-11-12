//! A paragraph of words, padded chips and inline pictures wraps inside its width, as a chat answer's does.
mod common;
use common::Harness;
use zgx_script::ScriptHost;

const BOX: &str = r#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'><rect width='10' height='10'/></svg>"#;

fn paragraph(width: f32, chip: &str, extra: &str) -> Harness {
    let code = format!(
        r##"local function chip(label)
    return ui.span {{ label, literal = true, bg = "#dddddd", padding = {{ 4, 2 }}, radius = 6, weight = 500,
        icon = {{ src = "data:image/svg+xml;utf8,{BOX}", size = 14, gap = 2 }}, on_click = function() return true end {chip} }}
end
return ui.column {{ w = {width}, ui.rich {{ id = "p", wrap = true, w_full = true, text_size = 16, line_height = 24,
    "The ideal value is the meet over every path from the entry ", chip("Aho et al., 2007"),
    " of the transfer functions along it, formally ", {extra}
    " and each such path is followed in turn ", chip("Aho et al., 2007"), " so that nothing is lost on the way ", chip("Cooper and Torczon, 2011"),
    " which is the formal statement of precision.",
}} }}"##
    );
    let code: &'static str = Box::leak(code.into_boxed_str());
    let mut h = Harness::new(move |runtime| ScriptHost::inline(runtime, "app", code));
    h.ui.prepare_frame();
    h.settle();
    h.ui.prepare_frame();
    h
}

fn boxes(tree: &str) -> Vec<(f32, f32, f32, f32)> {
    tree.lines()
        .filter(|l| l.contains(" link "))
        .filter_map(|l| {
            let mut words = l.split_whitespace().rev();
            let size = words.next()?;
            let at = words.next()?;
            let (x, y) = at.split_once(',')?;
            let (w, h) = size.split_once('x')?;
            Some((x.parse().ok()?, y.parse().ok()?, w.parse().ok()?, h.parse().ok()?))
        })
        .collect()
}

#[test]
fn padded_chips_wrap_inside_the_width() {
    for width in [390., 300., 240.] {
        let mut h = paragraph(width, "", "");
        let p = h.bounds("p");
        let tree = h.tree();
        for (x, _, w, _) in boxes(&tree) {
            assert!(x + w <= p.x + p.width + 0.5, "{width}: a chip ends at {} past {}\n{tree}", x + w, p.x + p.width);
        }
    }
}

fn formula(ems: f32) -> String {
    format!(r#"ui.span {{ "IDEAL[B] = f", literal = true, inline_svg = {{ svg = "{BOX}", w = {ems}, h = 1, d = 0.4 }} }},"#)
}

fn text_box(tree: &str) -> (f32, f32) {
    let line = tree.lines().find(|l| l.contains(" text \"The ideal")).expect("the paragraph's text");
    let size = line.split_whitespace().last().unwrap();
    let (w, h) = size.split_once('x').unwrap();
    (w.parse().unwrap(), h.parse().unwrap())
}

#[test]
fn a_box_wider_than_the_paragraph_overflows_it_and_the_lines_wrap_at_its_width() {
    let mut h = paragraph(300., "", &formula(25.));
    let p = h.bounds("p");
    let tree = h.tree();
    let (w, height) = text_box(&tree);
    assert_eq!(w, p.width, "the text is laid out at the paragraph's width\n{tree}");
    assert_eq!(height, p.height, "and is as tall as it measured\n{tree}");
    for (x, _, w, _) in boxes(&tree) {
        assert!(x + w <= p.x + p.width + 0.5, "a chip ends at {} past {}\n{tree}", x + w, p.x + p.width);
    }
}

#[test]
fn a_chip_with_no_break_inside_stays_whole() {
    let mut h = paragraph(300., "", "");
    let split = boxes(&h.tree());
    assert!(split.iter().any(|b| b.2 < 20.), "the plain label breaks after its icon: {split:?}");
    let whole = r##"ui.span { "\u{2060}Aho\u{a0}et\u{a0}al.,\u{a0}2007", literal = true, padding = { 4, 2 }, bg = "#dddddd",
        icon = { src = "x.svg", size = 14, gap = 2 }, on_click = function() return true end },"##;
    let mut h = paragraph(300., "", whole);
    let tree = h.tree();
    let chips = boxes(&tree);
    assert!(chips.iter().any(|b| b.2 > 100.), "the whole chip is on one line: {chips:?}\n{tree}");
}
