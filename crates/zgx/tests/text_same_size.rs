//! A text that doesn't wrap measures by its content alone: a change that
//! measures as before (a counter ticking from 21 to 22) is repainted, and
//! nothing around it is laid out again (#1288). One that measures otherwise
//! lays its parent out again, as before.
use zgx::scene::{Align, Color, Layout, NodeId, NodeKind, Scene, Style};

fn text(value: &str, wrap: bool) -> (NodeKind, Style) {
    (
        NodeKind::Text {
            text: value.into(),
            color: Color(255, 255, 255, 255),
            font_size: 14.,
        },
        Style {
            text_wrap: wrap,
            ..Style::default()
        },
    )
}

/// A column of three texts, the middle one `middle`.
fn scene(middle: &str, wrap: bool) -> (Scene, NodeId, [NodeId; 3]) {
    let mut scene = Scene::new(640., 480.);
    let root = scene.root();
    let page = scene.append(
        root,
        NodeKind::Container(Layout::Column),
        Style {
            gap: 6.,
            align: Align::Stretch,
            ..Style::default()
        },
    );
    let mut add = |value: &str, wrap: bool| {
        let (kind, style) = text(value, wrap);
        scene.append(page, kind, style)
    };
    let texts = [
        add("Header", false),
        add(middle, wrap),
        add("Footer", false),
    ];
    scene.flush();
    (scene, page, texts)
}

#[test]
fn a_text_measuring_as_before_lays_nothing_out_again() {
    let (mut live, page, texts) = scene("21 messages", false);
    live.set_text(texts[1], "22 messages");
    assert!(
        !live.layout_pending(page),
        "its parent is laid out as it was"
    );
    assert!(!live.layout_pending(texts[1]));
    live.flush();
    // Where a fresh scene with the new text puts everything.
    let (fresh, _, fresh_texts) = scene("22 messages", false);
    for (a, b) in texts.iter().zip(&fresh_texts) {
        assert_eq!(live.bounds(*a), fresh.bounds(*b));
    }
}

#[test]
fn a_text_measuring_otherwise_lays_its_parent_out_again() {
    // As long as before, but a line taller.
    let (mut live, page, texts) = scene("abc", false);
    let footer = live.bounds(texts[2]);
    live.set_text(texts[1], "a\nb");
    assert!(live.layout_pending(page));
    live.flush();
    assert!(live.bounds(texts[2]).y > footer.y, "the footer moved down");
    let (fresh, _, fresh_texts) = scene("a\nb", false);
    for (a, b) in texts.iter().zip(&fresh_texts) {
        assert_eq!(live.bounds(*a), fresh.bounds(*b));
    }
}

#[test]
fn a_wrapping_text_or_one_not_yet_measured_is_laid_out_again() {
    // Wrapping: its size depends on the room it is given.
    let (mut live, page, texts) = scene("21 messages", true);
    live.set_text(texts[1], "22 messages");
    assert!(live.layout_pending(page));
    // Never laid out: nothing to compare with.
    let mut scene = Scene::new(640., 480.);
    let root = scene.root();
    let (kind, style) = text("21", false);
    let fresh = scene.append(root, kind, style);
    scene.set_text(fresh, "22");
    assert!(scene.layout_pending(fresh));
}

/// What a platform shaper does with a face that falls back for some
/// characters (CJK, emoji): those are wider and their line taller.
fn with_fallback(scene: &mut Scene) {
    scene.set_font_text_measurer(|text, _, _, _| {
        let wide = text.chars().filter(|c| !c.is_ascii()).count() as f32;
        let narrow = text.chars().filter(char::is_ascii).count() as f32;
        let height = if wide > 0. { 22. } else { 17. };
        (narrow * 7. + wide * 14., height)
    });
}

#[test]
fn a_same_length_change_into_a_fallback_face_lays_out_again() {
    // `ab1` and `漢` are three bytes each; the second falls back to a
    // wider, taller face, as the shaper measures it.
    let build = |middle: &str| {
        let (mut scene, page, texts) = scene(middle, false);
        with_fallback(&mut scene);
        scene.flush();
        (scene, page, texts)
    };
    let (mut live, page, texts) = build("ab1");
    let footer = live.bounds(texts[2]);
    live.set_text(texts[1], "漢");
    assert!(
        live.layout_pending(page),
        "measured otherwise: laid out again"
    );
    live.flush();
    assert!(
        live.bounds(texts[2]).y > footer.y,
        "the taller line moved the footer"
    );
    let (fresh, _, fresh_texts) = build("漢");
    for (a, b) in texts.iter().zip(&fresh_texts) {
        assert_eq!(live.bounds(*a), fresh.bounds(*b));
    }
    // And back, a narrower face as long: the same.
    live.set_text(texts[1], "ab1");
    assert!(live.layout_pending(page));
}
