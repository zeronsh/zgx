//! `ui.glass`, `ui.glass_group`, `ui.symbol`, `ui.native_switch` and
//! `ui.native_tabs`: the system's own material and controls where it has them,
//! made the first time a script names one.
use crate::node::{
    GlassSpec, Kind, Node, Source, SymbolSpec, Ty, control_value, fill, node_ud, owner, source,
};
use mlua::Table;

/// A `ui.native_tabs`'s `items`: each tab's title and system symbol.
fn tab_items(table: &Table) -> mlua::Result<Vec<(String, String)>> {
    let listed: Table = table.get("items")?;
    listed
        .sequence_values::<Table>()
        .map(|item| {
            let item = item?;
            Ok((item.get("title")?, item.get("symbol")?))
        })
        .collect()
}
/// `ui.glass`, `ui.glass_group` and `ui.symbol`, made the first time a script
/// names one.
pub(crate) fn install_glass(lazy: &crate::lazy::Lazy) {
    lazy.ui(
        &[
            "glass",
            "glass_group",
            "symbol",
            "native_switch",
            "native_tabs",
        ],
        |lua, ui| {
            ui.set(
                "native_tabs",
                lua.create_function(|lua, table: Table| {
                    let value = control_value(lua, &table)?;
                    table.raw_set("value", mlua::Value::Nil)?;
                    table.raw_set("on_change", mlua::Value::Nil)?;
                    let items = tab_items(&table)?;
                    table.raw_set("items", mlua::Value::Nil)?;
                    let node = fill(
                        lua,
                        Node::new(owner(lua)?, Kind::Tabs(items, value)),
                        Some(&table),
                    )?;
                    node_ud(lua, node)
                })?,
            )?;
            ui.set(
                "native_switch",
                lua.create_function(|lua, table: Table| {
                    let value = control_value(lua, &table)?;
                    table.raw_set("value", mlua::Value::Nil)?;
                    table.raw_set("on_change", mlua::Value::Nil)?;
                    let label = table.get::<Option<String>>("label")?.unwrap_or_default();
                    table.raw_set("label", mlua::Value::Nil)?;
                    let node = fill(
                        lua,
                        Node::new(owner(lua)?, Kind::Toggle(label, value)),
                        Some(&table),
                    )?;
                    node_ud(lua, node)
                })?,
            )?;
            ui.set(
                "symbol",
                lua.create_function(|lua, table: Table| {
                    let name: String = table.get("name")?;
                    table.raw_set("name", mlua::Value::Nil)?;
                    let take = |name: &str, ty: Ty| -> mlua::Result<Option<Source>> {
                        let value = table.get::<mlua::Value>(name)?;
                        table.raw_set(name, mlua::Value::Nil)?;
                        match value {
                            mlua::Value::Nil => Ok(None),
                            value => source(lua, ty, name, value).map(Some),
                        }
                    };
                    let spec = SymbolSpec {
                        name,
                        size: take("size", Ty::Num)?,
                        weight: take("weight", Ty::Num)?,
                        color: take("color", Ty::Color)?,
                    };
                    let node = fill(
                        lua,
                        Node::new(owner(lua)?, Kind::Symbol(Box::new(spec))),
                        Some(&table),
                    )?;
                    node_ud(lua, node)
                })?,
            )?;
            ui.set(
                "glass",
                lua.create_function(|lua, table: Table| {
                    let take = |name: &str, ty: Ty| -> mlua::Result<Option<Source>> {
                        let value = table.get::<mlua::Value>(name)?;
                        table.raw_set(name, mlua::Value::Nil)?;
                        match value {
                            mlua::Value::Nil => Ok(None),
                            value => source(lua, ty, name, value).map(Some),
                        }
                    };
                    let spec = GlassSpec {
                        variant: take("variant", Ty::Str)?,
                        tint: take("tint", Ty::Color)?,
                        interactive: take("interactive", Ty::Bool)?,
                        radius: take("radius", Ty::Num)?,
                    };
                    let node = fill(
                        lua,
                        Node::new(owner(lua)?, Kind::Glass(Box::new(spec))),
                        Some(&table),
                    )?;
                    node_ud(lua, node)
                })?,
            )?;
            ui.set(
                "glass_group",
                lua.create_function(|lua, table: Table| {
                    let spacing = table.get::<Option<f32>>("spacing")?.unwrap_or(20.);
                    table.raw_set("spacing", mlua::Value::Nil)?;
                    let node = fill(
                        lua,
                        Node::new(owner(lua)?, Kind::GlassGroup(spacing)),
                        Some(&table),
                    )?;
                    node_ud(lua, node)
                })?,
            )
        },
    );
}
