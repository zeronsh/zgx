//! Edits from outside an editor (`TextEditor::apply_external`,
//! `append_external`): taken as `set_text` takes the document they make
//! (its text, return value, revision, undo and redo, `load_next` and
//! `answer_next`), but applied where they are, and moving the selection
//! with the text around it.
use zgx::text_edit::TextEditor;

/// A small deterministic generator: the same edits on every run.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Text made of a few pieces, multi-byte ones among them, so ranges land
/// inside characters too.
fn piece(rng: &mut Rng) -> String {
    const PIECES: [&str; 9] = ["a", "b", "ab", "é", "漢", "😀", "\n", "", "abab"];
    (0..rng.below(4))
        .map(|_| PIECES[rng.below(PIECES.len())])
        .collect()
}

/// `text` with bytes `range` replaced, as `apply_external` clamps them.
fn replaced(text: &str, start: usize, end: usize, with: &str) -> String {
    let mut start = start.min(text.len());
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = end.clamp(start, text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    format!("{}{with}{}", &text[..start], &text[end..])
}

#[test]
fn an_external_edit_is_what_set_text_makes_of_its_document() {
    for seed in 1..=40u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut external = TextEditor::new("abé漢\nab");
        let mut whole = TextEditor::new("abé漢\nab");
        for step in 0..60 {
            let at = format!("seed {seed} step {step}");
            match rng.below(10) {
                // Undo and redo give the same text on both.
                0 => assert_eq!(external.undo(), whole.undo(), "{at}: undo"),
                1 => assert_eq!(external.redo(), whole.redo(), "{at}: redo"),
                2 => {
                    external.load_next();
                    whole.load_next();
                }
                3 => {
                    external.answer_next();
                    whole.answer_next();
                }
                _ => {
                    let len = external.len() + 2;
                    let (a, b) = (rng.below(len), rng.below(len));
                    let (start, end) = (a.min(b), a.max(b));
                    let with = piece(&mut rng);
                    let document = replaced(&whole.text(), start, end, &with);
                    assert_eq!(
                        external.apply_external(start..end, &with),
                        whole.set_text(document),
                        "{at}: what it returns"
                    );
                }
            }
            assert_eq!(external.text(), whole.text(), "{at}: text");
            assert_eq!(external.revision(), whole.revision(), "{at}: revision");
        }
        // Everything undone, then redone, step by step alike.
        while whole.undo() {
            assert!(external.undo(), "seed {seed}: as many undo steps");
            assert_eq!(external.text(), whole.text(), "seed {seed}: undone");
        }
        assert!(!external.undo());
        while whole.redo() {
            assert!(external.redo(), "seed {seed}: as many redo steps");
            assert_eq!(external.text(), whole.text(), "seed {seed}: redone");
        }
        assert!(!external.redo());
    }
}

#[test]
fn a_caret_at_the_end_follows_the_stream_and_one_inside_stays_with_its_text() {
    let mut editor = TextEditor::new("Hello");
    // At the end: it follows what streams in.
    assert!(editor.append_external(", wor"));
    assert!(editor.append_external("ld"));
    assert_eq!(editor.text(), "Hello, world");
    assert_eq!(editor.selection().focus, editor.len());
    // Inside: it stays with the text it is in, as an edit before it moves it.
    editor.set_selection(7, 12);
    assert!(editor.apply_external(0..5, "Goodbye"));
    assert_eq!(editor.text(), "Goodbye, world");
    assert_eq!(editor.selected_text(), "world");
    assert!(editor.append_external("!"));
    assert_eq!(
        editor.selected_text(),
        "world",
        "an edit after it leaves it"
    );
    // set_text, by contrast, puts the caret at the end.
    editor.set_text("Goodbye, world!?");
    assert_eq!(editor.selection().focus, editor.len());
}

#[test]
fn an_external_edit_that_changes_nothing_is_no_step() {
    let mut editor = TextEditor::new("abc");
    assert!(!editor.apply_external(1..2, "b"));
    assert!(!editor.append_external(""));
    assert_eq!(editor.revision(), 0);
    assert!(!editor.undo());
}
