//! A streaming reply's veil: the words arriving laid out at once and fading
//! in, colour only, so nothing moves or wraps for them. Each paragraph
//! remembers what it has shown; what's new is a chunk, fading exactly
//! once, its time following how fast text is coming (an average of the
//! gaps between chunks, three of them, between 120 and 400 ms), a little
//! faster while three or more fade at once. A chunk's fade is its runs'
//! `fade_in`: drawn a frame at a time in Rust, laying nothing out.
//!
//! Kept by the reply's key, as a script keeps it, so a view made again (a
//! row scrolled back) doesn't fade what it showed. `seed`: what is there as
//! the view is first made stays as it is (a chat opened mid-reply).
use std::{cell::RefCell, collections::HashMap, rc::Rc};

const MIN: f64 = 120.;
const MAX: f64 = 400.;
const EMA_SEED: f64 = 160.;
const GAP_MAX: f64 = 1000.;
/// How a chunk's alpha eases out: `1 - (1 - t)^1.6`.
pub(crate) const POWER: f32 = 1.6;

/// What of a paragraph's text arrived together: bytes `from..to`, fading
/// from `started` (ms, as `ui.now()`) over `ms`, `boost` times as fast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chunk {
    pub from: usize,
    pub to: usize,
    pub started: f64,
    pub ms: f64,
    pub boost: f64,
}
impl Chunk {
    /// Its fade, as a span's `fade_in`.
    pub fn fade(&self) -> zgx::compose::TextFade {
        zgx::compose::TextFade {
            start: self.started,
            ms: ((self.ms / self.boost) as f32).max(0.),
            power: POWER,
        }
    }
}

/// A paragraph's: what it has shown, what is still fading, and how fast
/// text has been coming.
struct Elem {
    text: String,
    chunks: Vec<Chunk>,
    ema: f64,
    last: Option<f64>,
}

/// A reply's veil: its paragraphs' by their place (top-level block, and
/// the paragraph's place in it, from 1).
pub(crate) struct Veil {
    elems: HashMap<(usize, u32), Elem>,
    pub seeding: bool,
}

/// The veils by their replies' keys, and how many were made since they
/// were last forgotten.
type Veils = (HashMap<Rc<str>, Rc<RefCell<Veil>>>, usize);
thread_local! {
    static VEILS: RefCell<Veils> = RefCell::default();
}
/// The veil kept by `key`, made (seeding or not) if there is none. (A
/// reply's blocks a chat at a time; forgotten in a lump now and then.)
pub(crate) fn veil_of(key: &str, seeding: bool) -> Rc<RefCell<Veil>> {
    VEILS.with_borrow_mut(|(veils, count)| {
        if let Some(v) = veils.get(key) {
            return v.clone();
        }
        *count += 1;
        if *count > 400 {
            veils.clear();
            *count = 1;
        }
        let v = Rc::new(RefCell::new(Veil {
            elems: HashMap::new(),
            seeding,
        }));
        veils.insert(key.into(), v.clone());
        v
    })
}

impl Veil {
    /// What of `text`, paragraph `at`'s now, is still fading at `now` (ms),
    /// after what has newly arrived is added.
    pub fn chunks(&mut self, at: (usize, u32), text: &str, now: f64) -> Vec<Chunk> {
        let seeding = self.seeding;
        let e = self.elems.entry(at).or_insert_with(|| Elem {
            text: if seeding {
                text.to_owned()
            } else {
                String::new()
            },
            chunks: Vec::new(),
            ema: EMA_SEED,
            last: None,
        });
        if text != e.text {
            let (new, old) = (text.as_bytes(), e.text.as_bytes());
            let most = new.len().min(old.len());
            // What it had shown, grown at its end (a stream's words), or
            // where they part.
            let mut p = if new.starts_with(old) { most } else { 0 };
            while p < most && new[p] == old[p] {
                p += 1;
            }
            // (Back to a whole character.)
            while p > 0 && p < new.len() && new[p] & 0xC0 == 0x80 {
                p -= 1;
            }
            e.chunks.retain_mut(|c| {
                c.to = c.to.min(p);
                c.from < p
            });
            if new.len() > p {
                if let Some(last) = e.last {
                    e.ema = e.ema * 0.7 + (now - last).min(GAP_MAX) * 0.3;
                }
                e.last = Some(now);
                e.chunks.push(Chunk {
                    from: p,
                    to: new.len(),
                    started: now,
                    ms: (e.ema * 3.).clamp(MIN, MAX),
                    boost: 1.,
                });
            }
            e.text = text.to_owned();
        }
        e.chunks.retain(|c| c.ms - (now - c.started) > 0.);
        // (A stream backed up, three or more at once: a little faster.)
        let boost = 1. + 0.3 * (e.chunks.len() as f64 - 2.).max(0.);
        for c in &mut e.chunks {
            c.boost = boost;
        }
        e.chunks.clone()
    }
}

/// Bytes `from..to` of a paragraph's text cut where a fading chunk starts
/// and ends: each piece, with its chunk if it is fading.
pub(crate) fn pieces(
    fading: &[Chunk],
    from: usize,
    to: usize,
) -> impl Iterator<Item = (usize, usize, Option<Chunk>)> + '_ {
    let mut at = from;
    let mut out = Vec::new();
    for c in fading {
        let (s, e) = (c.from.max(from), c.to.min(to));
        if s < e {
            if s > at {
                out.push((at, s, None));
            }
            out.push((s, e, Some(*c)));
            at = e;
        }
    }
    if at < to {
        out.push((at, to, None));
    }
    out.into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_arrives_fades_once_at_the_pace_text_comes() {
        let mut v = Veil {
            elems: HashMap::new(),
            seeding: false,
        };
        let c = v.chunks((0, 1), "Hello", 1000.);
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].from, c[0].to, c[0].ms), (0, 5, 400.));
        // 25 ms on: a gap of 25 brings the pace down.
        let c = v.chunks((0, 1), "Hello world", 1025.);
        assert_eq!(c.len(), 2);
        assert_eq!((c[1].from, c[1].to), (5, 11));
        assert!((c[1].ms - (160. * 0.7 + 25. * 0.3) * 3.).abs() < 1e-9);
        // The same text: nothing new, what ended gone.
        let c = v.chunks((0, 1), "Hello world", 1500.);
        assert!(c.is_empty());
        // Three fading at once: a little faster.
        for (i, t) in ["a", "ab", "abc"].iter().enumerate() {
            v.chunks((1, 1), t, 2000. + i as f64);
        }
        let c = v.chunks((1, 1), "abc", 2003.);
        assert_eq!(c.len(), 3);
        assert!((c[0].boost - 1.3).abs() < 1e-9);
        // Changed before its end: what was fading past the change is cut.
        let c = v.chunks((1, 1), "aXc", 2004.);
        assert_eq!(
            c.iter().map(|c| (c.from, c.to)).collect::<Vec<_>>(),
            [(0, 1), (1, 3)]
        );
    }

    #[test]
    fn seeding_shows_what_is_there_without_a_fade() {
        let mut v = Veil {
            elems: HashMap::new(),
            seeding: true,
        };
        assert!(v.chunks((0, 1), "Already here", 0.).is_empty());
        v.seeding = false;
        assert_eq!(v.chunks((0, 1), "Already here, and more", 10.).len(), 1);
    }

    #[test]
    fn pieces_cut_where_chunks_start_and_end() {
        let c = |from, to| Chunk {
            from,
            to,
            started: 0.,
            ms: 100.,
            boost: 1.,
        };
        let fading = [c(3, 6), c(6, 9)];
        let cut: Vec<_> = pieces(&fading, 0, 8)
            .map(|(s, e, c)| (s, e, c.is_some()))
            .collect();
        assert_eq!(cut, [(0, 3, false), (3, 6, true), (6, 8, true)]);
        let cut: Vec<_> = pieces(&fading, 8, 12)
            .map(|(s, e, c)| (s, e, c.is_some()))
            .collect();
        assert_eq!(cut, [(8, 9, true), (9, 12, false)]);
    }
}
