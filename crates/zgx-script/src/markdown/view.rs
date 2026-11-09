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
    /// The top-level block being built, for the veil's places.
    pub(crate) veil_at: Option<usize>,
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
        Some(Fading(self.veil_state.clone()?, (self.veil_at?, n)))
    }
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
}

/// What draws a markdown view's prose where the script gave a look
/// (`ui.markdown_view { style = … }`): made where scripts' elements are
/// lowered, so each option resolves as an element's.
pub(crate) trait Look {
    /// A run as a span.
    fn span(&self, run: &Run) -> zgx::compose::TextSpan;
    /// A paragraph (its runs' rich text) with the style's own options.
    fn paragraph(&self, view: View) -> View;
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
    span_of(run, None, bold, look)
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

/// A paragraph's runs as spans; with a veil, each run cut where a fading
/// chunk starts and ends, its pieces fading in.
fn spans(
    runs: &[Value],
    bold: bool,
    look: Option<&dyn Look>,
    fading: Option<&Fading>,
) -> Vec<zgx::compose::TextSpan> {
    let Some(Fading(veil, at)) = fading else {
        return runs.iter().map(|r| span(r, bold, look)).collect();
    };
    // The paragraph's words, a picture among them a byte no word holds.
    let texts: Vec<Rc<str>> = runs
        .iter()
        .map(|r| match r.field("img") {
            Some(_) => Rc::from("\0"),
            None => run_text(r),
        })
        .collect();
    let chunks: Vec<Chunk> =
        veil.borrow_mut()
            .chunks(*at, &texts.concat(), zgx::compose::fade_now());
    let mut out = Vec::with_capacity(runs.len());
    let mut pos = 0;
    for (r, text) in runs.iter().zip(&texts) {
        let (from, to) = (pos, pos + text.len());
        pos = to;
        if chunks.is_empty() || r.field("img").is_some() {
            out.push(span(r, bold, look));
            continue;
        }
        for (s, e, chunk) in super::veil::pieces(&chunks, from, to) {
            let piece = span_of(r, Some(&text[s - from..e - from]), bold, look);
            out.push(match chunk {
                Some(c) => piece.fade_in(c.fade()),
                None => piece,
            });
        }
    }
    out
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
        .map(|l| spans(l.list(), bold, look, fading))
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
            let fading = options.fading(1);
            let runs: View = faded_runs(
                b.field("s"),
                false,
                options.look.as_deref(),
                fading.as_ref(),
            )
            .into();
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
                    .map(|(i, b)| place(i, i == last, b))
                    .collect()
            })
        },
        move |place: Place, cx: &mut Context| {
            let (index, live) = match place {
                Place::Kept(i, ..) => (i as usize, true),
                Place::Whole(i, _) => (i, false),
            };
            let b = reading.with_untracked(|l| l.list().get(index).cloned().unwrap_or(Value::Nil));
            // (The veil's places count from the top-level blocks.)
            let mut options = options.clone();
            options.veil_at = top.then_some(index);
            let options = &options;
            if !live {
                if let Some(slot) = building.borrow_mut().get_mut(index) {
                    *slot = None;
                }
                return static_block(&b, options, cx);
            }
            let cell = cx.runtime().signal(b);
            if let Some(slot) = building.borrow_mut().get_mut(index) {
                *slot = Some(cell.clone());
            }
            block(cell, options, cx)
        },
    )
    .w_full()
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
    zgx::compose::rich_text_signal(move || {
        cell.with(|b| {
            b.field(field)
                .map(|runs| spans(runs.list(), bold, look.as_deref(), fading.as_ref()))
                .unwrap_or_default()
        })
    })
    .text_wrap(true)
}

fn block(cell: Signal<Value>, options: &ViewOptions, cx: &mut Context) -> View {
    let b = cell.with_untracked(Clone::clone);
    let kind = text_of(b.field("t"));
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
            let runs = faded_runs_of(cell, "s", false, options.look.clone(), options.fading(1));
            match &options.look {
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
