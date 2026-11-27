//! `ui.actions { name = function(arg) … end }`: actions a script defines,
//! called as a Rust host's are (`ui.invoke(name, arg)`, `ui.action(name,
//! arg)` as a handler), by every script. A Luau app run by `zgx run` has no
//! Rust of its own: this is how its scripts offer each other what to call.
//!
//! Defined again as a reload runs the script again.
use crate::{Actions, value::Value};
use mlua::{Function, Lua, Table};
use std::rc::Rc;

pub(crate) fn install(lua: &Lua, ui: &Table, actions: Actions) -> mlua::Result<()> {
    ui.set(
        "actions",
        lua.create_function(move |lua, table: Table| {
            crate::reloads::top_level(lua, "ui.actions");
            let vm = crate::node::env(lua)?.vm.clone();
            for pair in table.pairs::<String, Function>() {
                let (name, function) = pair.map_err(|_| {
                    mlua::Error::runtime("ui.actions takes { name = function(arg) … end }")
                })?;
                let vm = vm.clone();
                let action = move |arg: Value| {
                    if let Some(vm) = vm.upgrade()
                        && let Err(error) = vm.call_as::<()>("an action's function", &function, arg)
                    {
                        vm.report(error);
                    }
                };
                actions.borrow_mut().insert(name, Rc::new(action));
            }
            Ok(())
        })?,
    )
}
