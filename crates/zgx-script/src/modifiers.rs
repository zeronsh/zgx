//! `ui.modifiers`: the modifier keys held now, each a state (`:get()`
//! tracked), a modifier alone included (Command held to show shortcut
//! hints). Cleared as the window loses focus. Made the first time a script
//! reads `ui.modifiers`; until then the host writes nothing.
use crate::{node::env, value::Value};
use mlua::{Lua, Table};
use zgx::input::Modifiers;

pub(crate) fn install(lua: &Lua, ui: &Table) -> mlua::Result<()> {
    let runtime = env(lua)?.runtime.clone();
    let held = zgx::input::held_modifiers(&runtime);
    let modifiers = lua.create_table()?;
    type Read = fn(Modifiers) -> bool;
    let keys: [(&str, Read); 4] = [
        ("shift", |m| m.shift),
        ("control", |m| m.control),
        ("alt", |m| m.alt),
        ("meta", |m| m.meta),
    ];
    for (name, read) in keys {
        let initial = runtime.untracked(|| read(held.get()));
        let state = runtime.signal(Value::Bool(initial));
        let (from, mirror) = (held.clone(), state.clone());
        let follow = runtime.effect(move || {
            mirror.set(Value::Bool(read(from.get())));
        });
        env(lua)?.retained.borrow_mut().push(Box::new(follow));
        modifiers.set(name, crate::value::State::new(state))?;
    }
    modifiers.set_readonly(true);
    ui.set("modifiers", modifiers)
}
