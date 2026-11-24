//! GFM's autolink literals (`Options::autolinks`), which pulldown-cmark
//! doesn't read: `https://…`, `www.…` and `a@b.c` in a block's text become
//! links, as a web chat rendering markdown with remark-gfm shows them
//! (<https://github.github.com/gfm/#autolinks-extension->; `ftp://` is no
//! link there, so none here).
//!
//! It works on a block's runs once they are read (`link_runs`), so text the
//! parser gave in pieces (an entity, a delimiter that marked nothing) reads
//! whole, a link spans runs of other marks and keeps them, and code, links,
//! files and pictures are left as they are.

use super::Run;
use std::cell::RefCell;

thread_local! {
    /// The markdown being read with autolinks, and where it starts in the
    /// whole text (`read_from`), for the length of that parse (`release`):
    /// kept apart from the parse's own state, so a parse without them reads
    /// as it did.
    static SOURCE: RefCell<(String, usize)> = const { RefCell::new((String::new(), 0)) };
}

/// What `release` keeps of the copy's capacity: a short reply's, so its
/// next chunk doesn't allocate again; a long one's goes with the parse.
const KEPT_CAPACITY: usize = 4096;

/// The markdown a parse with autolinks is about to read, from `offset` in
/// the whole text: what its links read of what was written
/// (`entity_in_source`), until the parse ends (`release`).
#[cold]
#[inline(never)]
pub(super) fn read_from(text: &str, offset: usize) {
    SOURCE.with_borrow_mut(|(source, at)| {
        source.clear();
        source.push_str(text);
        *at = offset;
    });
}

/// The parse that `read_from` began has ended: its copy goes, so a long
/// text isn't held at rest (a small one's capacity stays, `KEPT_CAPACITY`).
#[cold]
#[inline(never)]
pub(super) fn release() {
    SOURCE.with_borrow_mut(|(source, at)| {
        if source.capacity() > KEPT_CAPACITY {
            *source = String::new();
        } else {
            source.clear();
        }
        *at = 0;
    });
}

/// A link found in text: its bytes and where it goes.
#[derive(Debug, PartialEq)]
pub(super) struct Found {
    pub start: usize,
    pub end: usize,
    pub to: String,
}

/// Whether `c` may come just before a literal: a line's start, a space, or
/// GFM's delimiters (`*`, `_`, `~`, `(`).
fn opens_after(c: char) -> bool {
    c.is_whitespace() || matches!(c, '*' | '_' | '~' | '(')
}

/// The autolink literals in `text`. A literal may start at a byte in
/// `edges` too (where a run of other marks starts: a delimiter the parser
/// took was there). `source` is the markdown it was read from, for what
/// the text alone can't say: whether a link's last character was an
/// entity there (`&amp;`), which GFM leaves out of it as written.
pub(super) fn find(text: &str, edges: &[usize], source: &str) -> Vec<Found> {
    let mut found = Vec::new();
    let bytes = text.as_bytes();
    let mut at = 0;
    // Where the last link ended: an email's local part starts after it.
    let mut floor = 0;
    while at < bytes.len() {
        let b = bytes[at];
        let opens = || {
            at == 0
                || edges.contains(&at)
                || text[..at].chars().next_back().is_some_and(opens_after)
        };
        let link = match b {
            b'h' | b'H' if opens() => url(text, at, source),
            b'w' | b'W' if opens() => www(text, at, source),
            b'@' => email(text, floor, at),
            _ => None,
        };
        match link {
            Some(link) => {
                at = link.end;
                floor = link.end;
                found.push(link);
            }
            None => at += 1,
        }
    }
    found
}

/// `http://` or `https://` (any case) and a valid domain at `at`, a
/// period in it or not (`https://localhost:3000`, as micromark's
/// `domainAfter`, remark-gfm's, links it).
fn url(text: &str, at: usize, source: &str) -> Option<Found> {
    let rest = &text[at..];
    let scheme = ["https://", "http://"].into_iter().find(|s| {
        rest.get(..s.len())
            .is_some_and(|r| r.eq_ignore_ascii_case(s))
    })?;
    let end = at + extent(text, at, scheme.len(), false, source)?;
    Some(Found {
        start: at,
        end,
        to: text[at..end].to_string(),
    })
}

/// `www.` (any case) and a valid domain at `at`: linked to `http://`.
fn www(text: &str, at: usize, source: &str) -> Option<Found> {
    let rest = &text[at..];
    if !rest
        .get(..4)
        .is_some_and(|r| r.eq_ignore_ascii_case("www."))
    {
        return None;
    }
    let end = at + extent(text, at, 0, true, source)?;
    Some(Found {
        start: at,
        end,
        to: format!("http://{}", &text[at..end]),
    })
}

/// How far a URL starting at `at` in `all` reaches (from there), its
/// domain `domain` bytes on: trailing punctuation, an unbalanced `)` and
/// an entity left out, whether in the text (`&hl;`) or in its `source`
/// (`&amp;`, read as `&`). `None` if its domain isn't valid (needing a
/// period with `dot`).
fn extent(all: &str, at: usize, domain: usize, dot: bool, source: &str) -> Option<usize> {
    let text = &all[at..];
    // To a space or a `<`.
    let mut end = text
        .find(|c: char| c.is_whitespace() || c == '<')
        .unwrap_or(text.len());
    loop {
        let link = &text[..end];
        let Some(last) = link.chars().next_back() else {
            break;
        };
        let cut = end - last.len_utf8();
        if cut > domain && entity_in_source(source, &text[..cut]) {
            end = cut;
            continue;
        }
        let unbalanced = || link.matches(')').count() > link.matches('(').count();
        if matches!(last, '?' | '!' | '.' | ',' | ':' | '*' | '_' | '~')
            || (last == ')' && unbalanced())
        {
            end -= 1;
        } else if last == ';'
            && let Some(amp) = link.rfind('&')
            && !link[amp + 1..end - 1].is_empty()
            && link[amp + 1..end - 1]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric())
        {
            end = amp;
        } else {
            break;
        }
    }
    let host = &text[domain..end];
    let host_end = host
        .find(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .unwrap_or(host.len());
    valid_domain(&host[..host_end], dot).then_some(end)
}

/// Whether, in `source`, the text `before` is followed by an entity-like
/// `&name;` or `&#…;` (so the character after `before` in the text is that
/// entity, read). The first place `before` is written decides.
fn entity_in_source(source: &str, before: &str) -> bool {
    let Some(at) = source.find(before) else {
        return false;
    };
    let rest = &source[at + before.len()..];
    let Some(name) = rest.strip_prefix('&') else {
        return false;
    };
    let name = name.strip_prefix('#').unwrap_or(name);
    let len = name
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(name.len());
    len > 0 && name[len..].starts_with(';')
}

/// GFM's valid domain: segments of letters, digits, `_` and `-` between
/// periods, at least one period (with `dot`), and no `_` in the last two
/// segments.
fn valid_domain(domain: &str, dot: bool) -> bool {
    let domain = domain.trim_end_matches('.');
    let segments: Vec<&str> = domain.split('.').collect();
    segments.len() >= if dot { 2 } else { 1 }
        && segments.iter().all(|s| !s.is_empty())
        && segments.iter().rev().take(2).all(|s| !s.contains('_'))
}

/// An email around the `@` at `at`: a local part of letters, digits, `.`,
/// `-`, `_` and `+` (from `floor` on), and a domain of letters, digits, `-`
/// and `_` between periods, with at least one period and ending in neither
/// `-` nor `_` (a trailing `.` left out).
fn email(text: &str, floor: usize, at: usize) -> Option<Found> {
    let local = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+');
    let start = text[floor..at]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| local(c))
        .last()
        .map(|(i, _)| floor + i)?;
    let after = &text[at + 1..];
    let mut end = after
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .unwrap_or(after.len());
    while after[..end].ends_with('.') {
        end -= 1;
    }
    let domain = &after[..end];
    if !domain.contains('.') || domain.ends_with(['-', '_']) || domain.split('.').any(str::is_empty)
    {
        return None;
    }
    let end = at + 1 + end;
    Some(Found {
        start,
        end,
        to: format!("mailto:{}", &text[start..end]),
    })
}

/// Whether a run's text may hold a literal: not code, a link, a file or a
/// picture.
fn plain(run: &Run) -> bool {
    !run.code && run.link.is_none() && run.file.is_none() && run.img.is_none()
}

/// A block's runs with their autolink literals linked: each stretch of
/// plain runs is read as one text, and its runs split where a link starts
/// or ends, keeping their marks. `top` is where the block's top-level
/// element starts in the whole text: its links read the markdown from
/// there (`read_from`).
pub(super) fn link_runs(runs: &mut Vec<Run>, top: usize) {
    SOURCE.with_borrow(|(text, offset)| {
        let source = text.get(top.saturating_sub(*offset)..).unwrap_or(text);
        link_runs_in(runs, source);
    });
}

/// `link_runs`, reading `source` for what was written.
fn link_runs_in(runs: &mut Vec<Run>, source: &str) {
    let mut out: Vec<Run> = Vec::with_capacity(runs.len());
    let mut i = 0;
    while i < runs.len() {
        if !plain(&runs[i]) {
            out.push(std::mem::take(&mut runs[i]));
            i += 1;
            continue;
        }
        let first = i;
        while i < runs.len() && plain(&runs[i]) {
            i += 1;
        }
        let stretch = &mut runs[first..i];
        let mut text = String::new();
        let mut edges = Vec::new();
        for run in stretch.iter() {
            if !text.is_empty() {
                edges.push(text.len());
            }
            text.push_str(&run.text);
        }
        let found = find(&text, &edges, source);
        if found.is_empty() {
            out.extend(stretch.iter_mut().map(std::mem::take));
            continue;
        }
        let mut offset = 0;
        for run in stretch.iter_mut() {
            let (from, to) = (offset, offset + run.text.len());
            offset = to;
            let mut cut = from;
            let piece = |out: &mut Vec<Run>, a: usize, b: usize, link: Option<&str>| {
                if a < b {
                    out.push(Run {
                        text: text[a..b].to_string(),
                        style: run.style,
                        link: link.map(str::to_string),
                        ..Run::default()
                    });
                }
            };
            for f in found.iter().filter(|f| f.start < to && f.end > from) {
                let (a, b) = (f.start.max(from), f.end.min(to));
                piece(&mut out, cut, a, None);
                piece(&mut out, a, b, Some(&f.to));
                cut = b;
            }
            piece(&mut out, cut, to, None);
        }
    }
    *runs = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The links in `text`, as `[text](to)`, the rest as it is.
    fn shown(text: &str) -> String {
        let mut out = String::new();
        let mut at = 0;
        for f in find(text, &[], text) {
            out.push_str(&text[at..f.start]);
            out.push_str(&format!("[{}]({})", &text[f.start..f.end], f.to));
            at = f.end;
        }
        out.push_str(&text[at..]);
        out
    }

    #[track_caller]
    fn links(text: &str, want: &str) {
        assert_eq!(shown(text), want, "{text:?}");
    }

    // GFM's examples (621–636), but `ftp://`, which remark-gfm doesn't link.
    #[test]
    fn www_literals_link_to_http() {
        links(
            "www.commonmark.org",
            "[www.commonmark.org](http://www.commonmark.org)",
        );
        links(
            "Visit www.commonmark.org/help for more information.",
            "Visit [www.commonmark.org/help](http://www.commonmark.org/help) for more information.",
        );
        links(
            "Visit www.commonmark.org.",
            "Visit [www.commonmark.org](http://www.commonmark.org).",
        );
        links(
            "Visit www.commonmark.org/a.b.",
            "Visit [www.commonmark.org/a.b](http://www.commonmark.org/a.b).",
        );
    }

    #[test]
    fn parentheses_balance_and_trailing_ones_stay_out() {
        links(
            "www.google.com/search?q=Markup+(business)",
            "[www.google.com/search?q=Markup+(business)](http://www.google.com/search?q=Markup+(business))",
        );
        links(
            "www.google.com/search?q=Markup+(business)))",
            "[www.google.com/search?q=Markup+(business)](http://www.google.com/search?q=Markup+(business))))",
        );
        links(
            "(www.google.com/search?q=Markup+(business))",
            "([www.google.com/search?q=Markup+(business)](http://www.google.com/search?q=Markup+(business)))",
        );
        links(
            "(www.google.com/search?q=Markup+(business)",
            "([www.google.com/search?q=Markup+(business)](http://www.google.com/search?q=Markup+(business))",
        );
        links(
            "www.google.com/search?q=(business))+ok",
            "[www.google.com/search?q=(business))+ok](http://www.google.com/search?q=(business))+ok)",
        );
    }

    #[test]
    fn an_entity_like_end_and_a_less_than_stay_out() {
        links(
            "www.google.com/search?q=commonmark&hl=en",
            "[www.google.com/search?q=commonmark&hl=en](http://www.google.com/search?q=commonmark&hl=en)",
        );
        links(
            "www.google.com/search?q=commonmark&hl;",
            "[www.google.com/search?q=commonmark](http://www.google.com/search?q=commonmark)&hl;",
        );
        links(
            "www.commonmark.org/he<lp",
            "[www.commonmark.org/he](http://www.commonmark.org/he)<lp",
        );
    }

    #[test]
    fn http_literals_link_as_written() {
        links(
            "http://commonmark.org",
            "[http://commonmark.org](http://commonmark.org)",
        );
        links(
            "(Visit https://encrypted.google.com/search?q=Markup+(business))",
            "(Visit [https://encrypted.google.com/search?q=Markup+(business)](https://encrypted.google.com/search?q=Markup+(business)))",
        );
        links(
            "Anonymous FTP is available at ftp://foo.bar.baz.",
            "Anonymous FTP is available at ftp://foo.bar.baz.",
        );
        links("HTTPS://A.B/c", "[HTTPS://A.B/c](HTTPS://A.B/c)");
    }

    #[test]
    fn a_parses_copy_of_its_markdown_goes_with_the_parse() {
        let options = super::super::Options {
            autolinks: true,
            ..Default::default()
        };
        let long = "See https://a.b/c&amp; now.\n\n".repeat(4_000);
        let blocks = super::super::parse(&long, &options);
        assert_eq!(blocks.len(), 4_000);
        // Nothing of a long text is held at rest; a short one's capacity
        // stays for the next parse.
        SOURCE.with_borrow(|(source, at)| {
            assert!(source.is_empty() && *at == 0);
            assert!(source.capacity() <= KEPT_CAPACITY, "{}", source.capacity());
        });
        super::super::parse("www.a.com", &options);
        SOURCE.with_borrow(|(source, _)| {
            assert!(source.is_empty());
            assert!(source.capacity() > 0 && source.capacity() <= KEPT_CAPACITY);
        });
    }

    #[test]
    fn emails_link_to_mailto() {
        links("foo@bar.baz", "[foo@bar.baz](mailto:foo@bar.baz)");
        links(
            "hello@mail+xyz.example isn't valid, but hello+xyz@mail.example is.",
            "hello@mail+xyz.example isn't valid, but [hello+xyz@mail.example](mailto:hello+xyz@mail.example) is.",
        );
        links("a.b-c_d@a.b", "[a.b-c_d@a.b](mailto:a.b-c_d@a.b)");
        links("a.b-c_d@a.b.", "[a.b-c_d@a.b](mailto:a.b-c_d@a.b).");
        links("a.b-c_d@a.b-", "a.b-c_d@a.b-");
        links("a.b-c_d@a.b_", "a.b-c_d@a.b_");
    }

    #[test]
    fn a_literal_starts_only_after_a_space_or_a_delimiter() {
        links("foohttp://a.com x", "foohttp://a.com x");
        links("xwww.a.com", "xwww.a.com");
        links("*www.a.com*", "*[www.a.com](http://www.a.com)*");
        links("~https://a.b~", "~[https://a.b](https://a.b)~");
        // No domain needs a period, as micromark's (remark-gfm's) has it:
        // `domainAfter` refuses only an empty domain or a `_` in its last
        // two parts. Nothing after `www.` is no domain.
        links(
            "https://localhost:3000/x",
            "[https://localhost:3000/x](https://localhost:3000/x)",
        );
        links("www.localhost x", "[www.localhost](http://www.localhost) x");
        links("www. x", "www. x");
        links("http://a_b.c x", "http://a_b.c x");
        links("www.a_b.com", "www.a_b.com");
        links("www.a_b.c.com", "[www.a_b.c.com](http://www.a_b.c.com)");
        links("http:// a.b and www.", "http:// a.b and www.");
    }
}
