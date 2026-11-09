//! A reply parsed as it streams shows what the same text parsed at once
//! shows, at every step: appended in pieces (`set_text`), followed through a
//! long text's edits (`set_value`, as `ui.markdown` given a state), and as
//! the grown list that form answers. `MD_SEEDS` runs more texts than CI's.
mod common;
use common::Harness;
use zgx::text::Text;
use zgx_script::Value;
use zgx_script::markdown::{Incremental, Options};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Bits of Markdown that change what they join, and what comes before them.
const BITS: &[&str] = &[
    "[REF]",
    "[Ref]: z\n",
    "[a  b]",
    "[a b]: q\n",
    "![ref]",
    "[t][ref]",
    "[ref][]",
    "[ref]: <u v> \"t\"\n",
    "[other]: o\n",
    "```\n[ref]: c\n```\n",
    "foo]: y\n",
    " <div\n",
    "  <p>x</p>\n",
    "- m![]()\n",
    "- ![](u)\n",
    "1. a ![b](c)\n",
    "> w ![](u)\n",
    "a\n![](u)\n",
    "# h\n",
    "## two\n\n",
    "para words\n",
    "words ",
    "\n",
    "\n\n",
    "\n\n\n",
    "- item\n",
    "  - nested\n",
    "    deep\n",
    "1. one\n",
    "2. two\n",
    "3",
    ".",
    "> quote\n",
    ">",
    "> ",
    "```\n",
    "```rust\n",
    "~~~\n",
    "code line\n",
    "| a | b |\n",
    "|---|---|\n",
    "| 1 | 2 |\n",
    "|",
    "**bold",
    "**",
    "*em*",
    "*",
    "_",
    "__",
    "`code`",
    "`",
    "``",
    "[link](http://x)",
    "[l](",
    "](",
    "[",
    "]",
    "![](u)",
    "![alt](u)\n",
    "![]()",
    "m![]()",
    "![a](b)![c](d)",
    "[![b](i)](u)",
    "~~s~~",
    "~~",
    "~",
    "---\n",
    "***\n",
    "===\n",
    "-",
    "+ ",
    "* ",
    "\\",
    "<div>\n",
    "</div>\n",
    "<b>",
    "&amp;",
    "&",
    "é",
    "日本語",
    "{b}",
    "$",
    "$$",
    "   ",
    "    indented\n",
    "\t",
    "- [ ] task\n",
    "- [x] done\n",
    "\r\n",
    "a\nb\n",
    "[ref]: http://y\n",
    "[ref]",
    "http://z.io ",
    "]: x\n",
    ">[ref]: y\n",
    "- [ref]: z\n",
    "1. [ref]: w\n",
    "  [ref]:",
    ":",
    "\"t\"\n",
];

/// A text: bits at random, or (one in four) a long run of paragraphs, so a
/// reply passes the grown list's chunks.
fn text(rng: &mut Rng) -> String {
    let mut out = String::new();
    if rng.below(4) == 0 {
        for i in 0..20 + rng.below(60) {
            out.push_str(&format!("Para {i} with **bold** and words.\n\n"));
            if rng.below(6) == 0 {
                out.push_str(BITS[rng.below(BITS.len())]);
            }
        }
        return out;
    }
    for _ in 0..1 + rng.below(24) {
        out.push_str(BITS[rng.below(BITS.len())]);
    }
    out
}

/// `text` cut at random, some bytes a piece, on character boundaries.
fn pieces(text: &str, rng: &mut Rng) -> Vec<String> {
    let most = [1, 3, 12, 60][rng.below(4)];
    let (mut out, mut rest) = (Vec::new(), text);
    while !rest.is_empty() {
        let at = rest.floor_char_boundary(1 + rng.below(most));
        let at = if at == 0 {
            rest.ceil_char_boundary(1)
        } else {
            at
        };
        out.push(rest[..at].to_string());
        rest = &rest[at..];
    }
    out
}

/// What the text parsed at once shows.
fn fresh(text: &str, options: &Options) -> Vec<Value> {
    let mut parser = Incremental::new(options.clone());
    parser.set_text(text);
    parser.blocks()
}

/// The first step at which `pieces` streamed differs from the text at once,
/// and how it differs.
fn disagreement(pieces: &[String], options: &Options) -> Option<String> {
    let (mut by_text, mut by_value, mut whole) = (
        Incremental::new(options.clone()),
        Incremental::new(options.clone()),
        String::new(),
    );
    let mut long = Text::new("");
    for (step, piece) in pieces.iter().enumerate() {
        whole.push_str(piece);
        long.append(piece);
        by_text.set_text(&whole);
        by_value.set_value(&Value::Text(long.clone()));
        let want = fresh(&whole, options);
        let grown = by_value.blocks_appended();
        for (form, got) in [
            ("set_text", by_text.blocks()),
            ("set_value", by_value.blocks()),
            ("grown list", grown.iter().cloned().collect()),
        ] {
            if got != want {
                let at = got.iter().zip(&want).take_while(|(a, b)| a == b).count();
                return Some(format!(
                    "step {step} of {}, {form}, block {at}:\n  streamed {:?}\n  at once  {:?}",
                    pieces.len(),
                    got.get(at),
                    want.get(at)
                ));
            }
        }
    }
    None
}

/// `pieces` made as short as it still disagrees: the example a failure shows.
fn smallest(mut pieces: Vec<String>, options: &Options) -> Vec<String> {
    loop {
        let before = pieces.clone();
        for i in (0..pieces.len()).rev() {
            let mut fewer = pieces.clone();
            fewer.remove(i);
            if disagreement(&fewer, options).is_some() {
                pieces = fewer;
            }
        }
        // Pieces joined, then their characters dropped.
        for i in (1..pieces.len()).rev() {
            let mut joined = pieces.clone();
            let last = joined.remove(i);
            joined[i - 1].push_str(&last);
            if disagreement(&joined, options).is_some() {
                pieces = joined;
            }
        }
        for i in 0..pieces.len() {
            let chars: Vec<char> = pieces[i].chars().collect();
            for drop in (0..chars.len()).rev() {
                if i >= pieces.len() {
                    break;
                }
                let mut less = pieces.clone();
                less[i] = chars
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != drop)
                    .map(|(_, c)| c)
                    .collect();
                if less[i].chars().count() + 1 == pieces[i].chars().count()
                    && disagreement(&less, options).is_some()
                {
                    pieces = less;
                    break;
                }
            }
        }
        if pieces == before {
            return pieces;
        }
    }
}

/// The repro: a picture after a word in a tight list item, a paragraph with
/// a line break before it.
#[test]
fn a_picture_after_a_word_in_a_list_item_stays_among_the_words() {
    let start = "é\n```\n```\n{b}\nd\n- m![]()\n\n_";
    let mut parser = Incremental::new(Options::default());
    parser.set_text(start);
    parser.set_text(&format!("{start}$"));
    let (got, want) = (
        parser.blocks(),
        fresh(&format!("{start}$"), &Options::default()),
    );
    assert_eq!(got, want);
    let list = want
        .iter()
        .find(|b| matches!(b.field("t"), Some(Value::Str(t)) if &**t == "ul"))
        .expect("the list");
    let first = list
        .field("items")
        .expect("its items")
        .item(0)
        .expect("its item")
        .clone();
    assert_eq!(
        first.list().len(),
        1,
        "one paragraph, the picture in it: {first:?}"
    );
}

/// A link definition reaches back over the whole text, wherever it is: in a
/// quote, in a list, with its label across two lines. Prose with brackets
/// isn't one, and a reply holding it stays parsed by its tail.
#[test]
fn a_definition_found_late_links_what_came_before() {
    for pieces in [
        &["[ref]\n\n-\n>[re", "f]:h"][..],
        &["[ref]\n\n- [re", "f]: h\n"],
        &["[ref]\n\n1. [ref]", ": h\n"],
        &["[a b]\n\n[a\n", "b]: h\n"],
    ] {
        let pieces: Vec<String> = pieces.iter().map(|p| p.to_string()).collect();
        assert_eq!(
            disagreement(&pieces, &Options::default()),
            None,
            "{pieces:?}"
        );
    }
    let mut parser = Incremental::new(Options::default());
    parser.set_text("First.\n\nSecond with [x]: y and d[k]: v.\n\nThird");
    parser.set_text("First.\n\nSecond with [x]: y and d[k]: v.\n\nThird more");
    assert!(
        parser.last_parse_bytes() < 50,
        "parsed {} bytes",
        parser.last_parse_bytes()
    );
}

/// What a definition costs: nothing for what only looks like one; nothing
/// beyond its tail for one with no reference ahead of it, whose later
/// references it links all the same; and for one with a reference ahead of
/// it, one whole parse.
#[test]
fn a_definition_parses_the_whole_text_only_for_a_reference_ahead_of_it() {
    let long: String = (0..60).map(|i| format!("Para {i}.\n\n")).collect();
    let tail_only = |parser: &Incremental| parser.last_parse_bytes() < 100;
    for looks_like_one in [
        "```\n[x]: y\n```\n\n",
        "foo]: y\n\n",
        "see [x]: y and d[k]: v.\n\n",
    ] {
        let mut parser = Incremental::new(Options::default());
        let mut text = format!("{long}{looks_like_one}Next");
        parser.set_text(&text);
        for piece in [" words", " more\n\nAfter", " this"] {
            text.push_str(piece);
            parser.set_text(&text);
            assert!(
                tail_only(&parser),
                "{looks_like_one:?}: {}",
                parser.last_parse_bytes()
            );
        }
    }
    // None ahead of it: its tail alone, as it comes and after, and a
    // reference after it links.
    let mut parser = Incremental::new(Options::default());
    let mut text = long.clone();
    parser.set_text(&text);
    for piece in [
        "[x]: ht",
        "tp://y\n\n",
        "Now [x] is a link",
        " still\n\n",
        "and more",
    ] {
        text.push_str(piece);
        parser.set_text(&text);
        assert!(
            tail_only(&parser),
            "{piece:?}: {}",
            parser.last_parse_bytes()
        );
        assert_eq!(
            parser.blocks(),
            fresh(&text, &Options::default()),
            "{piece:?}"
        );
    }
    // One ahead of it: whole as it lands, then the tail again.
    let mut parser = Incremental::new(Options::default());
    let mut text = format!("[x] named first.\n\n{long}");
    parser.set_text(&text);
    text.push_str("[x]: http://y\n\n");
    parser.set_text(&text);
    assert_eq!(parser.last_parse_bytes(), text.len(), "parsed whole");
    assert_eq!(parser.blocks(), fresh(&text, &Options::default()));
    for piece in ["More", " words\n\nAfter", " this"] {
        text.push_str(piece);
        parser.set_text(&text);
        assert!(
            tail_only(&parser),
            "{piece:?}: {}",
            parser.last_parse_bytes()
        );
    }
    // Those it parsed whole and found the same keep their values: all but
    // the one naming it.
    let mut whole = Incremental::new(Options::default());
    let mut text = format!("[x] named first.\n\n{long}");
    whole.set_text(&text);
    let then = whole.blocks();
    text.push_str("[x]: http://y\n\n");
    whole.set_text(&text);
    let at = |v: &Value| match v {
        Value::Map(fields) => fields.as_ptr() as usize,
        _ => 0,
    };
    let now = whole.blocks();
    let kept = now
        .iter()
        .zip(&then)
        .filter(|(a, b)| at(a) == at(b))
        .count();
    assert_eq!(kept, then.len() - 1, "all but the first");
    // Defined again to elsewhere: first wins, however it streamed.
    let pieces: Vec<String> = [
        "[x]: first\n\n",
        "words\n\n",
        "more\n\n",
        "again\n\n",
        "[x]: second\n\n",
        "[x] goes\n\n",
        "on",
    ]
    .iter()
    .map(|p| p.to_string())
    .collect();
    assert_eq!(disagreement(&pieces, &Options::default()), None);
}

#[test]
fn a_reply_streamed_shows_what_it_shows_at_once() {
    let seeds: u64 = std::env::var("MD_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    let start: u64 = std::env::var("MD_SEED_START")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let options = [
        Options::default(),
        Options {
            breaks: true,
            ..Options::default()
        },
    ];
    let mut found = Vec::new();
    for seed in start..start + seeds {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let text = text(&mut rng);
        let pieces = pieces(&text, &mut rng);
        let options = &options[(seed % 2) as usize];
        if disagreement(&pieces, options).is_some() {
            let small = smallest(pieces, options);
            found.push(format!(
                "seed {seed}: {small:?}\n{}",
                disagreement(&small, options).unwrap()
            ));
            if found.len() == 12 {
                break;
            }
        }
    }
    assert!(
        found.is_empty(),
        "{} disagreed:\n{}",
        found.len(),
        found.join("\n")
    );
}

/// A Luau string of `text`, every byte escaped.
fn lua_string(text: &str) -> String {
    let bytes: String = text.bytes().map(|b| format!("\\{b:03}")).collect();
    format!("\"{bytes}\"")
}

/// `pieces` streamed into a script: the first step whose tables differ from
/// a fresh key's, for `ui.markdown` given the state, and given the string
/// (whose table is derived from the last, past `CACHED_FROM` blocks).
fn script_disagreement(pieces: &[String]) -> Option<String> {
    let list: Vec<String> = pieces.iter().map(|p| lua_string(p)).collect();
    let script = format!(
        r###"local pieces = {{ {list} }}
local reply = ui.state("reply", "")
local step = ui.state("step", 0)
local function differ(a: any, b: any, path: string): string?
    if type(a) ~= type(b) then return path end
    if type(a) ~= "table" then return if a == b then nil else path end
    for k, v in a do
        local d = differ(v, b[k], path .. "." .. tostring(k))
        if d then return d end
    end
    for k in b do
        if a[k] == nil then return path .. "." .. tostring(k) end
    end
    return nil
end
local by_state = ui.markdown(reply, {{ key = "state" }})
local by_text = ui.computed(function() return ui.markdown(reply:get(), {{ key = "text" }}) end)
local first = ui.state("first", "")
ui.command {{ id = "append", run = function()
    local n = step:get() + 1
    step:set(n)
    reply:append(pieces[n])
    if first:get() ~= "" then return end
    local fresh = ui.markdown(reply:get(), {{ key = "fresh" .. n }})
    local d = differ(by_state:get(), fresh, "state") or differ(by_text:get(), fresh, "text")
    if d then first:set("step " .. n .. " " .. d) end
end }}
return ui.column {{ ui.text {{ id = "first", function() return first:get() end }}, ui.each(by_state, function(b: any) return ui.div {{}} end) }}
"###,
        list = list.join(", ")
    );
    let mut h = Harness::inline(&script);
    h.settle();
    for _ in pieces {
        assert!(h.host.run_command("append"));
        h.settle();
    }
    let first = h.text("first");
    (!first.is_empty()).then_some(first)
}

/// What a script reads agrees too, past the size its tables are cached from.
#[test]
fn a_script_reads_what_the_text_at_once_gives() {
    let seeds: u64 = std::env::var("MD_SCRIPT_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    // As it was first seen: a picture after a word, in an app.
    let repro = [
        "é\n```\n```\n{b}\nd\n- m![]()\n\n_".to_string(),
        "$".to_string(),
    ];
    assert_eq!(script_disagreement(&repro), None);
    let mut found = Vec::new();
    for seed in 0..seeds {
        let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
        // Past 16 blocks, and again past the next cached size.
        let mut text = String::new();
        for i in 0..14 + rng.below(30) {
            text.push_str(&format!("Para {i} with **bold**.\n\n"));
            if rng.below(5) == 0 {
                text.push_str(BITS[rng.below(BITS.len())]);
            }
        }
        let pieces = pieces(&text, &mut rng);
        if let Some(d) = script_disagreement(&pieces) {
            found.push(format!("seed {seed}: {d}: {text:?}"));
            if found.len() == 6 {
                break;
            }
        }
    }
    assert!(
        found.is_empty(),
        "{} disagreed:\n{}",
        found.len(),
        found.join("\n")
    );
}
