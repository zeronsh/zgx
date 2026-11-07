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
    let kit: Table = crate::lazy::builtin(lua, "=ui.ios", SOURCE)?
        .call((ui.clone(), cfg!(target_os = "ios")))?;
    ui.set("scroll_edge", kit.get::<Function>("scroll_edge")?)?;
    ui.set("keyboard_bar", kit.get::<Function>("keyboard_bar")?)?;
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

/// `ui.pull_refresh { element, top, on_refresh }`: the system's pull to refresh
/// on a scroll view. `top()` is where its spinner hangs; `on_refresh(done)`.
fn pull_refresh(lua: &Lua, table: Table) -> mlua::Result<()> {
    use zgx::native_control::{PullRefresh, set_pull_refresh};
    let element: mlua::UserDataRef<crate::element::ElementUd> = table.get("element")?;
    let Some(node) = element.0.read(|_, id| id) else {
        return Ok(());
    };
    let top: Function = table.get("top")?;
    let on_refresh: Function = table.get("on_refresh")?;
    // What ends a pull, made once: it calls what the latest pull left.
    let current: std::rc::Rc<std::cell::RefCell<Option<zgx::native_control::RefreshDone>>> =
        Default::default();
    let ender = current.clone();
    let done_fn = lua.create_function(move |_, ()| {
        if let Some(done) = ender.borrow_mut().take() {
            done();
        }
        Ok(())
    })?;
    set_pull_refresh(
        node,
        PullRefresh {
            top: std::rc::Rc::new(move || top.call::<f32>(()).unwrap_or(0.)),
            refresh: std::rc::Rc::new(move |done| {
                *current.borrow_mut() = Some(done);
                let _ = on_refresh.call::<()>(done_fn.clone());
            }),
        },
    );
    Ok(())
}

/// `ui.native_pan { edge = "left" | element = …, can_begin, on_pan }`: a pan the
/// platform recognizes. `on_pan` hears `{ phase, x, y, vx, vy }`.
fn native_pan(lua: &Lua, table: Table) -> mlua::Result<()> {
    use zgx::native_control::{PanEvent, PanPhase, PanTarget, add_pan};
    let node = match table.get::<Option<mlua::UserDataRef<crate::element::ElementUd>>>("element")? {
        Some(element) => match element.0.read(|_, id| id) {
            Some(node) => Some(node),
            None => return Ok(()),
        },
        None => None,
    };
    let can_begin: Option<Function> = table.get("can_begin")?;
    let on_pan: Function = table.get("on_pan")?;
    let told = lua.create_table()?;
    add_pan(PanTarget {
        node,
        can_begin: can_begin.map(|can| {
            std::rc::Rc::new(move || can.call::<bool>(()).unwrap_or(false))
                as std::rc::Rc<dyn Fn() -> bool>
        }),
        on_pan: std::rc::Rc::new(move |event: PanEvent| {
            let phase = match event.phase {
                PanPhase::Began => "began",
                PanPhase::Changed => "changed",
                PanPhase::Ended => "ended",
                PanPhase::Cancelled => "cancelled",
            };
            let _ = (|| -> mlua::Result<()> {
                told.raw_set("phase", phase)?;
                told.raw_set("x", event.x)?;
                told.raw_set("y", event.y)?;
                told.raw_set("vx", event.vx)?;
                told.raw_set("vy", event.vy)?;
                on_pan.call::<()>(told.clone())
            })();
        }),
    });
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
            "native_pan",
            "gesture_log",
            "share",
            "native_edge",
            "native_keyboard_bar",
            "pull_refresh",
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
            ui.set("native_pan", lua.create_function(native_pan)?)?;
            ui.set("pull_refresh", lua.create_function(pull_refresh)?)?;
            ui.set(
                "native_edge",
                lua.create_function(
                    |_,
                     (element, edge, hard): (
                        mlua::UserDataRef<crate::element::ElementUd>,
                        String,
                        bool,
                    )| {
                        use zgx::native_control::{Edge, EdgeEffect, request_edge_effect};
                        let edge = match edge.as_str() {
                            "bottom" => Edge::Bottom,
                            "left" => Edge::Left,
                            "right" => Edge::Right,
                            _ => Edge::Top,
                        };
                        if let Some(node) = element.0.read(|_, id| id) {
                            // It paints nothing of zgx's: a walk must still find it.
                            if let Some(scene) = element.0.scene() {
                                scene.borrow_mut().set_platform_painted(node, true);
                            }
                            request_edge_effect(node, EdgeEffect { edge, hard });
                        }
                        Ok(())
                    },
                )?,
            )?;
            ui.set(
                "native_keyboard_bar",
                lua.create_function(
                    |_, (element, lift): (mlua::UserDataRef<crate::element::ElementUd>, Function)| {
                        if let Some(node) = element.0.read(|_, id| id) {
                            if let Some(scene) = element.0.scene() {
                                scene.borrow_mut().set_platform_painted(node, true);
                            }
                            zgx::native_control::request_keyboard_bar(
                                zgx::native_control::KeyboardBar {
                                    node,
                                    lift: std::rc::Rc::new(move |up| {
                                        let _ = lift.call::<()>(up);
                                    }),
                                },
                            );
                        }
                        Ok(())
                    },
                )?,
            )?;
            ui.set(
                "gesture_log",
                lua.create_function(|_, line: String| {
                    if let Some(system) = zgx::native_control::system_ui() {
                        system.log_gesture(&line);
                    }
                    Ok(())
                })?,
            )?;
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
                        clear: take("clear", Ty::Bool)?,
                        variant: take("variant", Ty::Str)?,
                        tint: take("tint", Ty::Color)?,
                        interactive: take("interactive", Ty::Bool)?,
                        radius: take("radius", Ty::Num)?,
                        corners: take("corners", Ty::Corners)?,
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
