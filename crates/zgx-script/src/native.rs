//! `ui.glass`, `ui.glass_group`, `ui.symbol`, `ui.native_switch` and
//! `ui.native_tabs`: the system's own material and controls where it has them,
//! made the first time a script names one.
use crate::node::{
    GlassSpec, Kind, Node, Source, SymbolSpec, Ty, control_value, fill, node_ud, owner, source,
};
use mlua::{Function, Lua, Table};
use zgx::native_control::{AlertButtonStyle, AlertStyle};

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
/// `ui.ios`, the kit of `ios_kit.luau`, made the first time a script names it.
pub(crate) fn install_kit(lua: &Lua, ui: &Table) -> mlua::Result<()> {
    const SOURCE: &str = include_str!("ios_kit.luau");
    let kit: Table = crate::lazy::builtin(lua, "=ui.ios", SOURCE)?.call(ui.clone())?;
    ui.set("ios", kit)
}

/// What `ui.alert` and `ui.action_sheet` take: `title`, `message`, `buttons`
/// (`{ title, style }`), `on_choose`.
fn present_alert(table: Table, style: AlertStyle) -> mlua::Result<()> {
    let title: String = table.get::<Option<String>>("title")?.unwrap_or_default();
    let message: String = table.get::<Option<String>>("message")?.unwrap_or_default();
    let on_choose: Option<Function> = table.get("on_choose")?;
    let mut buttons = Vec::new();
    let listed: Table = table.get("buttons")?;
    for button in listed.sequence_values::<Table>() {
        let button = button?;
        let kind = match button.get::<Option<String>>("style")?.as_deref() {
            Some("cancel") => AlertButtonStyle::Cancel,
            Some("destructive") => AlertButtonStyle::Destructive,
            _ => AlertButtonStyle::Default,
        };
        buttons.push((button.get::<String>("title")?, kind));
    }
    let Some(system) = zgx::native_control::system_ui() else {
        return Ok(());
    };
    system.alert(
        style,
        &title,
        &message,
        &buttons,
        Box::new(move |index| {
            if let Some(on_choose) = on_choose {
                let _ = on_choose.call::<()>(index + 1);
            }
        }),
    );
    Ok(())
}

/// `ui.system_menu(element, build)`: `build` gives `{ title, symbol, style, on_choose }`s.
fn system_menu(
    _: &Lua,
    (element, build): (mlua::UserDataRef<crate::element::ElementUd>, Function),
) -> mlua::Result<()> {
    let Some(node) = element.0.read(|_, id| id) else {
        return Ok(());
    };
    zgx::native_control::set_system_menu(
        node,
        std::rc::Rc::new(move || {
            let mut actions = Vec::new();
            let Ok(listed) = build.call::<Table>(()) else {
                return actions;
            };
            for item in listed.sequence_values::<Table>().flatten() {
                let Ok(title) = item.get::<String>("title") else {
                    continue;
                };
                let on_choose = item.get::<Option<Function>>("on_choose").ok().flatten();
                actions.push(zgx::native_control::MenuAction {
                    title,
                    symbol: item.get("symbol").ok().flatten(),
                    destructive: item
                        .get::<Option<String>>("style")
                        .ok()
                        .flatten()
                        .is_some_and(|style| style == "destructive"),
                    run: std::rc::Rc::new(move || {
                        if let Some(on_choose) = &on_choose {
                            let _ = on_choose.call::<()>(());
                        }
                    }),
                });
            }
            actions
        }),
    );
    Ok(())
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
            "haptic",
            "alert",
            "system_menu",
            "on_escape",
            "share",
            "action_sheet",
        ],
        |lua, ui| {
            ui.set(
                "haptic",
                lua.create_function(|_, kind: String| {
                    if let Some(system) = zgx::native_control::system_ui() {
                        system.haptic(&kind);
                    }
                    Ok(())
                })?,
            )?;
            ui.set(
                "alert",
                lua.create_function(|_, table: Table| present_alert(table, AlertStyle::Alert))?,
            )?;
            ui.set(
                "action_sheet",
                lua.create_function(|_, table: Table| {
                    present_alert(table, AlertStyle::ActionSheet)
                })?,
            )?;
            ui.set("system_menu", lua.create_function(system_menu)?)?;
            ui.set(
                "share",
                lua.create_function(|_, table: Table| {
                    let text: Option<String> = table.get("text")?;
                    let url: Option<String> = table.get("url")?;
                    let files: Vec<String> = table
                        .get::<Option<Table>>("files")?
                        .map(|files| files.sequence_values().collect())
                        .transpose()?
                        .unwrap_or_default();
                    if let Some(system) = zgx::native_control::system_ui() {
                        system.share(text.as_deref(), url.as_deref(), &files);
                    }
                    Ok(())
                })?,
            )?;
            ui.set(
                "on_escape",
                lua.create_function(|_, handler: Function| {
                    zgx::native_control::on_escape(std::rc::Rc::new(move || {
                        handler.call::<bool>(()).unwrap_or(false)
                    }));
                    Ok(())
                })?,
            )?;
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
