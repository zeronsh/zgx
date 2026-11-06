//! Rows' heights estimated from their data (`ui.list { estimate = … }`).
//!
//! The script declares once how a row's data makes its height: what's fixed,
//! which fields hold text, in what size and how wide a column. Rust runs that
//! per row, with no script: each row's text is scanned once into its lines'
//! widths, and a new width is arithmetic over those. Rows still measure as they
//! mount: this is what the rows out of view stand at until then.
use crate::value::Value;
use mlua::Table;
use rustc_hash::FxHashMap;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use zgx::compose::{RowsWatch, VariableHeights};

/// A text in a row: its size, and how wide its column is at a list's width.
#[derive(Clone, Debug, PartialEq)]
struct TextRule {
    field: Rc<str>,
    /// Its font's size, and a line's height.
    size: f32,
    line: f32,
    /// A narrow character's average width, in ems.
    char: f32,
    /// An average word's length in characters: wrapping leaves about half of
    /// one unused on each line.
    word: f32,
    /// The column: `share` of (the list less `inset`, at most `most`, less `minus`).
    inset: f32,
    most: f32,
    minus: f32,
    share: f32,
    /// Clamped to this many lines, and what shows beyond them then.
    most_lines: Option<f32>,
    beyond: f32,
    /// Its text is Markdown: what each kind of line adds, its source scanned.
    markdown: Option<Markdown>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Markdown {
    paragraph: f32,
    heading: f32,
    item: f32,
    code: f32,
    table: f32,
}
/// A field, a part for each of its values, and one for any other.
type ByField = (Rc<str>, Vec<(Rc<str>, Part)>, Option<Box<Part>>);
/// What a part of a row adds.
#[derive(Clone, Debug, Default, PartialEq)]
struct Part {
    height: f32,
    texts: Vec<usize>,
    /// For each item of a list field.
    each: Option<(Rc<str>, Box<Part>)>,
    /// By a field's value.
    by: Option<ByField>,
    /// If a field holds anything.
    has: Vec<(Rc<str>, Part)>,
}
/// A list's rule, read from its script's table once.
#[derive(Debug, PartialEq)]
pub(crate) struct Rule {
    root: Part,
    texts: Vec<TextRule>,
}
fn number(t: &Table, key: &str, default: f32) -> mlua::Result<f32> {
    let value = t.get::<Option<f32>>(key)?.unwrap_or(default);
    if !value.is_finite() || value < 0. {
        return Err(mlua::Error::runtime(format!(
            "ui.list `estimate`: `{key}` must be a number, 0 or more"
        )));
    }
    Ok(value)
}
fn name(t: &Table, key: &str) -> mlua::Result<Option<Rc<str>>> {
    Ok(t.get::<Option<String>>(key)?.map(|s| Rc::from(s.as_str())))
}
impl Rule {
    /// Reads `estimate`, saying what's wrong with one that can't be used.
    pub(crate) fn read(table: &Table) -> mlua::Result<Self> {
        let mut texts = Vec::new();
        let root = Self::part(table, &mut texts)?;
        Ok(Self { root, texts })
    }
    fn part(t: &Table, texts: &mut Vec<TextRule>) -> mlua::Result<Part> {
        let mut part = Part {
            height: number(t, "height", 0.)?,
            ..Part::default()
        };
        match t.get::<Option<Table>>("text")? {
            // One text's table, or a list of them.
            Some(text) if text.raw_len() > 0 => {
                for one in text.sequence_values::<Table>() {
                    part.texts.push(Self::text(&one?, texts)?);
                }
            }
            Some(text) => part.texts.push(Self::text(&text, texts)?),
            None => {}
        }
        if let Some(field) = name(t, "each")? {
            let item: Table = t.get::<Option<Table>>("item")?.ok_or_else(|| {
                mlua::Error::runtime("ui.list `estimate`: `each` needs `item`, what each adds")
            })?;
            part.each = Some((field, Box::new(Self::part(&item, texts)?)));
        }
        if let Some(field) = name(t, "by")? {
            let cases: Table = t.get::<Option<Table>>("cases")?.ok_or_else(|| {
                mlua::Error::runtime(
                    "ui.list `estimate`: `by` needs `cases`, a part for each value",
                )
            })?;
            let mut list = Vec::new();
            for pair in cases.pairs::<String, Table>() {
                let (value, case) = pair?;
                list.push((Rc::from(value.as_str()), Self::part(&case, texts)?));
            }
            let default = match t.get::<Option<Table>>("default")? {
                Some(d) => Some(Box::new(Self::part(&d, texts)?)),
                None => None,
            };
            part.by = Some((field, list, default));
        }
        if let Some(has) = t.get::<Option<Table>>("has")? {
            for pair in has.pairs::<String, Table>() {
                let (field, adds) = pair?;
                part.has
                    .push((Rc::from(field.as_str()), Self::part(&adds, texts)?));
            }
        }
        Ok(part)
    }
    fn text(t: &Table, texts: &mut Vec<TextRule>) -> mlua::Result<usize> {
        let field = name(t, "field")?.ok_or_else(|| {
            mlua::Error::runtime("ui.list `estimate`: a `text` needs `field`, the field holding it")
        })?;
        let size = number(t, "size", 14.)?;
        let markdown = match t.get::<Option<Table>>("markdown")? {
            Some(m) => Some(Markdown {
                paragraph: number(&m, "paragraph", 0.)?,
                heading: number(&m, "heading", 0.)?,
                item: number(&m, "item", 0.)?,
                code: number(&m, "code", 0.)?,
                table: number(&m, "table", 0.)?,
            }),
            None => None,
        };
        texts.push(TextRule {
            field,
            size,
            line: number(t, "line", size * 1.4)?,
            char: number(t, "char", 0.5)?,
            word: number(t, "word", 5.)?,
            inset: number(t, "inset", 0.)?,
            most: t.get::<Option<f32>>("most")?.unwrap_or(f32::MAX),
            minus: number(t, "minus", 0.)?,
            share: number(t, "share", 1.)?,
            most_lines: t.get::<Option<f32>>("most_lines")?,
            beyond: number(t, "beyond", 0.)?,
            markdown,
        });
        Ok(texts.len() - 1)
    }
}
impl TextRule {
    /// Its column at a list this wide; never so narrow lines count no end.
    fn column(&self, width: f32) -> f32 {
        let column = ((width - self.inset).min(self.most) - self.minus) * self.share;
        (column - self.word / 2. * self.char * self.size).max(self.size)
    }
}
/// How wide a line is, in ems: wide characters (Han, kana, Hangul, full-width
/// forms, emoji) an em, the rest `narrow`.
fn line_ems(line: &str, narrow: f32) -> f32 {
    if line.is_ascii() {
        return line.len() as f32 * narrow;
    }
    line.chars()
        .map(|c| if is_wide(c as u32) { 1. } else { narrow })
        .sum()
}
fn is_wide(c: u32) -> bool {
    matches!(
        c,
        0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1FAFF
            | 0x20000..=0x3FFFD
    )
}
/// Whether a Markdown line begins an item: `- `, `* `, `+ `, or `1. ` and `1) `.
fn starts_an_item(line: &str) -> bool {
    if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ") {
        return true;
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    (1..=9).contains(&digits) && {
        let rest = &line[digits..];
        rest.starts_with(". ") || rest.starts_with(") ")
    }
}
/// A row, scanned: what doesn't change with width, and its lines' widths.
#[derive(Debug, Default)]
struct Scanned {
    fixed: f32,
    /// By text rule: the lines' widths in ems.
    lines: Vec<(u32, Box<[f32]>)>,
}
impl Scanned {
    fn of(rule: &Rule, row: &Value) -> Self {
        let mut out = Self::default();
        out.add(rule, &rule.root, row);
        out
    }
    fn add(&mut self, rule: &Rule, part: &Part, row: &Value) {
        self.fixed += part.height;
        for &index in &part.texts {
            let text = &rule.texts[index];
            if let Some(Value::Str(s)) = row.field(&text.field) {
                self.text(text, index, s);
            }
        }
        if let Some((field, item)) = &part.each
            && let Some(Value::List(list)) = row.field(field)
        {
            for entry in list.iter() {
                self.add(rule, item, entry);
            }
        }
        if let Some((field, cases, default)) = &part.by {
            let value = match row.field(field) {
                Some(Value::Str(s)) => Some(&**s),
                _ => None,
            };
            let case = value.and_then(|v| cases.iter().find(|(name, _)| &**name == v));
            match (case, default) {
                (Some((_, part)), _) => self.add(rule, part, row),
                (None, Some(part)) => self.add(rule, part, row),
                (None, None) => {}
            }
        }
        for (field, adds) in &part.has {
            let holds = match row.field(field) {
                None | Some(Value::Nil) | Some(Value::Bool(false)) => false,
                Some(Value::List(list)) => !list.is_empty(),
                Some(Value::Str(s)) => !s.is_empty(),
                Some(_) => true,
            };
            if holds {
                self.add(rule, adds, row);
            }
        }
    }
    fn text(&mut self, text: &TextRule, index: usize, s: &str) {
        let mut lines = Vec::new();
        match &text.markdown {
            None => {
                for line in s.split('\n') {
                    lines.push(line_ems(line, text.char));
                }
            }
            Some(md) => {
                let (mut fence, mut blank) = (false, true);
                // A paragraph's or an item's source lines flow as one: wrapped
                // by hand or not, they fill the column they get.
                let mut flowing: Option<f32> = None;
                for line in s.split('\n') {
                    let t = line.trim_start();
                    if t.starts_with("```") {
                        lines.extend(flowing.take());
                        if !fence {
                            self.fixed += md.paragraph;
                        }
                        fence = !fence;
                        blank = true;
                    } else if fence {
                        self.fixed += md.code;
                    } else if t.is_empty() {
                        lines.extend(flowing.take());
                        blank = true;
                    } else {
                        if blank {
                            self.fixed += md.paragraph;
                        }
                        blank = false;
                        if t.starts_with('#') {
                            lines.extend(flowing.take());
                            self.fixed += md.heading;
                        } else if t.starts_with('|') {
                            lines.extend(flowing.take());
                            self.fixed += md.table;
                        } else {
                            if starts_an_item(t) {
                                lines.extend(flowing.take());
                                self.fixed += md.item;
                            }
                            let ems = line_ems(t, text.char);
                            flowing = Some(flowing.map_or(ems, |was| was + text.char + ems));
                        }
                    }
                }
                lines.extend(flowing);
            }
        }
        self.lines.push((index as u32, lines.into_boxed_slice()));
    }
    fn height(&self, rule: &Rule, per_em: &[f32]) -> f32 {
        let mut height = self.fixed;
        for (index, lines) in &self.lines {
            let text = &rule.texts[*index as usize];
            let per_em = per_em[*index as usize];
            let count: f32 = lines.iter().map(|ems| (ems * per_em).ceil()).sum();
            height += match text.most_lines {
                Some(most) if count > most => most * text.line + text.beyond,
                _ => count * text.line,
            };
        }
        height.max(1.)
    }
}
/// A list's estimates: told of each pass of the list.
pub(crate) struct Estimator {
    rule: Rc<Rule>,
    items: Rc<dyn Fn() -> Value>,
    heights: VariableHeights,
    width: Cell<f32>,
    /// The list last seen, each row's scan by the row.
    seen: Cell<usize>,
    scanned: RefCell<FxHashMap<usize, Rc<Scanned>>>,
}
impl Estimator {
    pub(crate) fn new(
        rule: Rc<Rule>,
        items: Rc<dyn Fn() -> Value>,
        heights: VariableHeights,
    ) -> Self {
        Self {
            rule,
            items,
            heights,
            width: Cell::new(0.),
            seen: Cell::new(0),
            scanned: RefCell::default(),
        }
    }
}
impl RowsWatch for Estimator {
    fn pass(&self, width: f32, mounted: &mut dyn Iterator<Item = usize>) {
        if width <= 0. {
            return;
        }
        let widened = self.width.replace(width) != width;
        let items = (self.items)();
        let list = crate::value::identity(&items);
        if !widened && self.seen.get() == list {
            return;
        }
        self.seen.set(list);
        // Rows the same as they were keep their scans; a new or changed one
        // is scanned, and only those get heights while the width stands: the
        // others hold what they were measured at.
        let rows = items.list();
        let mut scanned = self.scanned.borrow_mut();
        let mut next: FxHashMap<usize, Rc<Scanned>> = FxHashMap::default();
        next.reserve(rows.len());
        let per_em: Vec<f32> = self
            .rule
            .texts
            .iter()
            .map(|t| t.size / t.column(width))
            .collect();
        let mut updates = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let at = crate::value::identity(row);
            let known = scanned.get(&at).filter(|_| at != 0).cloned();
            let new = known.is_none();
            let scan = known.unwrap_or_else(|| Rc::new(Scanned::of(&self.rule, row)));
            if new || widened {
                updates.push((index, scan.height(&self.rule, &per_em)));
            }
            next.insert(at, scan);
        }
        *scanned = next;
        drop(scanned);
        // A mounted row's height is its own.
        let mounted: rustc_hash::FxHashSet<usize> = mounted.collect();
        updates.retain(|(index, _)| !mounted.contains(index));
        self.heights.set_heights(&updates);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn rule(code: &str) -> Rule {
        let lua = mlua::Lua::new();
        Rule::read(&lua.load(code).eval::<Table>().unwrap()).unwrap()
    }
    fn height_of(rule: &Rule, row: &Value, width: f32) -> f32 {
        let per_em: Vec<f32> = rule
            .texts
            .iter()
            .map(|t| t.size / t.column(width))
            .collect();
        Scanned::of(rule, row).height(rule, &per_em)
    }
    fn row(text: &str) -> Value {
        Value::record([("text", Value::from(text))])
    }
    const PLAIN: &str = "return { height = 10, text = { field = 'text', size = 10, line = 20, char = 0.5, word = 0, word = 0 } }";

    #[test]
    fn lines_follow_the_width() {
        let r = rule(PLAIN);
        // 100 characters, 5 points each: 500 points.
        let text = "a".repeat(100);
        assert_eq!(height_of(&r, &row(&text), 500.), 10. + 20.);
        assert_eq!(height_of(&r, &row(&text), 499.), 10. + 40.);
        assert_eq!(height_of(&r, &row(&text), 250.), 10. + 40.);
        assert_eq!(height_of(&r, &row(&text), 249.), 10. + 60.);
    }
    #[test]
    fn wide_characters_count_an_em() {
        let r = rule(PLAIN);
        let (narrow, wide) = ("a".repeat(100), "漢".repeat(100));
        // Twice as wide at half an em: twice the lines.
        assert_eq!(height_of(&r, &row(&narrow), 500.), 30.);
        assert_eq!(height_of(&r, &row(&wide), 500.), 50.);
    }
    #[test]
    fn a_column_takes_its_inset_cap_and_share() {
        let r = rule(
            "return { text = { field = 'text', size = 10, line = 20, char = 0.5, word = 0,
                inset = 100, most = 400, minus = 100, share = 0.5 } }",
        );
        // At most (400 - 100) * 0.5 = 150 points, however wide: 30 characters a line.
        assert_eq!(height_of(&r, &row(&"a".repeat(30)), 2000.), 20.);
        assert_eq!(height_of(&r, &row(&"a".repeat(31)), 2000.), 40.);
    }
    #[test]
    fn a_clamp_shows_its_most_lines_and_what_is_beyond() {
        let r = rule(
            "return { text = { field = 'text', size = 10, line = 20, char = 0.5, word = 0,
                most_lines = 2, beyond = 7 } }",
        );
        assert_eq!(height_of(&r, &row("a\nb"), 500.), 40.);
        assert_eq!(height_of(&r, &row("a\nb\nc"), 500.), 47.);
    }
    #[test]
    fn markdown_lines_add_their_kinds() {
        let r = rule(
            "return { text = { field = 'text', size = 10, line = 20, char = 0.5, word = 0,
                markdown = { paragraph = 1, heading = 100, item = 1000, code = 10000, table = 100000 } } }",
        );
        assert_eq!(
            height_of(&r, &row("one\nstill one\n\ntwo"), 500.),
            2. + 2. * 20.
        );
        assert_eq!(height_of(&r, &row("# title"), 500.), 1. + 100.);
        assert_eq!(height_of(&r, &row("- item"), 500.), 1. + 1000. + 20.);
        assert_eq!(
            height_of(&r, &row("```\ncode\nmore\n```"), 500.),
            1. + 20000.
        );
        assert_eq!(height_of(&r, &row("| a | b |"), 500.), 1. + 100000.);
    }
    #[test]
    fn markdown_lines_wrapped_by_hand_flow_as_one() {
        let r = rule(
            "return { text = { field = 'text', size = 10, line = 20, char = 0.5, word = 0,
                markdown = {} } }",
        );
        // 60 and 60 characters and a space: 605 points, however it's wrapped.
        let wrapped = format!("{0}\n{0}", "a".repeat(60));
        assert_eq!(height_of(&r, &row(&wrapped), 1000.), 20.);
        assert_eq!(height_of(&r, &row(&wrapped), 500.), 40.);
        // Plain text keeps its lines: only Markdown joins them.
        let plain = rule(
            "return { text = { field = 'text', size = 10, line = 20, char = 0.5, word = 0 } }",
        );
        assert_eq!(height_of(&plain, &row(&wrapped), 1000.), 40.);
        // An item goes on over its lines, and each item is its own.
        assert_eq!(height_of(&r, &row("- one\n  still one\n- two"), 1000.), 40.);
        // A number is an item's only with its mark.
        for (line, item) in [
            ("3. x", true),
            ("3) x", true),
            ("3 x", false),
            ("3.x", false),
            ("+ x", true),
            ("-x", false),
        ] {
            assert_eq!(starts_an_item(line), item, "{line}");
        }
    }
    #[test]
    fn parts_choose_by_field_and_each_and_has() {
        let r = rule(
            "return { height = 1,
                has = { tags = { height = 10 } },
                by = 'kind',
                cases = { big = { height = 100 } },
                default = { height = 1000 },
                each = 'items', item = { height = 10000 } }",
        );
        let of = |fields: Vec<(&str, Value)>| height_of(&r, &Value::record(fields), 500.);
        assert_eq!(of(vec![("kind", Value::from("big"))]), 101.);
        assert_eq!(of(vec![("kind", Value::from("other"))]), 1001.);
        assert_eq!(of(vec![]), 1001.);
        let two = Value::List(Rc::from(vec![Value::Num(1.), Value::Num(2.)]));
        assert_eq!(
            of(vec![("kind", Value::from("big")), ("items", two.clone())]),
            20101.
        );
        assert_eq!(
            of(vec![("kind", Value::from("big")), ("tags", two)]),
            20111. - 20000.
        );
        let none = Value::List(Rc::from(Vec::new()));
        assert_eq!(of(vec![("kind", Value::from("big")), ("tags", none)]), 101.);
    }
    #[test]
    fn what_is_wrong_with_a_rule_is_said() {
        let lua = mlua::Lua::new();
        let wrong = |code: &str| {
            let t = lua.load(code).eval::<Table>().unwrap();
            Rule::read(&t).unwrap_err().to_string()
        };
        assert!(wrong("return { text = { size = 3 } }").contains("needs `field`"));
        assert!(wrong("return { each = 'x' }").contains("needs `item`"));
        assert!(wrong("return { by = 'x' }").contains("needs `cases`"));
        assert!(wrong("return { height = -1 }").contains("`height`"));
    }
}
