//! Spellchecking an editor (`View::spellcheck`), as a browser's text fields
//! do: the system's checker (`Platform::misspellings`) marks misspelled
//! words with a dotted line, and the context menu offers its guesses, Learn
//! Spelling and Ignore Spelling. An edit checks again only the paragraphs it
//! touched; the marks elsewhere move with the text.
use crate::{platform::PlatformService, widgets::EditorHandle, widgets::Ui};
use std::{cell::RefCell, ops::Range, rc::Rc};

/// Text as last checked, and its misspellings.
type Checked = (String, Vec<Range<usize>>);

pub(crate) fn mount(
    ui: &mut Ui,
    editor: EditorHandle,
    platform: Rc<PlatformService>,
    mut on: Box<dyn FnMut() -> bool>,
) {
    // The text last checked, and its misspellings (`None` while off).
    let checked: Rc<RefCell<Option<Checked>>> = Rc::default();
    let value = editor.value().clone();
    ui.bind(editor.node, move || {
        value.track();
        if !on() {
            // Off: no marks, and a fresh check when on again.
            if checked.borrow_mut().take().is_some() {
                editor.set_misspellings(Vec::new());
            }
            return;
        }
        let text = editor.text();
        let mut checked = checked.borrow_mut();
        let fresh = checked.is_none();
        let (old, ranges) = checked.get_or_insert_with(Default::default);
        if *old == text && !fresh {
            return;
        }
        let (start, end, shift) = changed_paragraphs(old, &text);
        let old_end = (end as isize - shift) as usize;
        let mut next: Vec<Range<usize>> =
            ranges.iter().filter(|r| r.end <= start).cloned().collect();
        let found = platform
            .0
            .misspellings(&text[start..end])
            .into_iter()
            .filter(|r| r.start < r.end && r.end <= end - start)
            .map(|r| r.start + start..r.end + start);
        // UITextField marks only a word the person just ended: nothing already
        // there when it first shows, and in an edited paragraph only that word
        // and what was marked before.
        #[cfg(target_os = "ios")]
        let found = only_typed(found.collect(), ranges, old, &text, fresh).into_iter();
        next.extend(found);
        next.extend(
            ranges
                .iter()
                .filter(|r| r.start >= old_end)
                .map(|r| (r.start as isize + shift) as usize..(r.end as isize + shift) as usize),
        );
        *old = text;
        *ranges = next.clone();
        drop(checked);
        editor.set_misspellings(next);
    });
}

/// The bytes `old` and `new` start with alike, on a character's boundary.
fn common_prefix(old: &str, new: &str) -> usize {
    old.char_indices()
        .zip(new.chars())
        .find(|((_, a), b)| a != b)
        .map_or(old.len().min(new.len()), |((i, _), _)| i)
}
/// The bytes `old` and `new` end with alike.
fn common_suffix(old: &str, new: &str) -> usize {
    old.chars()
        .rev()
        .zip(new.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum()
}

/// Where `old` and `new` differ, widened to whole paragraphs of `new`
/// (`start..end`), and how much longer `new` is.
fn changed_paragraphs(old: &str, new: &str) -> (usize, usize, isize) {
    let prefix = common_prefix(old, new);
    let suffix = common_suffix(&old[prefix..], &new[prefix..]);
    let changed_end = new.len() - suffix;
    let start = new[..prefix].rfind('\n').map_or(0, |i| i + 1);
    let end = new[changed_end..]
        .find('\n')
        .map_or(new.len(), |i| changed_end + i);
    (start, end, new.len() as isize - old.len() as isize)
}

/// Of `found` in `new`, what UITextField would mark (read in a reference app):
/// none on the first look (`fresh`); else a word a typed delimiter just ended
/// (the caret leaving it marks nothing), or one marked before (`marked`, in
/// `old`), wherever the edit moved it.
#[cfg(any(target_os = "ios", test))]
fn only_typed(
    found: Vec<Range<usize>>,
    marked: &[Range<usize>],
    old: &str,
    new: &str,
    fresh: bool,
) -> Vec<Range<usize>> {
    if fresh {
        return Vec::new();
    }
    let prefix = common_prefix(old, new);
    let suffix = common_suffix(&old[prefix..], &new[prefix..]);
    let ended_at = new[prefix..new.len() - suffix]
        .chars()
        .next()
        .is_some_and(|typed| !typed.is_alphanumeric())
        .then_some(prefix);
    let shift = new.len() as isize - old.len() as isize;
    let was_marked = |range: &Range<usize>| {
        marked.iter().any(|before| {
            let moved = if before.start >= old.len() - suffix {
                (before.start as isize + shift) as usize..(before.end as isize + shift) as usize
            } else {
                before.clone()
            };
            moved == *range
        })
    };
    found
        .into_iter()
        .filter(|range| ended_at == Some(range.end) || was_marked(range))
        .collect()
}

/// The context menu's spelling items for the misspelled word the selection
/// is in: its guesses, Learn Spelling and Ignore Spelling. None elsewhere.
pub(crate) fn menu_items(
    editor: &EditorHandle,
    platform: &Rc<PlatformService>,
) -> Vec<crate::compose::View> {
    use crate::compose::menu_item;
    let Some(range) = editor.misspelling_at_selection() else {
        return Vec::new();
    };
    let text = editor.text();
    let Some(word) = text.get(range.clone()).map(str::to_owned) else {
        return Vec::new();
    };
    let guesses = platform.0.spelling_guesses(&word);
    let mut items: Vec<crate::compose::View> = guesses
        .into_iter()
        .take(5)
        .enumerate()
        .map(|(index, guess)| {
            let (editor, range) = (editor.clone(), range.clone());
            menu_item(guess.clone())
                .id(format!("spelling-guess-{index}"))
                .on_click(move || {
                    editor.set_selection(range.start, range.end);
                    editor.paste(&guess);
                })
        })
        .collect();
    if items.is_empty() {
        items.push(
            menu_item("No Guesses Found")
                .id("spelling-none")
                .disabled(true),
        );
    }
    let forget = |editor: &EditorHandle, word: &str| {
        let text = editor.text();
        let left: Vec<_> = editor
            .misspellings()
            .into_iter()
            .filter(|r| text.get(r.clone()) != Some(word))
            .collect();
        editor.set_misspellings(left);
    };
    let (learn, ignore) = (
        (editor.clone(), platform.clone(), word.clone()),
        (editor.clone(), platform.clone(), word),
    );
    items.push(
        menu_item("Learn Spelling")
            .id("spelling-learn")
            .on_click(move || {
                let (editor, platform, word) = &learn;
                platform.0.learn_spelling(word);
                forget(editor, word);
            }),
    );
    items.push(
        menu_item("Ignore Spelling")
            .id("spelling-ignore")
            .on_click(move || {
                let (editor, platform, word) = &ignore;
                platform.0.ignore_spelling(word);
                forget(editor, word);
            }),
    );
    items
}

#[cfg(test)]
mod tests {
    use super::changed_paragraphs;
    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn uikit_marks_only_a_word_just_ended_and_what_it_marked_before() {
        use super::only_typed;
        use std::ops::Range;
        let none = Vec::<Range<usize>>::new();
        // Already there when the field first shows: nothing marked.
        assert_eq!(
            only_typed(vec![0..3, 4..8], &[], "", "teh wrld", true),
            none
        );
        // A space typed after the last word: it alone, not the two before.
        assert_eq!(
            only_typed(
                vec![0..3, 4..8, 9..16],
                &[],
                "teh wrld recieve",
                "teh wrld recieve ",
                false
            ),
            vec![9..16]
        );
        // Punctuation ends a word as a space does.
        assert_eq!(
            only_typed(vec![0..3], &[], "teh", "teh.", false),
            vec![0..3]
        );
        // A letter doesn't: the word is still being typed.
        assert_eq!(only_typed(vec![0..3], &[], "te", "teh", false), none);
        // A word marked before stays while another is typed.
        assert_eq!(
            only_typed(
                vec![0..3, 4..8, 9..12],
                &[0..3],
                "teh wrld ab",
                "teh wrld abc",
                false
            ),
            vec![0..3]
        );
        // Text typed in front moves a marked word, which stays marked.
        assert_eq!(
            only_typed(vec![1..4, 5..9], &[0..3], "teh wrld", "zteh wrld", false),
            vec![1..4]
        );
        // Corrected, it isn't found, so it isn't kept.
        assert_eq!(only_typed(vec![], &[0..3], "teh", "the", false), none);
    }
    #[test]
    fn an_edit_rechecks_only_the_paragraphs_it_touched() {
        assert_eq!(changed_paragraphs("", "ab cd"), (0, 5, 5));
        // Typing in the second paragraph.
        assert_eq!(
            changed_paragraphs("one\ntwo\nsix", "one\ntwoo\nsix"),
            (4, 8, 1)
        );
        // Joining two paragraphs: the joined one.
        assert_eq!(changed_paragraphs("one\ntwo", "onetwo"), (0, 6, -1));
        assert_eq!(changed_paragraphs("héllo", "héllo!"), (0, 7, 1));
    }
}
