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
/// took was there).
pub(super) fn find(text: &str, edges: &[usize]) -> Vec<Found> {
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
            b'h' | b'H' if opens() => url(text, at),
            b'w' | b'W' if opens() => www(text, at),
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

/// `http://` or `https://` (any case) and a valid domain at `at`.
fn url(text: &str, at: usize) -> Option<Found> {
    let rest = &text[at..];
    let scheme = ["https://", "http://"].into_iter().find(|s| {
        rest.get(..s.len())
            .is_some_and(|r| r.eq_ignore_ascii_case(s))
    })?;
    let end = at + scheme.len() + extent(&rest[scheme.len()..])?;
    Some(Found {
        start: at,
        end,
        to: text[at..end].to_string(),
    })
}

/// `www.` (any case) and a valid domain at `at`: linked to `http://`.
fn www(text: &str, at: usize) -> Option<Found> {
    let rest = &text[at..];
    if !rest
        .get(..4)
        .is_some_and(|r| r.eq_ignore_ascii_case("www."))
    {
        return None;
    }
    let end = at + extent(rest)?;
    Some(Found {
        start: at,
        end,
        to: format!("http://{}", &text[at..end]),
    })
}

/// How far a URL from its domain (`text`'s start) reaches, trailing
/// punctuation, an unbalanced `)` and an entity left out; `None` if its
/// domain isn't valid.
fn extent(text: &str) -> Option<usize> {
    // To a space or a `<`.
    let mut end = text
        .find(|c: char| c.is_whitespace() || c == '<')
        .unwrap_or(text.len());
    loop {
        let link = &text[..end];
        let Some(last) = link.chars().next_back() else {
            break;
        };
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
    let domain_end = text[..end]
        .find(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .unwrap_or(end);
    valid_domain(&text[..domain_end]).then_some(end)
}

/// GFM's valid domain: segments of letters, digits, `_` and `-` between
/// periods, at least one period, and no `_` in the last two segments.
fn valid_domain(domain: &str) -> bool {
    let domain = domain.trim_end_matches('.');
    let segments: Vec<&str> = domain.split('.').collect();
    segments.len() >= 2
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
/// or ends, keeping their marks.
pub(super) fn link_runs(runs: &mut Vec<Run>) {
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
        let found = find(&text, &edges);
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
        for f in find(text, &[]) {
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
        // Its domain needs a period, and no `_` in its last two parts.
        links("https://localhost/x", "https://localhost/x");
        links("www.a_b.com", "www.a_b.com");
        links("www.a_b.c.com", "[www.a_b.c.com](http://www.a_b.c.com)");
        links("http:// a.b and www.", "http:// a.b and www.");
    }
}
