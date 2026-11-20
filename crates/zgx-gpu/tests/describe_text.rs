//! `Scene::describe_text` (what `zgx inspect text` prints): the family a text
//! asks for, what it resolves to, each line's baseline, ascent and descent,
//! and the face each run of glyphs used.
use zgx::scene::{Color, NodeKind, Scene, Style};

#[test]
fn a_text_node_says_its_family_its_lines_and_the_faces_its_runs_used() {
    let mut scene = Scene::new(300., 100.);
    zgx_gpu::TextServices::new().install(&mut scene);
    let id = scene.append(
        scene.root(),
        NodeKind::Text {
            text: "Hello world".into(),
            color: Color(255, 255, 255, 255),
            font_size: 13.,
        },
        Style::default(),
    );
    let report = scene
        .describe_text(id, 300.)
        .expect("a describer is installed");
    let text = report.join("\n");
    assert!(
        text.contains("asked family \"sans-serif\" weight 400"),
        "{text}"
    );
    assert!(text.contains("line 0: baseline "), "{text}");
    assert!(
        text.contains("ascent ") && text.contains("descent "),
        "{text}"
    );
    assert!(text.contains("face \""), "{text}");
    assert!(
        scene.describe_text(scene.root(), 300.).is_none(),
        "not text"
    );
}

/// With the default sans-serif family not installed (Open Sans here), the text
/// font is the system's own; a line of Japanese alone stands on its baseline
/// as the Latin line does (13/20: SF's ascent 13, descent 3, baseline 15).
#[cfg(target_os = "macos")]
#[test]
fn a_pure_japanese_line_stands_where_a_latin_one_does_when_the_default_family_is_missing() {
    let mut scene = Scene::new(300., 100.);
    zgx_gpu::TextServices::new().install(&mut scene);
    let font = zgx::text_layout::FontStyle {
        line_height: zgx::text_layout::LineHeight::px(20.),
        ..Default::default()
    };
    let mut baseline = |text: &str| {
        let id = scene.append(
            scene.root(),
            NodeKind::Text {
                text: text.into(),
                color: Color(255, 255, 255, 255),
                font_size: 13.,
            },
            Style::default(),
        );
        scene.set_font(id, font.clone());
        let report = scene.describe_text(id, 300.).unwrap().join("\n");
        let line = report
            .lines()
            .find(|l| l.starts_with("line 0"))
            .unwrap()
            .to_string();
        line.split_whitespace().nth(3).unwrap().to_string()
    };
    let (latin, mixed, japanese) = (
        baseline("Hello world"),
        baseline("Anaraの使い方"),
        baseline("こんにちは。あなたのガイドです。"),
    );
    assert_eq!(latin, "15");
    assert_eq!(mixed, latin);
    assert_eq!(japanese, latin);
}

/// The generic sans-serif family is the system face on macOS, not a family no
/// Mac has: nothing depends on the fallback path for the text's own font.
#[cfg(target_os = "macos")]
#[test]
fn sans_serif_resolves_to_the_system_face_on_macos() {
    let mut scene = Scene::new(300., 100.);
    zgx_gpu::TextServices::new().install(&mut scene);
    let id = scene.append(
        scene.root(),
        NodeKind::Text {
            text: "Hello".into(),
            color: Color(255, 255, 255, 255),
            font_size: 13.,
        },
        Style::default(),
    );
    let report = scene.describe_text(id, 300.).unwrap().join("\n");
    let asked = report.lines().next().unwrap();
    assert!(asked.contains("ps=.SFNS-"), "{asked}");
    assert!(!asked.contains("face none"), "{asked}");
}
