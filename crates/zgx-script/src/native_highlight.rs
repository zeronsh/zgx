//! A native module's line highlighter, which a textarea calls itself with no
//! script between (`zgx_module::Host::highlighter`, unstable, doc(hidden)): a
//! script chooses it and gives its kinds colours, `ui.highlighter(name,
//! { kind = colour })`, and hands it to a textarea's `highlight` in place of a
//! function. A highlighter of Luau colours a long file's lines one call and
//! one table of spans each; this one is a call into the module, its spans
//! handed over as numbers.
//!
//! Its states are whole numbers, 0 where nothing is open. A textarea that
//! shows a window of a longer text starts its first line from
//! [`Highlighter::start`] (`h:start(n)`, the state its window's first line has
//! in the whole text), which the textarea asks for as 0: the states a textarea
//! keeps are the highlighter's plus one. `h:states(text, step)` is what every
//! `step`th line of a text leaves, and `h:after(line, state)` one line's, for
//! the script to find a window's `start` by.
use mlua::{Lua, Table, UserData, UserDataMethods};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

/// A highlighter: what its kinds are called, by number, and its call (a
/// line's spans to `push`, bytes from 1 with both ends in, and what the line
/// leaves).
pub(crate) struct Native {
    /// What scripts and errors call it.
    pub(crate) name: String,
    pub(crate) kinds: Vec<String>,
    pub(crate) line: Box<Line>,
}

/// A highlighter's call: a line and what the line before left, each span
/// handed to the `FnMut`, what the line leaves returned.
pub(crate) type Line = dyn Fn(&[u8], u32, &mut dyn FnMut(u32, u32, u32)) -> u32;

/// What a highlighter returns when it failed, and the states it may leave
/// (`zgx_module::abi`'s `HIGHLIGHT_FAILED` and `HIGHLIGHT_STATES`).
const FAILED: u32 = u32::MAX;
const STATES: u32 = (1 << 31) - 1;

/// A span as the runtime takes it from a highlighter: bytes of its line
/// (from 0, the end out), and its kind's number.
pub(crate) type Span = (std::ops::Range<usize>, usize);

impl Native {
    /// What `line` leaves, from `state`, and its spans in `out`, none of
    /// them trusted: each kept only within the line, of a kind it has, in
    /// order and not overlapping (a later one over an earlier is let go),
    /// and no more of them than the line has bytes. Err where it failed
    /// (a panic, or a state past its own), saying so.
    pub(crate) fn spans(
        &self,
        line: &[u8],
        state: u32,
        out: &mut Vec<Span>,
    ) -> Result<u32, String> {
        out.clear();
        let (n, kinds) = (line.len(), self.kinds.len());
        let mut ordered = true;
        let next = (self.line)(line, state, &mut |from, to, kind| {
            let (from, to, kind) = (from as usize, to as usize, kind as usize);
            if out.len() >= n || from == 0 || from > n || to < from || kind >= kinds {
                return;
            }
            let range = from - 1..to.min(n);
            ordered &= out.last().is_none_or(|(last, _)| range.start >= last.end);
            out.push((range, kind));
        });
        let next = self.checked(next)?;
        if !ordered {
            out.sort_by_key(|(range, _)| range.start);
            let mut end = 0;
            out.retain(|(range, _)| {
                let keep = range.start >= end;
                if keep {
                    end = range.end;
                }
                keep
            });
        }
        Ok(next)
    }
    /// What a line left, or why it is a failure.
    fn checked(&self, next: u32) -> Result<u32, String> {
        match next {
            FAILED => Err(format!(
                "the highlighter `{}` failed (its module panicked): it is off for this textarea",
                self.name
            )),
            next if next >= STATES => Err(format!(
                "the highlighter `{}` left a state past 2^31 - 2 ({next}): it is off for this textarea",
                self.name
            )),
            next => Ok(next),
        }
    }
}

type FunctionsOf = <crate::Functions as std::ops::Deref>::Target;
/// A host's highlighter as found: its host, its name, and it.
type Found = (Weak<FunctionsOf>, String, Weak<Native>);

/// A host's highlighters, by name: what keeps them.
pub(crate) type Owned = RefCell<Vec<(String, Rc<Native>)>>;

thread_local! {
    /// Each host's highlighters, by its functions (as `modules::LOADED`
    /// knows a host's modules) and name: found by `ui.highlighter`, kept by
    /// the host ([`Owned`]).
    static NATIVE: RefCell<Vec<Found>> = const { RefCell::new(Vec::new()) };
}

/// Registers `native` as `name` for the host of `functions`, which `owned`
/// keeps (a module's, once it has loaded): registered again, the last's.
#[cfg_attr(not(feature = "modules"), allow(dead_code))]
pub(crate) fn register(functions: &crate::Functions, owned: &Owned, name: String, native: Native) {
    let native = Rc::new(native);
    {
        let mut owned = owned.borrow_mut();
        owned.retain(|(named, _)| *named != name);
        owned.push((name.clone(), native.clone()));
    }
    let host = Rc::downgrade(functions);
    NATIVE.with_borrow_mut(|all| {
        all.retain(|(of, named, native)| {
            of.strong_count() > 0
                && native.strong_count() > 0
                && !(of.ptr_eq(&host) && *named == name)
        });
        all.push((host, name, Rc::downgrade(&native)));
    });
}

/// What `ui.highlighter` gives: a highlighter, its kinds' colours, and the
/// state a textarea's first line starts from.
pub(crate) struct Highlighter {
    pub(crate) native: Rc<Native>,
    /// Each kind's colour (a theme token or a colour), by its number.
    pub(crate) colors: Vec<Option<String>>,
    pub(crate) start: Cell<u32>,
}

impl Highlighter {
    /// What `line` leaves, its spans not asked for.
    fn after(&self, line: &[u8], state: u32) -> mlua::Result<u32> {
        self.native
            .checked((self.native.line)(line, state, &mut |_, _, _| {}))
            .map_err(mlua::Error::runtime)
    }
}

pub(crate) struct HighlighterUd(pub(crate) Rc<Highlighter>);

impl UserData for HighlighterUd {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        // The state the textarea's first line starts from.
        m.add_method("start", |_, h, state: u32| {
            h.0.start.set(state);
            Ok(())
        });
        // What each `step`th line of `text` leaves, by that line's number.
        m.add_method(
            "states",
            |lua, h, (text, step): (mlua::LuaString, usize)| {
                let out = lua.create_table()?;
                let step = step.max(1);
                let mut state = 0;
                for (k, line) in text.as_bytes().split(|&b| b == b'\n').enumerate() {
                    state = h.0.after(line, state)?;
                    if (k + 1) % step == 0 {
                        out.raw_set(k + 1, state)?;
                    }
                }
                Ok(out)
            },
        );
        // What `line` leaves, from `state`.
        m.add_method("after", |_, h, (line, state): (mlua::LuaString, u32)| {
            h.0.after(&line.as_bytes(), state)
        });
        // `line`'s spans, `{ from, to, kind }` with the kind by name, and what
        // it leaves: for holding a highlighter to another.
        m.add_method("lex", |lua, h, (line, state): (mlua::LuaString, u32)| {
            let mut spans = Vec::new();
            let next =
                h.0.native
                    .spans(&line.as_bytes(), state, &mut spans)
                    .map_err(mlua::Error::runtime)?;
            let list = lua.create_table_with_capacity(spans.len(), 0)?;
            for (range, kind) in spans {
                list.raw_push(lua.create_sequence_from([
                    mlua::Value::Number((range.start + 1) as f64),
                    mlua::Value::Number(range.end as f64),
                    mlua::Value::String(lua.create_string(&h.0.native.kinds[kind])?),
                ])?)?;
            }
            Ok((list, next))
        });
    }
}

/// `ui.highlighter(name, colours)`: the app's own scripts only, as
/// `ui.module`.
pub(crate) fn install(lua: &Lua, ui: &Table) -> mlua::Result<()> {
    ui.set(
        "highlighter",
        lua.create_function(|lua, (name, colors): (String, Option<Table>)| {
            let functions = {
                let env = crate::node::env(lua)?;
                if !matches!(*env.grants, crate::Grants::All) {
                    return Err(mlua::Error::runtime(format!(
                        "ui.highlighter `{name}`: a module's highlighter is the app's own"
                    )));
                }
                Rc::downgrade(&env.functions)
            };
            let native = NATIVE
                .with_borrow(|all| {
                    all.iter()
                        .find(|(of, named, _)| of.ptr_eq(&functions) && *named == name)
                        .and_then(|(.., native)| native.upgrade())
                })
                .ok_or_else(|| {
                    mlua::Error::runtime(format!(
                        "ui.highlighter: no highlighter named `{name}` (a native module registers one as it loads)"
                    ))
                })?;
            let colors = native
                .kinds
                .iter()
                .map(|kind| {
                    colors
                        .as_ref()
                        .and_then(|c| c.raw_get::<Option<String>>(kind.as_str()).ok().flatten())
                })
                .collect();
            Ok(HighlighterUd(Rc::new(Highlighter {
                native,
                colors,
                start: Cell::new(0),
            })))
        })?,
    )
}
