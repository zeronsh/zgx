//! How a window looks and sits, as `zgx.json` and `ui.window` say it: one
//! table of keys ([`LOOK_KEYS`]) and one parser for both, platform-neutral
//! (the host maps it onto its window's options), and the platform blocks
//! that override a window's keys.
//!
//! **Platform blocks.** A window's keys may differ by platform: `"macos"`,
//! `"windows"` and `"linux"` each hold window keys that replace the
//! window's own on that platform. Shallow, per key: a key a block has
//! replaces the window's whole (a table such as `glass` or
//! `traffic_lights` included, never merged into), and the keys it doesn't
//! have stay the window's. Only window keys go in a block (an app's keys,
//! such as `app_id`, are the same everywhere), and a block holds no other
//! block. Every block is checked, the other platforms' too, so a slip in
//! `"windows"` is told on a Mac. Any other key, a platform's name
//! misspelled or one zgx doesn't know (`"ios"`), is the unknown key it is.

/// The platforms a window's keys may differ by, as their blocks are named.
pub const PLATFORMS: [&str; 3] = ["macos", "windows", "linux"];

/// This build's platform, as its block is named.
pub fn this_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

/// A key's value as given, from JSON or a script's table.
#[derive(Clone, Debug, PartialEq)]
pub enum Given {
    /// `true` or `false`.
    Bool(bool),
    /// A number.
    Num(f64),
    /// A string.
    Str(String),
    /// A table of string keys.
    Map(Vec<(String, Given)>),
    /// Anything else, by what it is.
    Other(&'static str),
}
impl Given {
    fn field(&self, key: &str) -> Option<&Given> {
        match self {
            Given::Map(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}
impl From<&serde_json::Value> for Given {
    fn from(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Bool(b) => Given::Bool(*b),
            serde_json::Value::Number(n) => n.as_f64().map_or(Given::Other("number"), Given::Num),
            serde_json::Value::String(s) => Given::Str(s.clone()),
            serde_json::Value::Object(fields) => Given::Map(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), Given::from(v)))
                    .collect(),
            ),
            serde_json::Value::Array(_) => Given::Other("list"),
            serde_json::Value::Null => Given::Other("null"),
        }
    }
}
impl From<&mlua::Value> for Given {
    fn from(value: &mlua::Value) -> Self {
        match value {
            mlua::Value::Boolean(b) => Given::Bool(*b),
            mlua::Value::Integer(i) => Given::Num(*i as f64),
            mlua::Value::Number(n) => Given::Num(*n),
            mlua::Value::String(s) => Given::Str(s.to_string_lossy()),
            mlua::Value::Table(table) => Given::Map(
                table
                    .pairs::<mlua::Value, mlua::Value>()
                    .filter_map(Result::ok)
                    .filter_map(|(k, v)| match k {
                        mlua::Value::String(k) => Some((k.to_string_lossy(), Given::from(&v))),
                        _ => None,
                    })
                    .collect(),
            ),
            mlua::Value::Nil => Given::Other("nil"),
            _ => Given::Other("value"),
        }
    }
}

/// A glass's material, as the platform's blur behind the window is asked
/// for (macOS's `NSVisualEffectMaterial`; elsewhere the nearest the
/// platform has).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Material {
    /// `"hud"`: a dark, saturated heads-up-display glass.
    #[default]
    Hud,
    /// `"liquid"`: the system's newest glass (macOS 26's), else `hud`.
    Liquid,
    /// `"under_window_background"`: colourless, under a window's own background.
    UnderWindowBackground,
    /// `"under_page_background"`: behind a page's content.
    UnderPageBackground,
    /// `"window_background"`: a window's background.
    WindowBackground,
    /// `"content_background"`: the background of a window's content.
    ContentBackground,
    /// `"titlebar"`: a title bar's.
    Titlebar,
    /// `"header_view"`: a header view's.
    HeaderView,
    /// `"sidebar"`: a sidebar's.
    Sidebar,
    /// `"menu"`: a menu's.
    Menu,
    /// `"popover"`: a popover's.
    Popover,
    /// `"sheet"`: a sheet's.
    Sheet,
    /// `"tool_tip"`: a tooltip's.
    ToolTip,
    /// `"full_screen_ui"`: a full-screen window's controls.
    FullScreenUi,
    /// `"selection"`: the selection's highlight.
    Selection,
}
impl Material {
    /// Each by the name a key gives it.
    pub const NAMES: [(&'static str, Material); 15] = [
        ("hud", Material::Hud),
        ("liquid", Material::Liquid),
        ("under_window_background", Material::UnderWindowBackground),
        ("under_page_background", Material::UnderPageBackground),
        ("window_background", Material::WindowBackground),
        ("content_background", Material::ContentBackground),
        ("titlebar", Material::Titlebar),
        ("header_view", Material::HeaderView),
        ("sidebar", Material::Sidebar),
        ("menu", Material::Menu),
        ("popover", Material::Popover),
        ("sheet", Material::Sheet),
        ("tool_tip", Material::ToolTip),
        ("full_screen_ui", Material::FullScreenUi),
        ("selection", Material::Selection),
    ];
}

/// The desktop blurred behind the window (`glass`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Glass {
    /// The blur's material.
    pub material: Material,
    /// Only the blur: no tint or saturation of the platform's.
    pub plain: bool,
}

/// Where the window sits among others (`level`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Level {
    /// `"normal"`: among ordinary windows.
    #[default]
    Normal,
    /// Over every app's ordinary windows.
    AlwaysOnTop,
}

/// What [`LOOK_KEYS`] say of a window, each `None` where nothing was said
/// (the host's default stands).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct WindowLook {
    /// `glass`: `true` (the HUD material) or `{ material, plain }`; `false`
    /// or none, no glass.
    pub glass: Option<Glass>,
    /// `decorations`: the platform's own frame and title bar.
    pub decorations: Option<bool>,
    /// `resizable`.
    pub resizable: Option<bool>,
    /// `transparent`: see-through where nothing is drawn.
    pub transparent: Option<bool>,
    /// `shadow`: the platform's shadow around it.
    pub shadow: Option<bool>,
    /// `covers_display`: over the whole display.
    pub covers_display: Option<bool>,
    /// `level`.
    pub level: Option<Level>,
}

/// What a key of [`LOOK_KEYS`] holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `true` or `false`.
    Bool,
    /// `true`, `false` or `{ material, plain }`.
    Glass,
    /// `"normal"` or `"always_on_top"`.
    Level,
}

/// A key of a window's look.
#[derive(Clone, Copy, Debug)]
pub struct LookKey {
    /// Its name, as `zgx.json` and `ui.window` give it.
    pub name: &'static str,
    /// What it holds.
    pub kind: Kind,
    /// What it does, for the docs and the schema.
    pub doc: &'static str,
}

/// Every key of a window's look, `zgx.json`'s and `ui.window`'s alike.
pub const LOOK_KEYS: [LookKey; 7] = [
    LookKey {
        name: "glass",
        kind: Kind::Glass,
        doc: "The desktop blurred behind the window: true (the HUD material), false, or { material, plain }",
    },
    LookKey {
        name: "decorations",
        kind: Kind::Bool,
        doc: "The platform's own frame and title bar (true by default)",
    },
    LookKey {
        name: "resizable",
        kind: Kind::Bool,
        doc: "Whether the person can resize it (true by default)",
    },
    LookKey {
        name: "transparent",
        kind: Kind::Bool,
        doc: "See-through where nothing is drawn (false by default)",
    },
    LookKey {
        name: "shadow",
        kind: Kind::Bool,
        doc: "The platform's shadow around it (true by default)",
    },
    LookKey {
        name: "covers_display",
        kind: Kind::Bool,
        doc: "Over the whole display, the menu bar and dock included (false by default)",
    },
    LookKey {
        name: "level",
        kind: Kind::Level,
        doc: "Where it sits among windows: \"normal\" or \"always_on_top\"",
    },
];

impl WindowLook {
    /// The look `get` gives, each key read by its name (already resolved
    /// for the platform: the platform block's, else the window's own).
    pub fn parse(get: impl Fn(&str) -> Option<Given>) -> Result<WindowLook, String> {
        let mut look = WindowLook::default();
        for key in LOOK_KEYS {
            let Some(given) = get(key.name) else {
                continue;
            };
            let name = key.name;
            match key.kind {
                Kind::Bool => {
                    let Given::Bool(on) = given else {
                        return Err(format!("`{name}` is not a boolean"));
                    };
                    let field = match name {
                        "decorations" => &mut look.decorations,
                        "resizable" => &mut look.resizable,
                        "transparent" => &mut look.transparent,
                        "shadow" => &mut look.shadow,
                        "covers_display" => &mut look.covers_display,
                        _ => unreachable!("a boolean key of LOOK_KEYS"),
                    };
                    *field = Some(on);
                }
                Kind::Level => {
                    look.level = Some(match &given {
                        Given::Str(level) if level == "normal" => Level::Normal,
                        Given::Str(level) if level == "always_on_top" => Level::AlwaysOnTop,
                        Given::Str(level) => {
                            return Err(format!(
                                "`level` is `{level}`: one of normal, always_on_top"
                            ));
                        }
                        _ => return Err("`level` is not a string".into()),
                    });
                }
                Kind::Glass => look.glass = glass(&given)?,
            }
        }
        Ok(look)
    }
}

/// `glass` as given: true, false, or `{ material, plain }` (both
/// optional, nothing else).
fn glass(given: &Given) -> Result<Option<Glass>, String> {
    let fields = match given {
        Given::Bool(on) => return Ok(on.then(Glass::default)),
        Given::Map(fields) => fields,
        _ => return Err("`glass` is not a boolean or { material, plain }".into()),
    };
    if let Some((key, _)) = fields.iter().find(|(k, _)| k != "material" && k != "plain") {
        return Err(format!("`glass` has `{key}`: it takes material and plain"));
    }
    let material = match given.field("material") {
        None => Material::default(),
        Some(Given::Str(name)) => match Material::NAMES.iter().find(|(n, _)| n == name) {
            Some((_, material)) => *material,
            None => {
                let names: Vec<&str> = Material::NAMES.iter().map(|(n, _)| *n).collect();
                return Err(format!(
                    "`glass.material` is `{name}`: one of {}",
                    names.join(", ")
                ));
            }
        },
        Some(_) => return Err("`glass.material` is not a string".into()),
    };
    let plain = match given.field("plain") {
        None => false,
        Some(Given::Bool(plain)) => *plain,
        Some(_) => return Err("`glass.plain` is not a boolean".into()),
    };
    Ok(Some(Glass { material, plain }))
}

/// Whether the platform block `platform`'s keys (`keys`) are window keys
/// (`window_keys`): an error naming the first that isn't.
pub fn check_block<'k>(
    platform: &str,
    keys: impl IntoIterator<Item = &'k str>,
    window_keys: &[&str],
) -> Result<(), String> {
    for key in keys {
        if PLATFORMS.contains(&key) {
            return Err(format!(
                "`{platform}` holds `{key}`: a platform's block holds window keys, not another block"
            ));
        }
        if !window_keys.contains(&key) {
            return Err(format!(
                "`{platform}` has `{key}`, which isn't a window key that can differ by platform"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn map(fields: &[(&str, Given)]) -> Given {
        Given::Map(
            fields
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
        )
    }
    fn parse(fields: &[(&str, Given)]) -> Result<WindowLook, String> {
        let given = map(fields);
        WindowLook::parse(|key| given.field(key).cloned())
    }

    #[test]
    fn a_look_is_read_key_by_key() {
        assert_eq!(parse(&[]), Ok(WindowLook::default()));
        let look = parse(&[
            (
                "glass",
                map(&[
                    ("material", Given::Str("under_window_background".into())),
                    ("plain", Given::Bool(true)),
                ]),
            ),
            ("decorations", Given::Bool(false)),
            ("transparent", Given::Bool(true)),
            ("level", Given::Str("always_on_top".into())),
        ])
        .unwrap();
        assert_eq!(
            look.glass,
            Some(Glass {
                material: Material::UnderWindowBackground,
                plain: true
            })
        );
        assert_eq!(
            (look.decorations, look.transparent, look.resizable),
            (Some(false), Some(true), None)
        );
        assert_eq!(look.level, Some(Level::AlwaysOnTop));
        assert_eq!(
            parse(&[("glass", Given::Bool(true))]).unwrap().glass,
            Some(Glass::default())
        );
        assert_eq!(parse(&[("glass", Given::Bool(false))]).unwrap().glass, None);
        assert_eq!(
            parse(&[("glass", map(&[]))]).unwrap().glass,
            Some(Glass::default())
        );
    }

    #[test]
    fn a_wrong_value_says_what_it_should_be() {
        let wrong = |fields: &[(&str, Given)]| parse(fields).unwrap_err();
        assert_eq!(
            wrong(&[("shadow", Given::Num(1.))]),
            "`shadow` is not a boolean"
        );
        assert_eq!(
            wrong(&[("level", Given::Str("top".into()))]),
            "`level` is `top`: one of normal, always_on_top"
        );
        assert!(
            wrong(&[("glass", map(&[("material", Given::Str("frost".into()))]))])
                .starts_with("`glass.material` is `frost`: one of hud, liquid,")
        );
        assert_eq!(
            wrong(&[("glass", map(&[("blur", Given::Num(3.))]))]),
            "`glass` has `blur`: it takes material and plain"
        );
        assert_eq!(
            wrong(&[("glass", Given::Str("yes".into()))]),
            "`glass` is not a boolean or { material, plain }"
        );
    }

    #[test]
    fn a_platform_block_holds_window_keys_only() {
        let window = ["title", "glass", "decorations"];
        assert_eq!(
            check_block("macos", ["glass", "decorations"], &window),
            Ok(())
        );
        assert_eq!(
            check_block("macos", ["app_id"], &window).unwrap_err(),
            "`macos` has `app_id`, which isn't a window key that can differ by platform"
        );
        assert_eq!(
            check_block("windows", ["linux"], &window).unwrap_err(),
            "`windows` holds `linux`: a platform's block holds window keys, not another block"
        );
    }
}
