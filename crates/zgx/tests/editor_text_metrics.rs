//! `text_metrics` after edits (the natural layouts): its height is a note's
//! opened with the same text, and what that note draws, at two widths, asked
//! in the edit's own effect, at once after it (no frame yet) and after a
//! frame: fences typed, deleted, undone, redone, split, merged; headings
//! with sizes and room of their own; code that never wraps in a scroll group,
//! and code that wraps.
use std::{cell::Cell, rc::Rc};
use zgx::{
    compose::prelude::*,
    input::InputEvent,
    scene::Color,
    theme::{Theme, ThemeKind, tok},
    widgets::{EditorHandle, LineInset, SpanStyle, Ui},
};

const WIDTHS: [f32; 2] = [400., 250.];
/// Fenced lines never wrap, in one scroll group (`Block`), or wrap as prose
/// does, with the same room (`Wrapped`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Arm {
    Block,
    Wrapped,
}

/// Lines by name: those starting `L` are wide (wrapping at both widths),
/// those starting `#` are headings (larger, with room of their own).
fn doc(names: &[&str]) -> String {
    let lines: Vec<String> = (names.iter())
        .map(|name| match name.starts_with('L') {
            true => format!("{name} {}", "word ".repeat(30)),
            false => name.to_string(),
        })
        .collect();
    lines.join("\n")
}
/// A text as it is told of: its lines' first words.
fn told(text: &str) -> String {
    (text.split('\n'))
        .map(|line| match line.starts_with('L') {
            true => line.split(' ').next().unwrap_or("").to_string(),
            false => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" / ")
}
fn base(second: &str) -> Vec<String> {
    let mut lines = vec!["intro".into(), second.to_string(), "# Head".into()];
    lines.extend((0..24).map(|i| format!("L{i:02}")));
    lines.push("# Tail head".into());
    lines.push("end".into());
    lines
}
fn doc_of(lines: &[String]) -> String {
    doc(&lines.iter().map(String::as_str).collect::<Vec<_>>())
}
fn blocks() -> String {
    let mut names = vec![
        "# Top",
        "```",
        "LA1",
        "LA2",
        "# in code",
        "```",
        "LM0",
        "```",
        "LB1",
        "```",
    ];
    names.extend(["LP0"; 14]);
    names.extend(["```", "LC1", "```", "# Tail head", "end"]);
    doc(&names)
}

struct Failures(Vec<String>);
impl Failures {
    /// What `n` measures now at `widths`, the only ones it is asked at, in
    /// each place it is asked, against a note opened with its text, and what
    /// each draws.
    fn check(
        &mut self,
        (arm, warm, widths): (Arm, bool, &[f32]),
        what: &str,
        before: &str,
        n: &mut Note,
        at_once: &[f32],
    ) {
        let text = n.text();
        let in_effect = if warm { Some(n.in_effect()) } else { None };
        n.frame();
        let after_frame: Vec<f32> = widths.iter().map(|w| n.measure(*w)).collect();
        for (k, width) in widths.iter().copied().enumerate() {
            let mut opened = note(&text, arm, width, &[]);
            let want = opened.measure(width);
            let opened_drawn = opened.drawn();
            // The edited area is WIDTHS[0] wide.
            let drawn = (width == WIDTHS[0]).then(|| n.drawn());
            let mut places = vec![("at once", at_once[k]), ("after a frame", after_frame[k])];
            places.extend(in_effect.map(|all| ("in the edit's effect", all[k])));
            let mut bad: Vec<String> = (places.iter())
                .filter(|(_, got)| (got - want).abs() > 0.05)
                .map(|(place, got)| format!("text_metrics {place}: got {got}"))
                .collect();
            if (opened_drawn - want).abs() > 0.05 {
                bad.push(format!("a note opened so draws {opened_drawn}"));
            }
            if let Some(drawn) = drawn
                && (drawn - opened_drawn).abs() > 0.05
            {
                bad.push(format!("the edited area draws {drawn}"));
            }
            let state = if bad.is_empty() { "PASS" } else { "FAIL" };
            println!(
                "{state} {arm:?} warm={warm} asked at {widths:?}: {what}, width {width}: expected {want}, opened draws {opened_drawn}, edited draws {drawn:?}, {places:?}"
            );
            if !bad.is_empty() {
                println!("  before: {}\n  after:  {}", told(before), told(&text));
                self.0.push(format!("{arm:?} warm={warm} asked at {widths:?}: {what}, width {width}: expected {want}; {}", bad.join("; ")));
            }
        }
        n.top();
    }
    fn done(self) {
        assert!(
            self.0.is_empty(),
            "{} failed:\n{}",
            self.0.len(),
            self.0.join("\n")
        );
    }
}
/// An edit made, then undone and done again, checked at each step. `typed`:
/// a character at a time, as keys.
fn steps(
    f: &mut Failures,
    arm: Arm,
    what: &str,
    start: &str,
    range: std::ops::Range<usize>,
    with: &str,
    typed: bool,
) {
    // Asked at one width alone, and at both in turn.
    for widths in [&WIDTHS[..1], &WIDTHS[1..], &WIDTHS[..]] {
        for warm in [true, false] {
            let how = (arm, warm, widths);
            let ask = |n: &mut Note| widths.iter().map(|w| n.measure(*w)).collect::<Vec<f32>>();
            let mut n = note(start, arm, WIDTHS[0], if warm { widths } else { &[] });
            if warm {
                ask(&mut n);
                n.frame();
            }
            n.edit(range.clone(), with, typed);
            let after = n.text();
            assert_ne!(after, start);
            let at_once = ask(&mut n);
            f.check(how, what, start, &mut n, &at_once);
            n.step_to(start, true);
            let at_once = ask(&mut n);
            f.check(how, &format!("{what}, undone"), &after, &mut n, &at_once);
            n.step_to(&after, false);
            let at_once = ask(&mut n);
            f.check(how, &format!("{what}, redone"), start, &mut n, &at_once);
        }
    }
}
fn cases(arm: Arm) {
    let mut f = Failures(Vec::new());
    let plain = doc_of(&base(""));
    let fenced = doc_of(&base("```"));
    steps(&mut f, arm, "fence typed", &plain, 6..6, "```", true);
    steps(&mut f, arm, "fence deleted", &fenced, 6..9, "", false);
    steps(&mut f, arm, "fence backspaced", &fenced, 8..9, "", false);
    let blocks = blocks();
    let at = |s: &str| blocks.find(s).unwrap();
    steps(
        &mut f,
        arm,
        "split",
        &blocks,
        at("LA2")..at("LA2"),
        "```\n",
        false,
    );
    steps(
        &mut f,
        arm,
        "merge",
        &blocks,
        at("```\nLM0")..at("LB1"),
        "",
        false,
    );
    steps(&mut f, arm, "flip", &blocks, 6..10, "", false);
    steps(
        &mut f,
        arm,
        "cut short",
        &blocks,
        at("LA1") + 3..at("LA2") - 1,
        "",
        false,
    );
    steps(&mut f, arm, "letter above", &fenced, 2..2, "z", true);
    let inside = fenced.find("L05").unwrap() + 3;
    steps(
        &mut f,
        arm,
        "letter inside",
        &fenced,
        inside..inside,
        "z",
        true,
    );
    steps(&mut f, arm, "heading made plain", &plain, 7..9, "", false);
    let head = plain.find("# Tail").unwrap();
    steps(
        &mut f,
        arm,
        "heading typed",
        &doc_of(&base("")).replace("# Tail", "Tail"),
        head..head,
        "# ",
        true,
    );
    let paste = doc(&["```", "LX1", "# in", "LX2", "```", "# after", "LX3"]);
    steps(&mut f, arm, "block pasted", &plain, 6..6, &paste, false);
    let open = doc(&["```", "LX1", "# in", "LX2"]);
    steps(
        &mut f,
        arm,
        "unclosed block pasted",
        &plain,
        6..6,
        &open,
        false,
    );
    f.done();
}
#[test]
fn text_metrics_follow_edits_in_blocks() {
    cases(Arm::Block);
}
#[test]
fn text_metrics_follow_edits_in_wrapped_code() {
    cases(Arm::Wrapped);
}
/// A highlighter, insets and a theme set after: the metrics follow.
#[test]
fn text_metrics_follow_what_is_set() {
    let mut f = Failures(Vec::new());
    for arm in [Arm::Block, Arm::Wrapped] {
        for width in WIDTHS {
            let text = doc_of(&base("```"));
            let mut n = note(&text, arm, WIDTHS[0], &[width]);
            n.measure(width);
            n.editor().rehighlight();
            let at_once = [n.measure(width)];
            f.check(
                (arm, false, &[width]),
                "asked anew (rehighlight)",
                &text,
                &mut n,
                &at_once,
            );
            n.theme();
            let at_once = [n.measure(width)];
            f.check(
                (arm, false, &[width]),
                "a theme set",
                &text,
                &mut n,
                &at_once,
            );
        }
    }
    f.done();
}

struct Note {
    ui: Ui,
    editor: EditorHandle,
    seen: Rc<[Cell<f32>; 2]>,
}
/// `told`: an effect on its text measures it, as a composer's does.
fn note(text: &str, arm: Arm, width: f32, told: &[f32]) -> Note {
    let mut ui = Ui::new(600., 400.);
    for (name, color) in [("a", Color(255, 0, 0, 255)), ("b", Color(0, 255, 0, 255))] {
        ui.theme()
            .register(Theme::new(name, ThemeKind::Dark).with(tok::TEXT_PRIMARY, color));
    }
    ui.theme().set("a").unwrap();
    let value = ui.signal(text.to_string());
    let view = ui.mount(
        text_area("Note", value.clone())
            .w(width)
            .h(100.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(true)
            .span_highlighter(|line: &str, state| {
                let mut spans = Vec::new();
                if line.starts_with('#') {
                    let style = SpanStyle::default().size(24.).line_height(34.).weight(600);
                    spans.push((0..line.len(), style));
                }
                (spans, if line == "```" { 1 - state } else { state })
            })
            .line_insets(move |_, line: &str, state| {
                if state == 1 && line != "```" {
                    let inset = LineInset::new(12., 2., 1.);
                    match arm {
                        Arm::Block => inset.in_group(1).unwrapped(),
                        Arm::Wrapped => inset,
                    }
                } else if line.starts_with('#') {
                    LineInset::new(0., 16., 6.)
                } else {
                    LineInset::default()
                }
            }),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    let seen = Rc::new([Cell::new(0.), Cell::new(0.)]);
    if !told.is_empty() {
        let told = told.to_vec();
        let (handle, seen) = (editor.clone(), seen.clone());
        ui.bind(view.node(), move || {
            value.get();
            for (k, width) in told.iter().copied().enumerate() {
                let height = handle.text_metrics(width).map_or(-1., |m| m.wrapped_height);
                seen[k].set(height);
            }
        });
    }
    editor.set_selection(0, 0);
    ui.input.focus(&ui.scene, Some(view.node()));
    let mut n = Note { ui, editor, seen };
    n.frame();
    n
}
impl Note {
    fn measure(&mut self, width: f32) -> f32 {
        (self.editor.text_metrics(width)).map_or(-1., |m| m.wrapped_height)
    }
    fn in_effect(&self) -> [f32; 2] {
        [self.seen[0].get(), self.seen[1].get()]
    }
    fn theme(&mut self) {
        self.ui.theme().set("b").unwrap();
    }
    fn frame(&mut self) {
        for _ in 0..2 {
            self.ui.prepare_frame();
            self.ui.scene.borrow_mut().flush();
        }
    }
}

impl Note {
    fn text(&self) -> String {
        self.editor.editor.borrow().text().to_string()
    }
    fn editor(&self) -> EditorHandle {
        self.editor.clone()
    }
    fn edit(&mut self, range: std::ops::Range<usize>, with: &str, typed: bool) {
        if !typed {
            assert!(self.editor.replace_text(range, with));
            return;
        }
        self.editor.set_selection(range.start, range.end);
        for c in with.chars() {
            self.ui.dispatch(InputEvent::Text(c.to_string()));
        }
    }
    /// Undo, or redo, until the text is `want`, with no frame between.
    fn step_to(&mut self, want: &str, undo: bool) {
        for _ in 0..8 {
            if self.text() == want {
                return;
            }
            assert!(if undo {
                self.editor.undo()
            } else {
                self.editor.redo()
            });
        }
        assert_eq!(self.text(), want);
    }
    fn top(&mut self) {
        self.editor.set_scroll_offset(0.);
        self.frame();
    }
    /// How high the drawn area is, its every line shown on the way down.
    fn drawn(&mut self) -> f32 {
        self.top();
        let mut last = -1.;
        while self.editor.scroll_offset().1 > last {
            last = self.editor.scroll_offset().1;
            self.editor.set_scroll_offset(last + 40.);
            self.frame();
        }
        let view = self.ui.scene.borrow().bounds(self.editor.node).height;
        self.editor.scroll_offset().1 + view
    }
}

/// An effect on the text made before the area runs before the area's own:
/// asked there, typing and undo measure the text as it is now, and what the
/// app sets is measured once the area has taken it, told again after layout.
#[test]
fn an_effect_that_runs_first_measures_the_text_it_is_told_of() {
    let mut ui = Ui::new(600., 400.);
    let value = ui.signal(String::from("one\ntwo"));
    let kept: Rc<std::cell::RefCell<Option<EditorHandle>>> = Rc::default();
    let log = Rc::new(std::cell::RefCell::new(Vec::new()));
    let (handle, told, text) = (kept.clone(), log.clone(), value.clone());
    let _effect = ui.runtime.effect(move || {
        let lines = text.get().split('\n').count();
        if let Some(editor) = handle.borrow().as_ref() {
            let height = editor.text_metrics(200.).map(|m| m.wrapped_height);
            told.borrow_mut().push((lines, height));
        }
    });
    let view = ui.mount(
        text_area("Note", value.clone())
            .w(200.)
            .h(100.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(true),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    *kept.borrow_mut() = Some(editor.clone());
    editor.set_selection(0, 0);
    ui.input.focus(&ui.scene, Some(view.node()));
    // Read once, so it runs again as the metrics change.
    value.set("one\ntwo\nthree".into());
    ui.prepare_frame();
    let set = std::mem::take(&mut *log.borrow_mut());
    // Told before the area took it, and again once laid out.
    assert_eq!(set, [(3, Some(40.)), (3, Some(60.))]);
    for _ in 0..2 {
        ui.dispatch(InputEvent::Text("\n".into()));
        let turn = std::mem::take(&mut *log.borrow_mut());
        assert!(!turn.is_empty());
        for (lines, height) in &turn {
            assert_eq!(*height, Some(20. * *lines as f32), "{turn:?}");
        }
        ui.prepare_frame();
        log.borrow_mut().clear();
    }
    assert!(editor.undo());
    let turn = std::mem::take(&mut *log.borrow_mut());
    let (lines, height) = *turn.first().expect("told in the turn");
    assert_eq!(height, Some(20. * lines as f32), "{turn:?}");
    // Nothing more to tell once it settles.
    for _ in 0..3 {
        ui.prepare_frame();
    }
    log.borrow_mut().clear();
    for _ in 0..3 {
        ui.prepare_frame();
    }
    assert!(log.borrow().is_empty());
}
