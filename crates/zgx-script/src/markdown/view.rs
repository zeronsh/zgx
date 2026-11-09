//! `ui.markdown_view { text }`: markdown drawn, the way a chat or a notes
//! app shows it, without the script drawing its blocks. The text is parsed
//! as it changes ([`Incremental`]: a reply streaming in parses its tail),
//! and only the blocks whose value changed are built again; the rest keep
//! their nodes, layout and paint. Paragraphs and headings are runs of
//! styled text, lists and quotes hold blocks, code blocks show their
//! language and a Copy button, tables lay out their cells in rows. Colours
//! and sizes are the theme's tokens.
use super::{
    Incremental, Options, PENDING_LINK_URL,
    veil::{Chunk, Veil},
};
use crate::value::{Value, identity};
use std::{cell::RefCell, rc::Rc, time::Duration};
use zgx::{
    compose::{Context, View, button, column, div, keyed, prelude::*, row, text},
    reactive::Signal,
    semantics::Role,
    text_layout::FontFamily,
    theme::{Themed, token},
};

/// How a markdown view draws.
#[derive(Clone)]
pub(crate) struct ViewOptions {
    pub parse: Options,
    /// Code blocks get a Copy button.
    pub copy: bool,
    /// The script's look for its prose (`style`, `on_link`, `link_menu`),
    /// if it gave one: its runs, its paragraphs and the room between its
    /// blocks drawn by it, not by the defaults here.
    pub look: Option<Rc<dyn Look>>,
    /// `veil = { key, seed }`: words arriving fade in (`super::veil`).
    pub veil: Option<VeilOptions>,
    /// The veil kept by its key, once the view is made.
    pub(crate) veil_state: Option<Rc<RefCell<Veil>>>,
    /// The top-level block being built: the veil's places and a find's
    /// numbering count from it.
    pub(crate) block_at: Option<usize>,
    /// `attached`: the names of what the text carries; words naming one
    /// (`(attachment: <name>)`) are a run of their own the look may draw
    /// as its chip.
    pub attached: Option<Rc<std::collections::HashSet<String>>>,
    /// The top-level block being built, as it is now (one being written,
    /// followed), and the place in it of the block being built (`order`):
    /// where a part nested in it stands among its parts.
    pub(crate) top: Option<Signal<Value>>,
    pub(crate) path: Rc<[usize]>,
    /// In a quote: its words in the quote's ink.
    pub(crate) quoted: bool,
}

/// `veil = { key = …, seed = … }`: the reply's key, which keeps what each
/// paragraph has shown, and whether what is there as the view is made
/// shows at once.
#[derive(Clone)]
pub(crate) struct VeilOptions {
    pub key: Rc<str>,
    pub seed: bool,
}

/// A paragraph's place in the veil, if it fades in: the veil, and the
/// paragraph's top-level block and its place in it.
#[derive(Clone)]
struct Fading(Rc<RefCell<Veil>>, (usize, u32));
impl ViewOptions {
    /// The veil's place for the top-level block's `n`th paragraph.
    fn fading(&self, n: u32) -> Option<Fading> {
        Some(Fading(self.veil_state.clone()?, (self.block_at?, n)))
    }
    /// How a paragraph of the block being built is drawn: its veil, what
    /// the text carries, and a find's marks, by its place in its top-level
    /// block (the top-level one itself is its block's first).
    fn para(&self) -> Para {
        let nested = !self.path.is_empty() && self.top.is_some();
        Para {
            fading: if nested { None } else { self.fading(1) },
            attached: self.attached.clone(),
            find_at: self.block_at.filter(|_| self.look.is_some()),
            nested: nested.then(|| {
                (
                    self.veil_state.clone().zip(self.block_at),
                    self.top.clone().expect("nested in one"),
                    self.path.clone(),
                )
            }),
        }
    }
}

/// How a paragraph is drawn beyond its runs: fading in (its place in the
/// veil), the names of what the text carries, and the top-level block a
/// find counts its matches in.
/// A nested paragraph's: the veil and its top-level block's place, the
/// block as it is now, and the paragraph's place in it.
type Nested = (
    Option<(Rc<RefCell<Veil>>, usize)>,
    Signal<Value>,
    Rc<[usize]>,
);
#[derive(Clone, Default)]
struct Para {
    fading: Option<Fading>,
    attached: Option<Rc<std::collections::HashSet<String>>>,
    find_at: Option<usize>,
    /// Nested in a top-level block: the veil and the block's place, the
    /// block as it is now, and the paragraph's place in it, which say its
    /// place in the veil and its matches' numbers as it is drawn.
    nested: Option<Nested>,
}

/// A run of a paragraph, as a look is asked to draw it: its text, its
/// kinds, and where it links (none for a link still streaming its
/// address).
pub(crate) struct Run<'a> {
    pub text: &'a str,
    pub strong: bool,
    pub em: bool,
    pub strike: bool,
    pub code: bool,
    /// A link's: its target, or `Some("")` while its address streams.
    pub link: Option<&'a str>,
    /// Words naming an item the text carries (`(attachment: <name>)`, where
    /// the view's `attached` holds the name): the name.
    pub attached: Option<&'a str>,
    /// A find's match it is (1), the current one (2), or none (0).
    pub mark: u8,
}

/// What draws a markdown view's prose where the script gave a look
/// (`ui.markdown_view { style = … }`): made where scripts' elements are
/// lowered, so each option resolves as an element's.
pub(crate) trait Look {
    /// A run as a span.
    fn span(&self, run: &Run) -> zgx::compose::TextSpan;
    /// A run the look may draw as a chip (a link elsewhere than the web, an
    /// item carried, named), as one, whole: a chip is never cut by a fade.
    /// None for a run drawn as words.
    fn chip(&self, run: &Run) -> Option<zgx::compose::TextSpan>;
    /// A find over the text, if the view has one: its query as typed and
    /// the current match's place among top-level block `block`'s (from 1;
    /// 0 when it is elsewhere). Tracked. None with no query.
    fn find(&self, block: usize) -> Option<(Rc<str>, usize)>;
    /// Whether the view has a find.
    fn finds(&self) -> bool;
    /// The top-level block holding the current match, laid out: brought
    /// into view as the find says.
    fn reveal(&self, block: zgx::compose::NodeHandle);
    /// A paragraph (its runs' rich text) with the style's own options.
    fn paragraph(&self, view: View) -> View;
    /// Whether the style names part `name` (`h1`, `list`, `quote`, `rule`…).
    fn has(&self, name: &str) -> bool;
    /// `view` with the style's options for part `name` (none if it names
    /// none).
    fn part(&self, name: &str, view: View) -> View;
    /// The top-level blocks' list with the style's room between them (the
    /// view's own where it gives none).
    fn blocks(&self, view: View) -> View;
}

fn themed<T>(name: &str) -> Themed<T> {
    Themed::Token(token(name))
}

fn text_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::Str(s)) => s.to_string(),
        Some(Value::Nil) | None => String::new(),
        Some(other) => other.to_string(),
    }
}
fn flag(run: &Value, name: &str) -> bool {
    matches!(run.field(name), Some(Value::Bool(true)))
}

/// A run's fields (`markdown::runs_value`): its text, its flags and its
/// link.
#[derive(Default)]
struct RunFields<'a> {
    text: Option<&'a Value>,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: Option<&'a Value>,
}
impl<'a> RunFields<'a> {
    /// Read in one pass over its fields, a run being a small map: not a
    /// search for each of six names, a document's every run.
    fn of(run: &'a Value) -> Self {
        let Value::Map(fields) = run else {
            return Self {
                text: run.field("x"),
                bold: flag(run, "b"),
                italic: flag(run, "i"),
                strike: flag(run, "s"),
                code: flag(run, "c"),
                link: run.field("a"),
            };
        };
        let mut out = Self::default();
        for (key, value) in fields.iter() {
            let on = matches!(value, Value::Bool(true));
            match &**key {
                "x" => out.text = Some(value),
                "b" => out.bold = on,
                "i" => out.italic = on,
                "s" => out.strike = on,
                "c" => out.code = on,
                "a" => out.link = Some(value),
                _ => {}
            }
        }
        out
    }
}

/// A run of text as a span: bold, italic, struck, code or a link; drawn
/// by the look, if there is one.
fn span(run: &Value, bold: bool, look: Option<&dyn Look>) -> zgx::compose::TextSpan {
    span_of(run, None, bold, look, 0)
}
/// A run's text: its words (a picture's alt).
fn run_text(run: &Value) -> Rc<str> {
    match run.field("x") {
        Some(Value::Str(s)) => s.clone(),
        other => text_of(other).into(),
    }
}
/// `span`, of `text` (a piece of the run's) where given.
fn span_of(
    run: &Value,
    text: Option<&str>,
    bold: bool,
    look: Option<&dyn Look>,
    mark: u8,
) -> zgx::compose::TextSpan {
    let fields = RunFields::of(run);
    if let Some(look) = look {
        let text: Rc<str> = match (text, fields.text) {
            (Some(text), _) => text.into(),
            (None, Some(Value::Str(s))) => s.clone(),
            (None, other) => text_of(other).into(),
        };
        let link = match fields.link {
            Some(Value::Str(href)) if &**href == PENDING_LINK_URL => Some(""),
            Some(Value::Str(href)) => Some(&**href),
            _ => None,
        };
        return look.span(&Run {
            text: &text,
            strong: bold || fields.bold,
            em: fields.italic,
            strike: fields.strike,
            code: fields.code,
            link,
            attached: None,
            mark,
        });
    }
    let mut out = zgx::compose::text_span(match text {
        Some(text) => text.to_owned(),
        None => text_of(fields.text),
    });
    if bold || fields.bold {
        out = out.font_bold();
    }
    if fields.italic {
        out = out.italic(true);
    }
    if fields.strike {
        out = out.strikethrough(Default::default());
    }
    if fields.code {
        out = out
            .font_family(FontFamily::Monospace)
            .background(themed("surface.sunken"))
            .padding(3., 0.5)
            .radius(3.);
    }
    if let Some(Value::Str(href)) = fields.link {
        out = out.text_color(themed("accent"));
        // Still streaming its address: styled as a link, going nowhere.
        if &**href != PENDING_LINK_URL {
            out = out.href(href.to_string());
        }
    }
    out
}

/// A run, or a piece of one: its text, and the item it names, if any.
struct Piece<'a> {
    run: &'a Value,
    text: Rc<str>,
    attached: Option<Rc<str>>,
}
const ATTACHED: &str = "(attachment: ";
/// `runs`, with the words naming an item carried (`(attachment: <name>)`,
/// `attached` holding the name) pieces of their own, keeping their run's
/// look; words outside links, code and pictures alone.
fn pieces<'a>(
    runs: &'a [Value],
    attached: Option<&std::collections::HashSet<String>>,
) -> Vec<Piece<'a>> {
    let mut out = Vec::with_capacity(runs.len());
    for run in runs {
        let text = match run.field("img") {
            // (A picture among the words: a byte no word holds.)
            Some(_) => Rc::from("\0"),
            None => run_text(run),
        };
        let plain = || {
            !matches!(run.field("a"), Some(Value::Str(_)))
                && !matches!(run.field("c"), Some(Value::Bool(true)))
                && run.field("img").is_none()
        };
        let Some(names) = attached.filter(|_| text.contains(ATTACHED) && plain()) else {
            out.push(Piece {
                run,
                text,
                attached: None,
            });
            continue;
        };
        let (mut at, mut copied) = (0, 0);
        while let Some(found) = text[at..].find(ATTACHED) {
            let start = at + found;
            let name_at = start + ATTACHED.len();
            let name_len = text[name_at..]
                .find([')', '\n'])
                .unwrap_or(text.len() - name_at);
            let closed = text[name_at + name_len..].starts_with(')');
            if name_len == 0 || !closed {
                // (No name closed here: the next one along.)
                at = start + 1;
                continue;
            }
            let end = name_at + name_len + 1;
            let name = &text[name_at..name_at + name_len];
            if names.contains(name) {
                if start > copied {
                    out.push(Piece {
                        run,
                        text: text[copied..start].into(),
                        attached: None,
                    });
                }
                out.push(Piece {
                    run,
                    text: text[start..end].into(),
                    attached: Some(name.into()),
                });
                copied = end;
            }
            at = end;
        }
        if copied == 0 {
            out.push(Piece {
                run,
                text,
                attached: None,
            });
        } else if copied < text.len() {
            out.push(Piece {
                run,
                text: text[copied..].into(),
                attached: None,
            });
        }
    }
    out
}

/// Where `q` (lower case, not empty) falls in `words`, case aside (ASCII's,
/// as a script's `string.lower`): each match's bytes, not overlapping.
fn occurrences(words: &str, q: &str) -> Vec<(usize, usize)> {
    let hay = words.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = hay[at..].find(q) {
        out.push((at + i, at + i + q.len()));
        at += i + q.len();
    }
    out
}

/// A paragraph's runs as spans: those the look draws as chips whole, and
/// the rest cut where a find's match starts and ends (each washed), or,
/// with no match, where a fading chunk of the veil does, its pieces fading
/// in.
fn spans(
    runs: &[Value],
    bold: bool,
    look: Option<&dyn Look>,
    para: &Para,
) -> Vec<zgx::compose::TextSpan> {
    // A find's query and the current match's place in this block.
    let find = para
        .find_at
        .and_then(|at| look?.find(at))
        .map(|(q, k)| (q.to_ascii_lowercase(), k));
    // Nested: its place in the veil and the words before it in its block,
    // from the block as it is (each change to it draws it again, as the
    // app's own drew the whole block again).
    let (fading, before) = match &para.nested {
        Some((veil, top, path)) => {
            let (n, before) = top.with(|t| super::order::before(t, path));
            let fading = veil.clone().map(|(v, at)| Fading(v, (at, n + 1)));
            (fading, before)
        }
        None => (para.fading.clone(), Vec::new()),
    };
    let fading = fading.as_ref();
    if fading.is_none() && para.attached.is_none() && find.is_none() {
        return runs
            .iter()
            .map(
                |r| match look.and_then(|look| chip_of(r, None, bold, look, 0)) {
                    Some(chip) => chip,
                    None => span(r, bold, look),
                },
            )
            .collect();
    }
    let pieces = pieces(runs, para.attached.as_deref());
    let words: String = pieces.iter().map(|p| &*p.text).collect();
    let chunks: Vec<Chunk> = match fading {
        Some(Fading(veil, at)) => veil
            .borrow_mut()
            .chunks(*at, &words, zgx::compose::fade_now()),
        None => Vec::new(),
    };
    // (A paragraph's matches are numbered on from those before it in its
    // top-level block.)
    let (marks, current, base) = match &find {
        Some((q, k)) => (
            occurrences(&words, q),
            *k,
            before
                .iter()
                .map(|w| occurrences(w, q).len())
                .sum::<usize>(),
        ),
        None => (Vec::new(), 0, 0),
    };
    let mark_of = |i: usize| if base + i + 1 == current { 2 } else { 1 };
    let mut out = Vec::with_capacity(pieces.len());
    let mut pos = 0;
    for p in &pieces {
        let (from, to) = (pos, pos + p.text.len());
        pos = to;
        if p.run.field("img").is_some() {
            out.push(span_of(p.run, Some(&p.text), bold, look, 0));
            continue;
        }
        // A chip is one piece, one wash: never cut by a fade or a find; a
        // match in it washes the whole chip.
        let washed = (marks.iter().enumerate())
            .rfind(|(_, m)| m.0 < to && m.1 > from)
            .map_or(0, |(i, _)| mark_of(i));
        if let Some(chip) = look.and_then(|look| chip_of(p.run, Some(p), bold, look, washed)) {
            out.push(chip);
            continue;
        }
        if !marks.is_empty() {
            let mut at = from;
            for (i, m) in marks.iter().enumerate() {
                let (s, e) = (m.0.max(from), m.1.min(to));
                if s < e {
                    if s > at {
                        out.push(span_of(
                            p.run,
                            Some(&p.text[at - from..s - from]),
                            bold,
                            look,
                            0,
                        ));
                    }
                    out.push(span_of(
                        p.run,
                        Some(&p.text[s - from..e - from]),
                        bold,
                        look,
                        mark_of(i),
                    ));
                    at = e;
                }
            }
            if at < to {
                out.push(span_of(p.run, Some(&p.text[at - from..]), bold, look, 0));
            }
            continue;
        }
        if chunks.is_empty() {
            out.push(span_of(p.run, Some(&p.text), bold, look, 0));
            continue;
        }
        for (s, e, chunk) in super::veil::pieces(&chunks, from, to) {
            let mut piece = span_of(p.run, Some(&p.text[s - from..e - from]), bold, look, 0);
            // Each piece after its run's first goes on the run's span: a chip
            // cut is padded and washed as the chip whole, a link underlined as
            // one. (Never pieces of two runs, however alike.)
            if s > from {
                piece = piece.continues();
            }
            out.push(match chunk {
                Some(c) => piece.fade_in(c.fade()),
                None => piece,
            });
        }
    }
    out
}

/// The run (or piece) as the look's chip, if it draws one: only a link
/// elsewhere than the web, or words naming an item carried, is asked.
fn chip_of(
    run: &Value,
    piece: Option<&Piece>,
    bold: bool,
    look: &dyn Look,
    mark: u8,
) -> Option<zgx::compose::TextSpan> {
    let attached = piece.and_then(|p| p.attached.as_deref());
    let link = match run.field("a") {
        Some(Value::Str(href)) if &**href != PENDING_LINK_URL && !super::web_address(href) => {
            Some(&**href)
        }
        _ => None,
    };
    if attached.is_none() && link.is_none() {
        return None;
    }
    let fields = RunFields::of(run);
    let text = match piece {
        Some(p) => p.text.clone(),
        None => run_text(run),
    };
    look.chip(&Run {
        text: &text,
        strong: bold || fields.bold,
        em: fields.italic,
        strike: fields.strike,
        code: fields.code,
        link: if attached.is_some() { None } else { link },
        attached,
        mark,
    })
}

/// A settled block (one before the last: a parse keeps it as it is),
/// built as it is, without following anything: a paragraph of runs.
fn static_runs(
    list: Option<&Value>,
    bold: bool,
    look: Option<&dyn Look>,
) -> zgx::compose::RichTextView {
    faded_runs(list, bold, look, None)
}
/// `static_runs`, fading in where the veil says.
fn faded_runs(
    list: Option<&Value>,
    bold: bool,
    look: Option<&dyn Look>,
    fading: Option<&Fading>,
) -> zgx::compose::RichTextView {
    let spans = list
        .map(|l| {
            let para = Para {
                fading: fading.cloned(),
                ..Para::default()
            };
            spans(l.list(), bold, look, &para)
        })
        .unwrap_or_default();
    zgx::compose::rich_text().children(spans).text_wrap(true)
}

fn static_blocks(list: Option<&Value>, options: &ViewOptions, cx: &mut Context) -> Vec<View> {
    list.map(|l| {
        l.list()
            .iter()
            .map(|b| static_block(b, options, cx))
            .collect()
    })
    .unwrap_or_default()
}

/// A code block: its language, a Copy button, and its text unwrapped,
/// scrolling sideways when long.
fn static_code(b: &Value, options: &ViewOptions, cx: &mut Context) -> View {
    let lang = text_of(b.field("lang"));
    let body = text_of(b.field("text"));
    let mut header = row()
        .w_full()
        .px(12.)
        .py(4.)
        .items_center()
        .selectable(false)
        .child(
            text(if lang.is_empty() {
                "text".into()
            } else {
                lang.clone()
            })
            .text_color(themed("text.tertiary"))
            .text_size(themed("font.size.sm"))
            .grow(),
        );
    if options.copy {
        let label: Signal<String> = cx.runtime().signal("Copy".to_string());
        let platform = cx.try_service::<zgx::platform::PlatformService>();
        let tasks = cx.tasks();
        let (shown, copied, body) = (label.clone(), label, body.clone());
        header = header.child(
            button()
                .id("copy")
                .px(8.)
                .py(3.)
                .bg(Themed::Value(zgx::style::rgba(0x00000000)))
                .on_click(move || {
                    if let Some(platform) = &platform {
                        platform.0.set_clipboard_text(body.clone());
                    }
                    copied.set("Copied".into());
                    let again = copied.clone();
                    tasks.spawn(async move {
                        zgx::timer::sleep(Duration::from_millis(1500)).await;
                        again.set("Copy".into());
                    });
                })
                .child(
                    zgx::compose::text_signal(move || shown.get())
                        .text_size(themed("font.size.sm")),
                ),
        );
    }
    column()
        .w_full()
        .role(Role::Group)
        .accessible_label(if lang.is_empty() {
            "Code".to_string()
        } else {
            format!("{lang} code")
        })
        .bg(themed("surface.sunken"))
        .rounded(8.)
        .border(1.)
        .border_color(themed("border.subtle"))
        .child(header)
        .child(
            zgx::compose::scroll_x(cx.state(0_f32))
                .w_full()
                .px(12.)
                .pb(10.)
                .child(
                    text(body)
                        .font_family(FontFamily::Monospace)
                        .text_size(13.)
                        .shrink_0(),
                ),
        )
}

fn static_block(b: &Value, options: &ViewOptions, cx: &mut Context) -> View {
    let kind = text_of(b.field("t"));
    match kind.as_str() {
        "h" => {
            let level = b.field("l").and_then(Value::as_f64).unwrap_or(1.) as usize;
            let size = match level {
                1 => "font.size.xl",
                2 => "font.size.lg",
                _ => "font.size.md",
            };
            View::from(
                static_runs(b.field("s"), true, options.look.as_deref())
                    .text_size(themed(size))
                    .pt(4.),
            )
            .heading(level)
        }
        "code" => static_code(b, options, cx),
        "ul" | "ol" => {
            let start = b.field("start").and_then(Value::as_f64).unwrap_or(1.) as usize;
            let items = b
                .field("items")
                .map(|i| i.list().to_vec())
                .unwrap_or_default();
            let mut list = column().role(Role::List).gap(4.);
            for (i, item) in items.iter().enumerate() {
                let marker = if kind == "ol" {
                    format!("{}.", start + i)
                } else {
                    "•".into()
                };
                list = list.child(
                    row()
                        .role(Role::ListItem)
                        .gap(8.)
                        .child(
                            text(marker)
                                .text_color(themed("text.tertiary"))
                                .accessible_hidden()
                                .shrink_0(),
                        )
                        .child(column().grow().min_w(0.).gap(4.).children(static_blocks(
                            Some(item),
                            options,
                            cx,
                        ))),
                );
            }
            list
        }
        "quote" => column()
            .pl(12.)
            .gap(6.)
            .border_edges(zgx::scene::Insets {
                left: 3.,
                ..Default::default()
            })
            .border_color(themed("border"))
            .text_color(themed("text.secondary"))
            .children(static_blocks(b.field("b"), options, cx)),
        "hr" => div().w_full().h(1.).my(4.).bg(themed("border.subtle")),
        "table" => {
            let line = |cells: Option<&Value>, bold: bool| {
                let mut out = row().gap(12.);
                for cell in cells.map(|c| c.list().to_vec()).unwrap_or_default() {
                    out = out.child(column().w(160.).child(static_runs(
                        Some(&cell),
                        bold,
                        options.look.as_deref(),
                    )));
                }
                out
            };
            let mut table = column().gap(4.).child(line(b.field("head"), true));
            for cells in b
                .field("rows")
                .map(|r| r.list().to_vec())
                .unwrap_or_default()
            {
                table = table.child(line(Some(&cells), false));
            }
            table
        }
        "img" => text(text_of(b.field("alt"))).text_color(themed("text.secondary")),
        _ => {
            // In a look, made by a function as a live one is: what its runs
            // read (a chip's answer, an ink following a setting) makes them
            // again, the paragraph kept.
            let runs: View = match &options.look {
                Some(_) => para_runs(
                    cx.runtime().signal(b.clone()),
                    "s",
                    false,
                    options.look.clone(),
                    options.para(),
                ),
                None => faded_runs(b.field("s"), false, None, None).into(),
            };
            match &options.look {
                Some(look) => look.paragraph(runs),
                None => runs,
            }
        }
    }
}

/// A block's place in the view, which keeps its build. The last block
/// (the one a reply is still writing) is built once for its place and
/// kind (a paragraph, a heading of a level, a code block, a list), its
/// contents following its value as the text grows. Those before it are
/// settled (a parse keeps them as they were): built as they are, again
/// only if their value changes (the last becoming settled, once), as a
/// table always is.
/// `Kept`: its place, kind (`kind_id`), and a heading's level or an
/// ordered list's start (0 for others). Plain numbers: a key holds no
/// part of the block it was found for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Place {
    Kept(u32, u8, u32),
    Whole(usize, usize),
}
/// The kinds of block the parser makes (`t`), by number; `None` for any
/// other, built whole.
fn kind_id(kind: &str) -> Option<u8> {
    Some(match kind {
        "p" => 1,
        "h" => 2,
        "quote" => 3,
        "code" => 4,
        "ul" => 5,
        "ol" => 6,
        "img" => 7,
        "hr" => 8,
        _ => return None,
    })
}
fn place(index: usize, last: bool, b: &Value) -> Place {
    let kind = match b.field("t") {
        Some(Value::Str(kind)) if last => kind_id(kind),
        _ => None,
    };
    let Some(kind) = kind else {
        return Place::Whole(index, identity(b));
    };
    let number = |field: &str| b.field(field).and_then(Value::as_f64).unwrap_or(1.) as u32;
    let n = match kind {
        2 => number("l"),
        6 => number("start"),
        _ => 0,
    };
    Place::Kept(index as u32, kind, n)
}

/// Write `value` into `cell` if it is another value (blocks a parse kept
/// are the same tables): what reads the cell follows.
fn put(cell: &Signal<Value>, value: &Value) {
    let same = cell.with_untracked(|was| {
        let at = identity(was);
        (at != 0 && at == identity(value)) || was == value
    });
    if !same {
        cell.mutate(|was| {
            *was = value.clone();
            true
        });
    }
}

/// The blocks `list` holds (a list of blocks), each built once for its
/// place and following its value.
fn block_list(list: Signal<Value>, options: &ViewOptions, top: bool) -> View {
    let cells: Rc<RefCell<Vec<Option<Signal<Value>>>>> = Rc::default();
    let (keeping, building, reading, options) =
        (cells.clone(), cells, list.clone(), options.clone());
    // In a look, every prose block follows its value in place (as the app
    // drew a block again only when its value changed): none is built again
    // as the one after it comes.
    let styled = options.look.is_some();
    keyed(
        move || {
            list.with(|blocks| {
                let blocks = blocks.list();
                let mut cells = keeping.borrow_mut();
                cells.resize(blocks.len(), None);
                for (cell, b) in cells.iter().zip(blocks) {
                    if let Some(cell) = cell {
                        put(cell, b);
                    }
                }
                let last = blocks.len().saturating_sub(1);
                blocks
                    .iter()
                    .enumerate()
                    .map(|(i, b)| place(i, i == last || styled, b))
                    .collect()
            })
        },
        move |place: Place, cx: &mut Context| {
            let (index, live) = match place {
                Place::Kept(i, ..) => (i as usize, true),
                Place::Whole(i, _) => (i, false),
            };
            let b = reading.with_untracked(|l| l.list().get(index).cloned().unwrap_or(Value::Nil));
            let heading = matches!(b.field("t"), Some(Value::Str(t)) if &**t == "h");
            // (The veil's places and a find's numbers count from the
            // top-level blocks.)
            let mut options = options.clone();
            if top {
                options.block_at = Some(index);
                options.path = Rc::from([]);
            } else {
                options.path = super::order::on(&options.path, &[index]);
            }
            let options = &mut options;
            let (view, cell) = if !live {
                if let Some(slot) = building.borrow_mut().get_mut(index) {
                    *slot = None;
                }
                (static_block(&b, options, cx), None)
            } else {
                let cell = cx.runtime().signal(b);
                if let Some(slot) = building.borrow_mut().get_mut(index) {
                    *slot = Some(cell.clone());
                }
                if top {
                    options.top = Some(cell.clone());
                }
                (block(cell.clone(), options, cx), Some(cell))
            };
            match &options.look {
                Some(look) if top && look.finds() => {
                    // Its box, as the app's: a block a find marks sits in a
                    // box of its own (a heading pulled toward what it heads,
                    // `pull`), which is what is revealed, holding it as the
                    // app's swap of a block as its marks change holds it (a
                    // one-cell grid, its content stretched).
                    let inner = column()
                        .grid()
                        .grid_cols(1)
                        .grid_rows(1)
                        .items_stretch()
                        .w_full()
                        .min_w(0.)
                        .child(column().contents().child(view));
                    let mut outer = div().w_full().min_w(0.).child(inner);
                    if heading {
                        outer = look.part("pull", outer);
                    }
                    revealed(outer, index, look.clone(), cell, cx)
                }
                _ => view,
            }
        },
    )
    .w_full()
}

/// Top-level block `index`'s `view`, brought into view (`Look::reveal`) a
/// tick after it is laid out holding a find's current match: as the match
/// comes to it, the query changes while it has it, or (being written,
/// `cell`) it grows.
fn revealed(
    view: View,
    index: usize,
    look: Rc<dyn Look>,
    cell: Option<Signal<Value>>,
    cx: &mut Context,
) -> View {
    let handle: Rc<RefCell<Option<zgx::compose::NodeHandle>>> = Rc::default();
    let (mounted, held, tasks) = (handle.clone(), handle, cx.tasks());
    let mut last: Option<(Rc<str>, usize)> = None;
    cx.retain(cx.runtime().effect(move || {
        let now = look.find(index);
        let grew = cell.as_ref().is_some_and(|c| {
            c.with(|_| ());
            true
        });
        if now == last && !grew {
            return;
        }
        last = now.clone();
        if !now.is_some_and(|(_, k)| k > 0) {
            return;
        }
        let (look, held) = (look.clone(), held.clone());
        tasks.spawn(async move {
            zgx::timer::sleep(Duration::ZERO).await;
            if let Some(node) = held.borrow().clone() {
                look.reveal(node);
            }
        });
    }));
    view.on_mount(move |node| {
        *mounted.borrow_mut() = Some(node.clone());
    })
}

/// The one block of the list of blocks `list` holds, following it: an
/// item of a list that is a line of text, without a list of its own. (A
/// second block coming builds the item again, as a list.)
fn single(list: Signal<Value>, options: &ViewOptions, cx: &mut Context) -> View {
    let first = |l: &Value| l.list().first().cloned().unwrap_or(Value::Nil);
    let cell = cx.runtime().signal(list.with_untracked(first));
    let (from, into) = (list, cell.clone());
    cx.retain(cx.runtime().effect(move || {
        let value = from.with(first);
        put(&into, &value);
    }));
    block(cell, options, cx)
}

/// A field of the block `cell` holds, as a cell of its own following it
/// (a list's items, a quote's blocks).
fn field_cell(cell: &Signal<Value>, name: &'static str, cx: &mut Context) -> Signal<Value> {
    let inner = cx
        .runtime()
        .signal(cell.with_untracked(|b| b.field(name).cloned().unwrap_or(Value::Nil)));
    let (cell, out) = (cell.clone(), inner.clone());
    cx.retain(cx.runtime().effect(move || {
        let value = cell.with(|b| b.field(name).cloned().unwrap_or(Value::Nil));
        put(&out, &value);
    }));
    inner
}

/// A code block: its language, a Copy button, and its text unwrapped,
/// scrolling sideways when long.
fn code(cell: Signal<Value>, options: &ViewOptions, cx: &mut Context) -> View {
    let lang = {
        let cell = cell.clone();
        move || {
            let lang = cell.with(|b| text_of(b.field("lang")));
            if lang.is_empty() { "text".into() } else { lang }
        }
    };
    let mut header = row()
        .w_full()
        .px(12.)
        .py(4.)
        .items_center()
        .selectable(false)
        .child(
            zgx::compose::text_signal(lang.clone())
                .text_color(themed("text.tertiary"))
                .text_size(themed("font.size.sm"))
                .grow(),
        );
    if options.copy {
        let label: Signal<String> = cx.runtime().signal("Copy".to_string());
        let platform = cx.try_service::<zgx::platform::PlatformService>();
        let tasks = cx.tasks();
        let (shown, copied, body) = (label.clone(), label, cell.clone());
        header = header.child(
            button()
                .id("copy")
                .px(8.)
                .py(3.)
                .bg(Themed::Value(zgx::style::rgba(0x00000000)))
                .on_click(move || {
                    if let Some(platform) = &platform {
                        platform
                            .0
                            .set_clipboard_text(body.with_untracked(|b| text_of(b.field("text"))));
                    }
                    copied.set("Copied".into());
                    let again = copied.clone();
                    tasks.spawn(async move {
                        zgx::timer::sleep(Duration::from_millis(1500)).await;
                        again.set("Copy".into());
                    });
                })
                .child(
                    zgx::compose::text_signal(move || shown.get())
                        .text_size(themed("font.size.sm")),
                ),
        );
    }
    let body = cell.clone();
    column()
        .w_full()
        .role(Role::Group)
        .accessible_label_when(move || {
            let lang = lang();
            if lang == "text" {
                "Code".to_string()
            } else {
                format!("{lang} code")
            }
        })
        .bg(themed("surface.sunken"))
        .rounded(8.)
        .border(1.)
        .border_color(themed("border.subtle"))
        .child(header)
        .child(
            zgx::compose::scroll_x(cx.state(0_f32))
                .w_full()
                .px(12.)
                .pb(10.)
                .child(
                    zgx::compose::text_signal(move || body.with(|b| text_of(b.field("text"))))
                        .font_family(FontFamily::Monospace)
                        .text_size(13.)
                        .shrink_0(),
                ),
        )
}

/// A prose block in the look's own structure, where its style names the
/// kind (`h1`, `list`, `quote`, `rule`): each part the element the app
/// draws, with the style's options for it. None for a kind it doesn't
/// name (drawn as the view's own), and a paragraph (`block`'s).
fn look_block(
    kind: &str,
    b: &Value,
    cell: Signal<Value>,
    look: &Rc<dyn Look>,
    options: &ViewOptions,
    cx: &mut Context,
) -> Option<View> {
    match kind {
        "h" if look.has("h1") => {
            let level = b.field("l").and_then(Value::as_f64).unwrap_or(1.) as usize;
            let part = ["h1", "h2", "h3"][level.clamp(1, 3) - 1];
            let runs = para_runs(cell, "s", false, Some(look.clone()), options.para());
            let mut view = look.part(part, look.paragraph(runs));
            if options.quoted {
                view = look.part("quote_ink", view);
            }
            Some(view.heading(level))
        }
        "ul" | "ol" if look.has("list") => {
            Some(look_list(kind == "ol", b, cell, look, options, cx))
        }
        "quote" if look.has("quote") => {
            let mut inner = options.clone();
            inner.quoted = true;
            let body = block_list(field_cell(&cell, "b", cx), &inner, false);
            Some(
                look.part(
                    "quote",
                    row()
                        .child(look.part("quote_rail", div()))
                        .child(look.part("quote_body", body)),
                ),
            )
        }
        "hr" if look.has("rule") => Some(look.part("rule", div())),
        _ => None,
    }
}

/// A list in the look: each item a row of its marker (a dot, or its number)
/// hanging in its gutter and its blocks, built once for its place and
/// following its value.
fn look_list(
    ordered: bool,
    b: &Value,
    cell: Signal<Value>,
    look: &Rc<dyn Look>,
    options: &ViewOptions,
    cx: &mut Context,
) -> View {
    let start = b.field("start").and_then(Value::as_f64).unwrap_or(1.) as usize;
    let items = field_cell(&cell, "items", cx);
    let (options, look, outer) = (options.clone(), look.clone(), look.clone());
    let cells: Rc<RefCell<Vec<Option<Signal<Value>>>>> = Rc::default();
    let (keeping, building, reading) = (cells.clone(), cells, items.clone());
    let list = keyed(
        move || {
            items.with(|l| {
                let list = l.list();
                let mut cells = keeping.borrow_mut();
                cells.resize(list.len(), None);
                for (cell, item) in cells.iter().zip(list) {
                    if let Some(cell) = cell {
                        put(cell, item);
                    }
                }
                // By place and how many blocks: an item of one (the usual,
                // a line of text) draws its block alone.
                list.iter()
                    .enumerate()
                    .map(|(i, item)| (i, item.list().len().min(2)))
                    .collect::<Vec<(usize, usize)>>()
            })
        },
        move |(i, blocks): (usize, usize), cx: &mut Context| {
            let item = cx
                .runtime()
                .signal(reading.with_untracked(|l| l.list().get(i).cloned().unwrap_or(Value::Nil)));
            if let Some(slot) = building.borrow_mut().get_mut(i) {
                *slot = Some(item.clone());
            }
            let mut inner = options.clone();
            inner.path = super::order::on(&options.path, &[i]);
            let body = match blocks {
                1 => {
                    let mut first = inner.clone();
                    first.path = super::order::on(&inner.path, &[0]);
                    look.part("item_body", column().child(single(item, &first, cx)))
                }
                _ => look.part("item_body", block_list(item, &inner, false)),
            };
            let marker: View = if ordered {
                look.part("number", text(format!("{}.", start + i)))
            } else {
                look.part("bullet", row().child(look.part("bullet_dot", div())))
            };
            look.part(
                "item",
                row()
                    .role(Role::ListItem)
                    .child(marker.accessible_hidden())
                    .child(body),
            )
        },
    );
    outer.part("list", list.role(Role::List))
}

/// A paragraph of the runs field `field` of `cell` holds, following it.
fn runs_of(
    cell: Signal<Value>,
    field: &'static str,
    bold: bool,
    look: Option<Rc<dyn Look>>,
) -> View {
    faded_runs_of(cell, field, bold, look, None)
}
/// `runs_of`, fading in where the veil says.
fn faded_runs_of(
    cell: Signal<Value>,
    field: &'static str,
    bold: bool,
    look: Option<Rc<dyn Look>>,
    fading: Option<Fading>,
) -> View {
    let para = Para {
        fading,
        ..Para::default()
    };
    para_runs(cell, field, bold, look, para)
}
/// A paragraph of the runs field `field` of `cell` holds, drawn as `para`
/// says, following it (and what drawing it reads: a find, a chip's answer).
fn para_runs(
    cell: Signal<Value>,
    field: &'static str,
    bold: bool,
    look: Option<Rc<dyn Look>>,
    para: Para,
) -> View {
    // In a look, as wide as what holds it, as the app's own paragraph is
    // (stretched across its column): its box, not only its words.
    let stretched = look.is_some();
    let view = zgx::compose::rich_text_signal(move || {
        cell.with(|b| {
            b.field(field)
                .map(|runs| spans(runs.list(), bold, look.as_deref(), &para))
                .unwrap_or_default()
        })
    })
    .text_wrap(true);
    if stretched { view.w_full() } else { view }
}

fn block(cell: Signal<Value>, options: &ViewOptions, cx: &mut Context) -> View {
    let b = cell.with_untracked(Clone::clone);
    let kind = text_of(b.field("t"));
    if let Some(look) = options.look.clone()
        && let Some(view) = look_block(&kind, &b, cell.clone(), &look, options, cx)
    {
        return view;
    }
    match kind.as_str() {
        "h" => {
            let level = b.field("l").and_then(Value::as_f64).unwrap_or(1.) as usize;
            let size = match level {
                1 => "font.size.xl",
                2 => "font.size.lg",
                _ => "font.size.md",
            };
            runs_of(cell, "s", true, options.look.clone())
                .text_size(themed(size))
                .pt(4.)
                .heading(level)
        }
        "code" => code(cell, options, cx),
        "ul" | "ol" => {
            let start = b.field("start").and_then(Value::as_f64).unwrap_or(1.) as usize;
            let ordered = kind == "ol";
            let items = field_cell(&cell, "items", cx);
            let options = options.clone();
            // Each item a list of blocks, built once for its place, its
            // cell written as the items change (no effect an item).
            let cells: Rc<RefCell<Vec<Option<Signal<Value>>>>> = Rc::default();
            let (keeping, building, reading) = (cells.clone(), cells, items.clone());
            keyed(
                move || {
                    items.with(|l| {
                        let list = l.list();
                        let mut cells = keeping.borrow_mut();
                        cells.resize(list.len(), None);
                        for (cell, item) in cells.iter().zip(list) {
                            if let Some(cell) = cell {
                                put(cell, item);
                            }
                        }
                        // By place and how many blocks: an item of one
                        // (the usual, a line of text) draws its block alone.
                        list.iter()
                            .enumerate()
                            .map(|(i, item)| (i, item.list().len().min(2)))
                            .collect::<Vec<(usize, usize)>>()
                    })
                },
                move |(i, blocks): (usize, usize), cx: &mut Context| {
                    let item =
                        cx.runtime()
                            .signal(reading.with_untracked(|l| {
                                l.list().get(i).cloned().unwrap_or(Value::Nil)
                            }));
                    if let Some(slot) = building.borrow_mut().get_mut(i) {
                        *slot = Some(item.clone());
                    }
                    let content = match blocks {
                        1 => single(item, &options, cx),
                        _ => block_list(item, &options, false),
                    };
                    let marker = if ordered {
                        format!("{}.", start + i)
                    } else {
                        "•".into()
                    };
                    row()
                        .role(Role::ListItem)
                        .gap(8.)
                        .child(
                            text(marker)
                                .text_color(themed("text.tertiary"))
                                .accessible_hidden()
                                .shrink_0(),
                        )
                        .child(column().grow().min_w(0.).gap(4.).child(content))
                },
            )
            .role(Role::List)
            .gap(4.)
        }
        "quote" => column()
            .pl(12.)
            .gap(6.)
            .border_edges(zgx::scene::Insets {
                left: 3.,
                ..Default::default()
            })
            .border_color(themed("border"))
            .text_color(themed("text.secondary"))
            .child(block_list(field_cell(&cell, "b", cx), options, false)),
        "hr" => div().w_full().h(1.).my(4.).bg(themed("border.subtle")),
        "table" => {
            let line = |cells: Option<&Value>, bold: bool| {
                let mut out = row().gap(12.);
                for cell in cells.map(|c| c.list().to_vec()).unwrap_or_default() {
                    let runs = cell
                        .list()
                        .iter()
                        .map(|r| span(r, bold, options.look.as_deref()))
                        .collect::<Vec<_>>();
                    out = out.child(
                        column()
                            .w(160.)
                            .child(zgx::compose::rich_text().children(runs).text_wrap(true)),
                    );
                }
                out
            };
            let mut table = column().gap(4.).child(line(b.field("head"), true));
            for cells in b
                .field("rows")
                .map(|r| r.list().to_vec())
                .unwrap_or_default()
            {
                table = table.child(line(Some(&cells), false));
            }
            table
        }
        "img" => zgx::compose::text_signal(move || cell.with(|b| text_of(b.field("alt"))))
            .text_color(themed("text.secondary")),
        _ => {
            let runs = para_runs(cell, "s", false, options.look.clone(), options.para());
            match &options.look {
                // (In a quote, its ink over the paragraph's.)
                Some(look) if options.quoted => look.part("quote_ink", look.paragraph(runs)),
                Some(look) => look.paragraph(runs),
                None => runs,
            }
        }
    }
}

/// The view of `source()`'s markdown, following it: parsed again for what
/// changed (its tail, as text is appended), each block built once for its
/// place and kind, and only what a block's new value changes laid out and
/// painted again: a reply streaming in grows its last paragraph or code
/// block in place.
/// What a markdown view shows: text made for it, or a state's value as it
/// holds it (`"{reply}"`), which a long text's edits are followed in.
pub(crate) enum Source {
    Made(String),
    Held(Value),
}
pub(crate) fn view(mut source: impl FnMut() -> Source + 'static, options: ViewOptions) -> View {
    zgx::compose::component(move |cx| {
        let mut options = options.clone();
        if let Some(veil) = &options.veil {
            let state = super::veil::veil_of(&veil.key, veil.seed);
            // Seeding until the view is made: a tick on, as a script's
            // `ui.after(0)`.
            if state.borrow().seeding {
                let seeded = state.clone();
                cx.tasks().spawn(async move {
                    zgx::timer::sleep(Duration::ZERO).await;
                    seeded.borrow_mut().seeding = false;
                });
            }
            options.veil_state = Some(state);
        }
        let parser = RefCell::new(Incremental::new(options.parse.clone()));
        let top = cx.runtime().signal(Value::List(Vec::new().into()));
        let blocks = top.clone();
        cx.retain(cx.runtime().effect(move || {
            let mut parser = parser.borrow_mut();
            match source() {
                Source::Made(text) => parser.set_text(&text),
                Source::Held(value) => parser.set_value(&value),
            }
            let parsed = parser.blocks();
            blocks.mutate(|was| {
                *was = Value::List(parsed.into());
                true
            });
        }));
        let blocks = block_list(top, &options, true);
        match &options.look {
            Some(look) => look.blocks(blocks),
            None => blocks.gap(10.),
        }
    })
}
