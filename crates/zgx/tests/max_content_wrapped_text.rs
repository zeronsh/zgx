//! A wrapping text's max-content width is its unwrapped width, as CSS takes it: a box whose width
//! comes from what it holds, under a `max-width`, is min(that, the cap), so text that only just
//! wraps fills to the cap rather than stopping at its widest wrapped line.
use zgx::{compose::prelude::*, size_calc::SizeCalc, widgets::Ui};

const WORDS: &str =
    "the quick brown fox jumps over the lazy dog while the rest of the band plays on";

/// The text's width on one line.
fn unwrapped(words: &str) -> f32 {
    let mut ui = Ui::new(2000., 200.);
    let mounted = ui.mount(row().child(text(words.to_owned()).id("t").text_size(14.)));
    ui.prepare_frame();
    ui.scene.borrow().bounds(mounted.find("t").unwrap()).width
}

/// A bubble holding `words`, sized by what it holds (`width`: None, or a `w` expression), at most 70% of
/// a row `parent` wide; its and its text's bounds.
fn bubble(parent: f32, words: &str, width: Option<&str>) -> (zgx::scene::Rect, zgx::scene::Rect) {
    let mut ui = Ui::new(2000., 400.);
    let mut b = column()
        .id("bubble")
        .max_w_percent(70.)
        .px(16.)
        .text_size(14.)
        .line_height(22.)
        .child(text(words.to_owned()).id("text").text_wrap(true));
    if let Some(width) = width {
        b = b.w_calc(width.parse::<SizeCalc>().unwrap());
    }
    let mounted = ui.mount(row().w(parent).justify_end().child(b));
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    (
        scene.bounds(mounted.find("bubble").unwrap()),
        scene.bounds(mounted.find("text").unwrap()),
    )
}

/// A parent for which the text just wraps in the bubble's content: its cap is `slack` points short of it.
fn just_wrapping(words: &str, slack: f32) -> f32 {
    let content = unwrapped(words) + 32.;
    ((content - slack) / 0.7).floor()
}

#[test]
fn text_that_only_just_wraps_fills_its_box_to_the_cap() {
    // However little room it lacks, wrapped text is as wide as the cap.
    for slack in [3., 10., 17., 25., 33., 48., 70., 110.] {
        let parent = just_wrapping(WORDS, slack);
        let cap = (parent * 0.7).floor();
        for width in [None, Some("fit-content"), Some("max-content")] {
            let (bubble, text) = bubble(parent, WORDS, width);
            assert!(text.height > 22.5, "{slack} {width:?}: it wraps: {text:?}");
            assert!(
                (bubble.width - cap).abs() <= 1.,
                "{slack} {width:?}: the cap {cap}, the bubble {bubble:?}"
            );
        }
    }
}

#[test]
fn text_that_fits_is_as_wide_as_its_line() {
    // Room to spare: the bubble is the line, not the cap.
    let line = unwrapped(WORDS);
    let (bubble, text) = bubble(3000., WORDS, None);
    assert!(text.height < 22.5 * 1.5, "one line: {text:?}");
    assert!(
        (bubble.width - (line + 32.)).abs() <= 1.,
        "{bubble:?} for a line of {line}"
    );
}
