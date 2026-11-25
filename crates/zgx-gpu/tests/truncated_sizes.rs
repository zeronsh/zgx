//! What layout makes of a one-line text with an ellipsis, as real fonts
//! measure it: its natural size is the whole text's, its width in a narrower
//! room is its cut line's, and a badge beside it stands where the row says.
use zgx::{compose::prelude::*, scene::Rect, widgets::Ui};
use zgx_gpu::GpuRenderer;

const TITLE: &str = "Reviewing internationalization requirements across departments";
/// Room for "Reviewing" and part of the next word: where a cut inside a word
/// and one at a word differ.
const ROOM: f32 = 150.;
const BADGE: f32 = 30.;

fn title(cut: bool) -> View {
    let title = text(TITLE).id("title").text_size(14.);
    match cut {
        true => title.truncate(),
        false => title,
    }
}
/// The title's and the badge's bounds in `view`, laid out.
fn laid_out(gpu: &GpuRenderer, view: View) -> (Rect, Option<Rect>) {
    let mut ui = Ui::new(2000., 400.);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let mounted = ui.mount(column().items_start().child(view));
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    let bounds = |id: &str| mounted.find(id).map(|id| scene.bounds(id));
    (bounds("title").unwrap(), bounds("badge"))
}
fn badge() -> View {
    div().id("badge").size(BADGE, 14.).flex_shrink(0.)
}

#[test]
fn a_truncated_title_is_as_wide_as_its_cut_and_its_row_places_the_badge() {
    let gpu = GpuRenderer::new(16, 16).unwrap();
    let whole = laid_out(&gpu, title(false)).0;
    let natural = laid_out(&gpu, title(true)).0;
    let first = {
        let word = text("Reviewing\u{2026}").id("title").text_size(14.);
        laid_out(&gpu, word).0.width
    };
    let fit = laid_out(&gpu, column().w(ROOM).items_start().child(title(true))).0;
    let capped = laid_out(&gpu, title(true).max_w(ROOM)).0;
    let least = laid_out(&gpu, row().w(1.).child(title(true).flex_shrink(1.))).0;
    let plain_least = laid_out(&gpu, row().w(1.).child(title(false).flex_shrink(1.))).0;
    let shrunk = laid_out(
        &gpu,
        row()
            .w(ROOM)
            .gap(4.)
            .child(title(true).min_w(0.).flex_shrink(1.))
            .child(badge()),
    );
    let grown = laid_out(
        &gpu,
        row()
            .w(ROOM)
            .gap(4.)
            .child(title(true).min_w(0.).grow().flex_shrink(1.))
            .child(badge()),
    );
    let beside = laid_out(
        &gpu,
        row().gap(4.).child(title(true).max_w(ROOM)).child(badge()),
    );
    println!("first word and dots  {first:.2}");
    println!(
        "whole, untruncated   {:.2} x {:.2}",
        whole.width, whole.height
    );
    println!(
        "natural, truncated   {:.2} x {:.2}",
        natural.width, natural.height
    );
    println!(
        "in a {ROOM} column     {:.2} x {:.2}",
        fit.width, fit.height
    );
    println!(
        "max_w({ROOM})          {:.2} x {:.2}",
        capped.width, capped.height
    );
    println!(
        "in a 1-wide row      {:.2} (untruncated {:.2})",
        least.width, plain_least.width
    );
    for (name, (title, badge)) in [
        ("shrinks, then badge", &shrunk),
        ("grows, then badge  ", &grown),
        ("max_w, then badge  ", &beside),
    ] {
        println!(
            "{name}  title {:.2}, badge at {:.2}",
            title.width,
            badge.unwrap().x
        );
    }
    assert_eq!(
        natural.width, whole.width,
        "its natural width is the whole text's"
    );
    assert!(whole.width > ROOM, "the title overflows the room");
    assert!(
        first < ROOM - 40.,
        "the first word alone is well short of it: {first}"
    );
    for cut in [fit, capped] {
        assert_eq!(cut.height, natural.height, "one line");
        // The cut takes what fits, inside the word that overflows: less than a
        // wide glyph of the room is left, where a cut at the word before it
        // leaves far more.
        assert!(
            cut.width <= ROOM && cut.width > ROOM - 20.,
            "{} of {ROOM}",
            cut.width
        );
    }
    for (title, badge) in [shrunk, grown] {
        let badge = badge.unwrap();
        assert_eq!(badge.x + badge.width, ROOM, "the badge ends the row");
        assert_eq!(
            title.x + title.width + 4.,
            badge.x,
            "just after the title's box"
        );
    }
    let (title, badge) = beside;
    assert_eq!(badge.unwrap().x, title.x + title.width + 4.);
}
