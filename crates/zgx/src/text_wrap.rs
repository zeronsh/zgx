//! `text-wrap: pretty` and `balance`, as Chromium breaks lines for them: a
//! port of Blink's `ScoreLineBreaker` (a Knuth/Minikin-style optimal fit over
//! the paragraph's break opportunities), with the same constants, windows
//! and fallbacks, so the same widths break in the same places.
//!
//! A paragraph is a list of candidates: a sentinel at its start, then each
//! break opportunity in order, the last at its end. `no_break` is where the
//! line is if it doesn't break there (past any spaces), `if_break` where it
//! ends if it does (before them).

/// Which of the two Chromium asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextWrap {
    /// Lines as even as can be, up to six of them.
    Balance,
    /// No short last line: the last four lines are fitted again.
    Pretty,
}

/// One break opportunity (or the start sentinel, at `no_break == if_break == 0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// Its UTF-8 offset in the paragraph's text.
    pub offset: usize,
    /// Where the line is if it doesn't break here.
    pub no_break: f32,
    /// Where it ends if it does.
    pub if_break: f32,
}

const SCORE_INFINITY: f32 = f32::MAX;
const SCORE_OVERFULL: f32 = 1e12;
const LAST_LINE_PENALTY_MULTIPLIER: f32 = 4.0;
const ORPHANS_PENALTY: f32 = 10000.;
const MIN_CANDIDATES: usize = 3;
const MAX_LINES_FOR_OPTIMAL: usize = 4;
const MAX_LINES_FOR_BALANCE: usize = 6;
const SHORT_LINE_DENOMINATOR: f32 = 3.;
/// `LayoutUnit`'s smallest step, added to the width a line must fit in.
const EPSILON: f32 = 1. / 64.;

/// The candidates each greedy line spans: `(start, end)` candidate indices.
fn greedy_lines(candidates: &[Candidate], width: f32) -> Vec<(usize, usize)> {
    let fit = width.max(0.) + EPSILON;
    let last = candidates.len() - 1;
    let mut lines = Vec::new();
    let mut start = 0;
    while start < last {
        let mut end = start + 1;
        while end < last && candidates[end + 1].if_break - candidates[start].no_break <= fit {
            end += 1;
        }
        lines.push((start, end));
        start = end;
    }
    lines
}

/// Whether a short last line of one word, which greedy left, is worth
/// fitting again (`pretty` only; there are no hyphens here to count).
fn should_optimize(candidates: &[Candidate], last: (usize, usize), width: f32) -> bool {
    let line = candidates[last.1].if_break - candidates[last.0].no_break;
    line < width.max(0.) / SHORT_LINE_DENOMINATOR && last.1 - last.0 == 1
}

/// The candidate each line of `candidates` ends at, fitted by score, or
/// `None` if the fit isn't one with as many lines as `lines`.
fn optimize(
    candidates: &[Candidate],
    width: f32,
    font_size: f32,
    balanced: bool,
    lines: usize,
) -> Option<Vec<usize>> {
    if candidates.len() < MIN_CANDIDATES + 2 {
        return None;
    }
    let width = width.max(0.);
    let hyphen_penalty = width * font_size * 2.;
    let line_penalty = hyphen_penalty * 2.;
    let mut penalty = vec![0_f32; candidates.len()];
    if candidates.len() >= 4 {
        penalty[candidates.len() - 2] += ORPHANS_PENALTY;
    }
    let fit = width + EPSILON;
    // (score, previous, line index)
    let mut scores: Vec<(f32, usize, usize)> = vec![(0., 0, 0)];
    let mut active = 0;
    let last = candidates.len() - 1;
    for end in 1..candidates.len() {
        let is_last = end == last;
        let start_edge = candidates[end].if_break - fit;
        let (mut best, mut best_prev, mut best_hope) = (SCORE_INFINITY, 0, 0_f32);
        let mut start = active;
        while start < end {
            let start_score = scores[start].0;
            if start_score + best_hope >= best {
                start += 1;
                continue;
            }
            let delta = candidates[start].no_break - start_edge;
            let (mut width_score, mut additional) = (0_f32, 0_f32);
            if delta < 0. {
                width_score = SCORE_OVERFULL;
            } else if is_last && !balanced {
                additional = LAST_LINE_PENALTY_MULTIPLIER * penalty[start];
            } else {
                width_score = delta * delta;
            }
            if delta < 0. {
                active = start + 1;
            } else {
                best_hope = width_score;
            }
            let score = start_score + width_score + additional;
            if score <= best {
                best = score;
                best_prev = start;
            }
            start += 1;
        }
        scores.push((
            best + penalty[end] + line_penalty,
            best_prev,
            scores[best_prev].2 + 1,
        ));
    }
    let mut breaks = Vec::new();
    let mut at = last;
    while at > 0 {
        breaks.push(at);
        at = scores[at].1;
    }
    breaks.reverse();
    (breaks.len() == lines).then_some(breaks)
}

/// The candidates (their indices in `candidates`) lines end at in a
/// paragraph of `width` wide lines, when they differ from where greedy
/// breaks them; `None` where Chromium would leave them be.
pub fn fitted_breaks(
    candidates: &[Candidate],
    width: f32,
    font_size: f32,
    wrap: TextWrap,
) -> Option<Vec<usize>> {
    if candidates.len() < 2 {
        return None;
    }
    let lines = greedy_lines(candidates, width);
    if lines.len() < 2 {
        return None;
    }
    let (first, balanced) = match wrap {
        TextWrap::Balance => {
            if lines.len() > MAX_LINES_FOR_BALANCE {
                return None;
            }
            (0, true)
        }
        TextWrap::Pretty => {
            let first = lines.len().saturating_sub(MAX_LINES_FOR_OPTIMAL);
            if lines.len() - first < 2 || !should_optimize(candidates, *lines.last()?, width) {
                return None;
            }
            (first, false)
        }
    };
    let from = lines[first].0;
    let fitted = optimize(
        &candidates[from..],
        width,
        font_size,
        balanced,
        lines.len() - first,
    )?;
    let fitted: Vec<usize> = fitted.into_iter().map(|at| at + from).collect();
    let greedy = lines[first..].iter().map(|line| line.1);
    if fitted.iter().copied().eq(greedy) {
        return None;
    }
    let mut breaks: Vec<usize> = lines[..first].iter().map(|line| line.1).collect();
    breaks.extend(fitted);
    Some(breaks)
}

/// Texts longer than this are left to greedy breaking: fitting them costs a
/// caret per break opportunity, and Chromium only fits the last lines anyway.
const MAX_FITTED_BYTES: usize = 8192;

/// A paragraph of `text` (`start..end`), as candidates, laid out on one line
/// in `layout`.
fn paragraph_candidates(
    text: &str,
    layout: &dyn crate::text_layout::TextLayout,
    start: usize,
    opportunities: &[usize],
) -> Vec<Candidate> {
    let bytes = text.as_bytes();
    let x = |offset: usize| layout.caret(offset).x;
    let origin = x(start);
    let mut candidates = vec![Candidate {
        offset: start,
        no_break: 0.,
        if_break: 0.,
    }];
    for &offset in opportunities {
        let mut past = offset;
        while past > start && matches!(bytes[past - 1], b'\n' | b'\r') {
            past -= 1;
        }
        let mut end = past;
        while end > start && matches!(bytes[end - 1], b' ' | b'\t') {
            end -= 1;
        }
        candidates.push(Candidate {
            offset,
            no_break: x(past) - origin,
            if_break: x(end) - origin,
        });
    }
    candidates
}

/// The spaces of `text` that Chrome breaks its lines at, in a box `width`
/// wide, as `wrap` asks: their byte offsets, `layout` being the text laid
/// out on its lines unwrapped. `None` where greedy breaking stands: a
/// paragraph whose fitted breaks aren't all at a space (a break after an
/// inline icon or between ideographs can't be a line feed of the text's own
/// length) is left as it is.
pub fn fitted_spaces(
    text: &str,
    layout: &dyn crate::text_layout::TextLayout,
    width: f32,
    font_size: f32,
    wrap: TextWrap,
) -> Option<Vec<usize>> {
    if text.len() > MAX_FITTED_BYTES || !width.is_finite() || width <= 0. {
        return None;
    }
    let mut spaces = Vec::new();
    let (mut start, mut opportunities) = (0, Vec::new());
    for (offset, kind) in unicode_linebreak::linebreaks(text) {
        opportunities.push(offset);
        if kind == unicode_linebreak::BreakOpportunity::Mandatory || offset == text.len() {
            let candidates = paragraph_candidates(text, layout, start, &opportunities);
            if let Some(breaks) = fitted_breaks(&candidates, width, font_size, wrap) {
                let at: Vec<usize> = breaks[..breaks.len() - 1]
                    .iter()
                    .map(|&b| candidates[b].offset - 1)
                    .collect();
                if at.iter().all(|&b| text.as_bytes()[b] == b' ') {
                    spaces.extend(at);
                }
            }
            start = offset;
            opportunities.clear();
        }
    }
    (!spaces.is_empty()).then_some(spaces)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Candidates of `words`, each character `10` wide and each space `10`.
    fn words(text: &str) -> Vec<Candidate> {
        let mut candidates = vec![Candidate {
            offset: 0,
            no_break: 0.,
            if_break: 0.,
        }];
        let mut at = 0;
        for word in text.split(' ') {
            let end = at + word.len();
            let space = end < text.len();
            candidates.push(Candidate {
                offset: end + space as usize,
                no_break: (end + space as usize) as f32 * 10.,
                if_break: end as f32 * 10.,
            });
            at = end + 1;
        }
        candidates
    }
    fn offsets(candidates: &[Candidate], breaks: &[usize]) -> Vec<usize> {
        breaks.iter().map(|&b| candidates[b].offset).collect()
    }

    // Hand computed (width 100, font size 10: a line costs 4000, the
    // orphan break 10000, a line's width (100 - w)^2): greedy is
    // "aaaa bbbb" / "ccc dddd" / "ee", whose last line is a short word.
    // Scores by line end: S1 7600, S2 4100, S3 min(S1 + 400, S2 + 4900) =
    // 8000 + 4000 = 12000, S4 4500 + 10000 + 4000 = 18500; the last line from
    // 3 costs 0, from 4 (20 wide) 4 x 10000 more: S5 = 12000 + 4000.
    #[test]
    fn pretty_fits_a_short_last_word_back_into_the_lines() {
        let c = words("aaaa bbbb ccc dddd ee");
        assert_eq!(greedy_lines(&c, 100.), vec![(0, 2), (2, 4), (4, 5)]);
        let breaks = fitted_breaks(&c, 100., 10., TextWrap::Pretty).unwrap();
        // "aaaa " / "bbbb ccc " / "dddd ee"
        assert_eq!(offsets(&c, &breaks), vec![5, 14, 21]);
    }

    // Hand computed (width 140, font size 10: a line 5600, the orphan break
    // 10000): "aaaa bbbb cccc" / "dd" greedy. Balanced, the last line costs its
    // width too: from 2 (70 wide) 4900 + S2 8100 = 13000 against 30000 from 3.
    #[test]
    fn balance_moves_a_word_down_for_even_lines() {
        let c = words("aaaa bbbb cccc dd");
        let breaks = fitted_breaks(&c, 140., 10., TextWrap::Balance).unwrap();
        assert_eq!(offsets(&c, &breaks), vec![10, 17]);
    }

    // Pretty makes the same choice there: from 2 the last line is free
    // (S2 = 8100), from 3 it costs 4 x 10000 more.
    #[test]
    fn pretty_and_balance_agree_on_a_short_last_word() {
        let c = words("aaaa bbbb cccc dd");
        assert_eq!(
            fitted_breaks(&c, 140., 10., TextWrap::Pretty),
            fitted_breaks(&c, 140., 10., TextWrap::Balance)
        );
    }

    #[test]
    fn pretty_leaves_a_last_line_that_is_not_short_alone() {
        // "aaaa bbbb" / "cccc dddd": the last line is 90 wide of 100.
        let c = words("aaaa bbbb cccc dddd");
        assert_eq!(fitted_breaks(&c, 100., 10., TextWrap::Pretty), None);
    }

    #[test]
    fn pretty_leaves_a_last_line_of_several_words_alone() {
        // "aaaaaaaaa bbb" / "c d": short (30 of 130) but breakable inside.
        let c = words("aaaaaaaaa bbb c d");
        assert_eq!(fitted_breaks(&c, 130., 10., TextWrap::Pretty), None);
    }

    #[test]
    fn balance_does_not_apply_past_six_lines() {
        // Seven 80-wide words, a line each at width 100.
        let c = words("aaaaaaaa bbbbbbbb cccccccc dddddddd eeeeeeee ffffffff gggggggg");
        assert_eq!(greedy_lines(&c, 100.).len(), 7);
        assert_eq!(fitted_breaks(&c, 100., 10., TextWrap::Balance), None);
    }

    #[test]
    fn too_few_break_opportunities_are_left_greedy() {
        // Three words: sentinel + 3 is under the 5 candidates Chromium needs.
        let c = words("aaaa bbbb cc");
        assert_eq!(fitted_breaks(&c, 100., 10., TextWrap::Balance), None);
        assert_eq!(fitted_breaks(&c, 100., 10., TextWrap::Pretty), None);
    }

    // Five greedy lines (x, y, "aaaa bbbb", "ccc dddd", "ee"): the window is
    // the last four, from y. Relative to y: S1 4100, S2 7700 + 4000, S3
    // 4200 + 4000, S4 12100 + 4000, S5 8600 + 10000 + 4000, and the last line
    // from 4 is free (16100) against 4 x 10000 more from 5.
    #[test]
    fn pretty_only_fits_the_last_four_lines() {
        let c = words("xxxxxxxxx yyyyyyyyy aaaa bbbb ccc dddd ee");
        assert_eq!(greedy_lines(&c, 100.).len(), 5);
        let breaks = fitted_breaks(&c, 100., 10., TextWrap::Pretty).unwrap();
        assert_eq!(offsets(&c, &breaks), vec![10, 20, 25, 34, 41]);
    }
}
