//! `set_text`'s history around an answer (`answer_next`) to an undo: what
//! it keeps to undo and redo must still apply to the text it holds.
use zgx::text_edit::TextEditor;

#[test]
fn a_document_loaded_as_an_answer_to_an_undo_leaves_nothing_to_redo() {
    let mut editor = TextEditor::new("abc");
    editor.set_text("abcd");
    assert!(editor.undo());
    editor.answer_next();
    editor.load_next();
    editor.set_text("x");
    // The step undone added "d" at byte 3, of a text that is gone: redoing
    // it into "x" was out of range.
    assert!(!editor.redo());
    assert!(!editor.undo());
    assert_eq!(editor.text(), "x");
}

#[test]
fn an_answer_to_an_undo_as_long_as_the_text_leaves_no_older_step_to_break() {
    let mut editor = TextEditor::new("a");
    editor.set_text("ab");
    editor.set_text("abz");
    assert!(editor.undo());
    assert_eq!(editor.text(), "ab");
    // As many bytes, another character: the step before ("b" put at byte
    // 1) would undo into the middle of "é".
    editor.answer_next();
    editor.set_text("é");
    while editor.undo() {}
    assert_eq!(editor.text(), "é", "nothing older to undo into it");
    // What redoes the undone step is kept, joined with the answer.
    assert!(editor.redo());
    assert_eq!(editor.text(), "abz");
}

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

#[test]
fn history_survives_answers_loads_undo_and_redo_in_any_order() {
    const PIECES: [&str; 9] = ["a", "b", "ab", "é", "漢", "😀", "\n", "", "abab"];
    for seed in 1..=40u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut editor = TextEditor::new("abé漢\nab");
        for _ in 0..60 {
            match rng.below(10) {
                0 => {
                    editor.undo();
                }
                1 => {
                    editor.redo();
                }
                2 => editor.load_next(),
                3 => editor.answer_next(),
                _ => {
                    let text = editor.text().into_owned();
                    let len = text.len() + 2;
                    let (a, b) = (rng.below(len), rng.below(len));
                    let mut start = a.min(b).min(text.len());
                    while !text.is_char_boundary(start) {
                        start -= 1;
                    }
                    let mut end = a.max(b).clamp(start, text.len());
                    while !text.is_char_boundary(end) {
                        end += 1;
                    }
                    let with: String = (0..rng.below(4))
                        .map(|_| PIECES[rng.below(PIECES.len())])
                        .collect();
                    editor.set_text(format!("{}{with}{}", &text[..start], &text[end..]));
                }
            }
        }
        while editor.undo() {}
        while editor.redo() {}
    }
}
