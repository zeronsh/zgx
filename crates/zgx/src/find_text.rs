//! Matching a find query in text as a browser's find in page does: case
//! and accents ignored (ICU's primary strength, which Chrome's and Safari's
//! finds use: "cafe" finds "Café"), composed and decomposed forms alike
//! (NFC and NFD), curly quotes and apostrophes as straight ones. Matches are
//! byte ranges of the text searched, whole characters.
use std::ops::Range;
use unicode_normalization::char::{decompose_canonical, is_combining_mark};

/// `text` folded for matching, and for each byte of it, where the character
/// it came from starts in `text` (one past the end, `text.len()`).
pub(crate) struct Folded {
    pub(crate) text: String,
    starts: Vec<u32>,
}
impl Folded {
    /// Where folded byte range `range` came from in the original text.
    fn original(&self, range: Range<usize>, len: usize) -> Range<usize> {
        let start = self.starts[range.start] as usize;
        // Its last byte's character, through that character's end: the
        // start of the next character it did not come from.
        let last = self.starts[range.end - 1] as usize;
        let end = self.starts[range.end..]
            .iter()
            .map(|at| *at as usize)
            .find(|at| *at > last)
            .unwrap_or(len);
        start..end
    }
}

/// One character, folded: pushed onto `out` (none for a combining mark).
fn fold_char(c: char, out: &mut String) {
    let quote = match c {
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' | '\u{02BC}' => Some('\''),
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' | '\u{2033}' => Some('"'),
        _ => None,
    };
    if let Some(quote) = quote {
        out.push(quote);
        return;
    }
    decompose_canonical(c, |d| {
        if !is_combining_mark(d) {
            out.extend(d.to_lowercase());
        }
    });
}

/// `text` folded (`Folded`).
pub(crate) fn fold(text: &str) -> Folded {
    let mut folded = String::with_capacity(text.len());
    let mut starts = Vec::with_capacity(text.len());
    for (at, c) in text.char_indices() {
        let before = folded.len();
        fold_char(c, &mut folded);
        starts.resize(folded.len(), 0);
        starts[before..].fill(at as u32);
    }
    Folded {
        text: folded,
        starts,
    }
}

/// A query folded once for every text it is matched in.
pub(crate) fn fold_query(query: &str) -> String {
    let mut folded = String::with_capacity(query.len());
    for c in query.chars() {
        fold_char(c, &mut folded);
    }
    folded
}

/// Where folded `query` (`fold_query`) matches in `text`: byte ranges of
/// `text`, in order, not overlapping.
pub(crate) fn matches(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() || text.is_empty() {
        return Vec::new();
    }
    if text.is_ascii() {
        // ASCII folding preserves byte offsets, so these matches can use
        // the original ranges directly.
        if !query.is_ascii() {
            return Vec::new();
        }
        let lowercase;
        let text = if text.bytes().any(|byte| byte.is_ascii_uppercase()) {
            lowercase = text.to_ascii_lowercase();
            lowercase.as_str()
        } else {
            text
        };
        return text
            .match_indices(query)
            .map(|(start, matched)| start..start + matched.len())
            .collect();
    }
    let folded = fold(text);
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = folded.text[from..].find(query) {
        let start = from + at;
        let end = start + query.len();
        let range = folded.original(start..end, text.len());
        // A character folding into several (İ into "i̇") may hold the end of
        // one match and the start of the next: the first keeps it.
        if found
            .last()
            .is_none_or(|last: &Range<usize>| last.end <= range.start)
        {
            found.push(range);
        }
        from = end;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    fn find(text: &str, query: &str) -> Vec<String> {
        matches(text, &fold_query(query))
            .into_iter()
            .map(|r| text[r].to_string())
            .collect()
    }
    #[test]
    fn case_accents_and_composed_forms_fold_away() {
        assert_eq!(find("Café and CAFE", "cafe"), ["Café", "CAFE"]);
        assert_eq!(find("café", "CAFÉ"), ["café"]);
        // Decomposed é (e and U+0301) matches composed, both ways.
        assert_eq!(find("cafe\u{301}!", "café"), ["cafe\u{301}"]);
        assert_eq!(find("café", "cafe\u{301}"), ["café"]);
        assert_eq!(find("Ångström", "angstrom"), ["Ångström"]);
    }
    #[test]
    fn quotes_match_straight_ones_and_ranges_are_whole_characters() {
        assert_eq!(find("it’s “here”", "it's \"here\""), ["it’s “here”"]);
        assert_eq!(find("naïve naive", "naive"), ["naïve", "naive"]);
        assert_eq!(find("aaaa", "aa"), ["aa", "aa"], "not overlapping");
        assert!(find("abc", "").is_empty());
        assert!(find("", "a").is_empty());
        assert!(find("abc", "d").is_empty());
    }

    #[test]
    fn ascii_matches_keep_original_offsets_and_do_not_overlap() {
        let text = "x\r\nABaba abABA";
        assert_eq!(matches(text, &fold_query("ába")), [3..6, 9..12]);
        assert_eq!(matches("x\r\nababa ababa", "aba"), [3..6, 9..12]);
        assert_eq!(
            matches(text, "\r\n").as_slice(),
            std::slice::from_ref(&(1..3))
        );
        assert!(matches(text, &fold_query("日本語")).is_empty());
    }
}
