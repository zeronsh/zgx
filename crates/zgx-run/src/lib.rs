//! Running a zgx Luau script as a desktop app: its commands (`ui.command`)
//! as the native menu bar (`menus`), and a whole app from a script and an
//! optional `zgx.json` beside it (`run`), as the `zgx-run` binary does.
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use zgx::input::{Key, Modifiers};
use zgx_desktop::{AppMenu, AppMenuEntry, Application, MenuAccelerator, WindowOptions};
use zgx_script::{Command, ScriptHost};

pub mod audit;
#[cfg(feature = "fetch")]
mod fetch;
pub mod schema;
/// Give `host` the fetcher `run` gives a script (feature `fetch`, on by
/// default): `ui.fetch` and remote images over HTTP and HTTPS, each request
/// on a thread of its own, its body streamed to the script as it arrives,
/// a cancelled one stopped. For a host made outside `run`.
#[cfg(feature = "fetch")]
pub fn install_fetcher(host: &ScriptHost) {
    fetch::install(host);
}

#[cfg(feature = "socket")]
mod socket;
/// Give `host` the connector `run` gives a script (feature `socket`, on by
/// default): `ui.socket` over WebSockets, `ws://` and `wss://`, each
/// connection on a thread of its own. For a host made outside `run`.
#[cfg(feature = "socket")]
pub fn install_connector(host: &ScriptHost) {
    socket::install(host);
}

/// Menus in the order the script first names them, items in its order.
pub fn menus(commands: &[Command]) -> Vec<AppMenu> {
    let mut menus: Vec<AppMenu> = Vec::new();
    for command in commands {
        let index = match menus.iter().position(|m| m.label == command.menu) {
            Some(index) => index,
            None => {
                menus.push(AppMenu::new(command.menu.clone(), []));
                menus.len() - 1
            }
        };
        let entry = if command.separator {
            AppMenuEntry::Separator
        } else {
            let item = AppMenuEntry::item(command.title.clone(), command.id.as_str());
            match command.key.as_deref().and_then(accelerator) {
                Some(key) => item.accelerator(key),
                None => item,
            }
        };
        menus[index].entries.push(entry);
    }
    menus
}

/// `cmd+shift+d`, `cmd+alt+arrowleft`, `cmd+1`, `cmd+[` as an accelerator.
/// `cmd` is the platform's command key, as menus' shortcuts use it: Command
/// on macOS, Control elsewhere (`super` and `meta` name the key itself).
pub fn accelerator(text: &str) -> Option<MenuAccelerator> {
    let mut modifiers = Modifiers::default();
    let mut parts: Vec<&str> = text.split('+').collect();
    // `cmd++` would be a plus key; none of ours use it.
    let key = parts.pop()?.to_ascii_lowercase();
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" if cfg!(target_os = "macos") => modifiers.meta = true,
            "cmd" | "command" => modifiers.control = true,
            "super" | "meta" => modifiers.meta = true,
            "ctrl" | "control" => modifiers.control = true,
            "alt" | "opt" | "option" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            other => {
                eprintln!("menus: unknown modifier `{other}` in `{text}`");
                return None;
            }
        }
    }
    let key = match key.as_str() {
        "arrowleft" | "left" => Key::ArrowLeft,
        "arrowright" | "right" => Key::ArrowRight,
        "arrowup" | "up" => Key::ArrowUp,
        "arrowdown" | "down" => Key::ArrowDown,
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "escape" | "esc" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "space" => Key::Space,
        f if f.len() > 1 && f.starts_with('f') && f[1..].parse::<u8>().is_ok() => {
            Key::Function(f[1..].parse().ok()?)
        }
        single if single.chars().count() == 1 => Key::Character(single.into()),
        other => {
            eprintln!("menus: unknown key `{other}` in `{text}`");
            return None;
        }
    };
    Some(MenuAccelerator::new(key, modifiers))
}

/// The window and app a script runs in (`zgx.json` beside the script):
/// every field optional.
///
/// ```json
/// { "title": "Mail", "width": 1100, "height": 720, "min_width": 640,
///   "min_height": 400, "titlebar": "inset", "traffic_lights": { "x": 18, "y": 22 },
///   "glass": true, "app_id": "mail" }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct AppConfig {
    pub title: String,
    pub width: f64,
    pub height: f64,
    pub min_size: Option<(f64, f64)>,
    /// `"standard"`, `"inset"` (content under a transparent title bar) or
    /// `"hidden"`.
    pub titlebar: zgx_desktop::Titlebar,
    pub glass: bool,
    /// Where `ui.persist` keeps its data (the app's data directory).
    pub app_id: String,
    /// A reverse-DNS application ID (`"dev.zgx.code"`): the system then
    /// hands the app the files and URLs it opens with it (`ui.on_open`),
    /// and on Windows a second launch's go to the first.
    pub application_id: Option<String>,
}
/// What zgx.json says of the window beyond [`AppConfig`]'s fields, read by
/// [`AppConfig::read`] and handed to the window's options: kept out of
/// `AppConfig`, a struct Rust hosts build by literal.
#[derive(Debug)]
struct WindowConfig {
    /// The name its frame is saved under (`frame_autosave`: `true` is the
    /// `app_id`, so apps zgx-run runs keep theirs apart; a string, that
    /// name).
    frame_autosave: Option<String>,
    /// False: it opens hidden, laid out and drawn, until the script shows it
    /// (`ui.window.visible:set(true)`): an app showing an intro of its own
    /// first.
    visible: bool,
    /// Its largest size in points (`max_width` and `max_height`): the
    /// same as `min_width` and `min_height`, a window of fixed size that a
    /// window manager can still move.
    max_size: Option<(f64, f64)>,
    /// How tall `titlebar: "inset"`'s caption buttons are on Windows, in the
    /// content's points (`caption_height`; `WindowOptions::caption_height`).
    caption_height: Option<f64>,
    /// Where `titlebar: "inset"`'s traffic lights go on macOS: the close
    /// button's top left in the content's points (`traffic_lights`,
    /// `{ "x": 18, "y": 22 }`; `WindowOptions::traffic_lights`).
    traffic_lights: Option<(f64, f64)>,
    /// Its glass, frame, level and the rest
    /// (`zgx_script::window_look::LOOK_KEYS`).
    look: zgx_script::window_look::WindowLook,
    /// The script a named window opens (`named_windows.<name>.script`), beside
    /// the app's; none, the app's own.
    script: Option<PathBuf>,
}

/// `zgx.json`'s window keys: those a platform's block (`"macos"`,
/// `"windows"`, `"linux"`) may hold, besides the look's
/// (`zgx_script::window_look::LOOK_KEYS`).
pub(crate) const WINDOW_KEYS: [&str; 12] = [
    "title",
    "width",
    "height",
    "min_width",
    "max_width",
    "max_height",
    "min_height",
    "titlebar",
    "caption_height",
    "traffic_lights",
    "frame_autosave",
    "visible",
];

/// `zgx.json`'s keys of the app as a whole, besides its window's (and
/// `zgx_script::app_config::KEYS`, its host's): none of them may differ by
/// platform.
pub(crate) const APP_KEYS: [&str; 5] = [
    "$schema",
    "app_id",
    "application_id",
    "named_windows",
    "first",
];

/// The window's options, as `config` and `window` say them.
fn options_of(config: &AppConfig, window: WindowConfig) -> WindowOptions {
    let WindowConfig {
        look,
        frame_autosave,
        visible,
        max_size,
        caption_height,
        traffic_lights,
        script: _,
    } = window;
    let mut options = WindowOptions {
        title: config.title.clone(),
        width: config.width,
        height: config.height,
        min_size: config.min_size,
        max_size,
        titlebar: config.titlebar,
        caption_height,
        traffic_lights,
        glass: config.glass.then(zgx_desktop::Glass::default),
        frame_autosave,
        visible,
        ..Default::default()
    };
    apply_look(&mut options, &look);
    options
}

/// The options of the window `window` names among the `named_windows` of
/// a bundle's `zgx.json` (none: the one its `first` names, else its top
/// level's), on this platform, as zgx-run opens it, and the script that
/// window opens, if one of its own: for a Rust host that opens its own
/// windows from what its app's `zgx.json` says. The file is checked as
/// the bundle's others are: changed or removed since it was built, it is
/// an error. So is what the file says wrong, or a name it doesn't have.
/// (Hidden from the docs until this Rust interface is settled.)
#[doc(hidden)]
pub fn window_options_in(
    bundle: &zgx_script::app_bundle::Opened,
    window: Option<&str>,
) -> Result<(WindowOptions, Option<PathBuf>), String> {
    let (config, _, mut window) = AppConfig::read_bundle(bundle, window)?;
    let own = window.script.take();
    Ok((
        options_of(&config, window),
        own.map(|own| bundle.ui.join(own)),
    ))
}

/// [`window_options_in`] for an app run from its folder.
#[doc(hidden)]
pub fn window_options(
    script: &Path,
    window: Option<&str>,
) -> Result<(WindowOptions, Option<PathBuf>), String> {
    let (config, _, mut window) = AppConfig::read_window(script, window)?;
    let own = window.script.take();
    let dir = script.parent().unwrap_or(Path::new("."));
    Ok((options_of(&config, window), own.map(|own| dir.join(own))))
}

/// `look` laid onto `options`: what it says replaces the option's default.
fn apply_look(options: &mut WindowOptions, look: &zgx_script::window_look::WindowLook) {
    use zgx_script::window_look::{Level, Material};
    if let Some(glass) = look.glass {
        let material = match glass.material {
            Material::Hud => zgx_desktop::GlassMaterial::Hud,
            Material::Liquid => zgx_desktop::GlassMaterial::Liquid,
            Material::UnderWindowBackground => zgx_desktop::GlassMaterial::UnderWindowBackground,
            Material::UnderPageBackground => zgx_desktop::GlassMaterial::UnderPageBackground,
            Material::WindowBackground => zgx_desktop::GlassMaterial::WindowBackground,
            Material::ContentBackground => zgx_desktop::GlassMaterial::ContentBackground,
            Material::Titlebar => zgx_desktop::GlassMaterial::Titlebar,
            Material::HeaderView => zgx_desktop::GlassMaterial::HeaderView,
            Material::Sidebar => zgx_desktop::GlassMaterial::Sidebar,
            Material::Menu => zgx_desktop::GlassMaterial::Menu,
            Material::Popover => zgx_desktop::GlassMaterial::Popover,
            Material::Sheet => zgx_desktop::GlassMaterial::Sheet,
            Material::ToolTip => zgx_desktop::GlassMaterial::ToolTip,
            Material::FullScreenUi => zgx_desktop::GlassMaterial::FullScreenUi,
            Material::Selection => zgx_desktop::GlassMaterial::Selection,
        };
        let made = zgx_desktop::Glass::new(material);
        options.glass = Some(if glass.plain { made.plain() } else { made });
    }
    let set = |option: &mut bool, given: Option<bool>| {
        if let Some(given) = given {
            *option = given;
        }
    };
    set(&mut options.decorations, look.decorations);
    set(&mut options.resizable, look.resizable);
    set(&mut options.transparent, look.transparent);
    set(&mut options.shadow, look.shadow);
    set(&mut options.covers_display, look.covers_display);
    if let Some(level) = look.level {
        options.level = match level {
            Level::Normal => zgx_desktop::WindowLevel::Normal,
            Level::AlwaysOnTop => zgx_desktop::WindowLevel::AlwaysOnTop,
        };
    }
}
impl AppConfig {
    /// The config for the script at `script`: its directory's `zgx.json`
    /// (or `zgui.json`, deprecated), or the defaults (named after the directory). rename-to-zgx: keep
    pub fn for_script(script: &Path) -> Result<Self, String> {
        Self::read(script).map(|(config, _, _)| config)
    }
    /// As [`for_script`](Self::for_script), with what it gives the script's
    /// host (`zgx_script::app_config`: its packages and the scripts it loads
    /// beside its own), and what it says of the window besides.
    fn read(
        script: &Path,
    ) -> Result<(Self, zgx_script::app_config::AppHost, WindowConfig), String> {
        Self::read_window(script, None)
    }
    /// As [`read`](Self::read), for the window `window` names among
    /// `zgx.json`'s `named_windows` (none: the one `first` names, else the
    /// top level's).
    fn read_window(
        script: &Path,
        window: Option<&str>,
    ) -> Result<(Self, zgx_script::app_config::AppHost, WindowConfig), String> {
        Self::read_for(script, zgx_script::window_look::this_platform(), window)
    }
    /// As [`read_window`](Self::read_window), on `platform`. Every window
    /// it names is read on every platform too, so what one says wrong is
    /// told wherever the app runs.
    fn read_for(
        script: &Path,
        platform: &str,
        window: Option<&str>,
    ) -> Result<(Self, zgx_script::app_config::AppHost, WindowConfig), String> {
        let dir = script.parent().unwrap_or(Path::new("."));
        // Or, for one release, the name it had before the rename to zgx.
        let old = dir.join("zgui.json"); // rename-to-zgx: keep
        let mut path = dir.join("zgx.json");
        if !path.exists() && old.exists() {
            eprintln!("zgx: {} is deprecated; name it zgx.json", old.display());
            path = old;
        }
        let text = std::fs::read_to_string(&path).ok();
        Self::read_text(dir, &path, text, platform, window)
    }
    /// As [`read_window`](Self::read_window), a bundle's `zgx.json` read
    /// through its checked files: one its manifest doesn't name is none,
    /// and one changed or removed since it was built is refused.
    fn read_bundle(
        bundle: &zgx_script::app_bundle::Opened,
        window: Option<&str>,
    ) -> Result<(Self, zgx_script::app_config::AppHost, WindowConfig), String> {
        let path = bundle.ui.join("zgx.json");
        let text = match bundle.file("zgx.json")? {
            None => None,
            Some(bytes) => {
                Some(String::from_utf8(bytes).map_err(|e| format!("{}: {e}", path.display()))?)
            }
        };
        let platform = zgx_script::window_look::this_platform();
        let read = Self::read_text(&bundle.ui, &path, text, platform, window)?;
        // A bundle of source's scripts are its own files: none by a link or
        // a path out of its `ui/`. (Checked files are refused as they are
        // read.)
        if bundle.kind() == zgx_script::app_bundle::Ui::Source {
            let (_, host, window) = &read;
            let scripts = window.script.iter().map(|own| bundle.ui.join(own));
            let packages = host.packages.iter().map(|(_, folder)| folder.clone());
            if let Some(out) = scripts.chain(packages).find(|path| !bundle.holds(path)) {
                return Err(format!("{}: outside its bundle's ui/", out.display()));
            }
        }
        Ok(read)
    }
    /// The window `window` names of the `zgx.json` `text` (none: there is
    /// none), read from `path` in `dir`, on `platform`.
    fn read_text(
        dir: &Path,
        path: &Path,
        text: Option<String>,
        platform: &str,
        window: Option<&str>,
    ) -> Result<(Self, zgx_script::app_config::AppHost, WindowConfig), String> {
        use zgx_script::window_look::PLATFORMS;
        let name = dir
            .canonicalize()
            .ok()
            .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "app".into());
        let Some(text) = text else {
            if let Some(window) = window {
                return Err(format!(
                    "no {} names a window `{window}` (`named_windows`)",
                    path.display()
                ));
            }
            let empty = serde_json::Map::new();
            let (config, window) = Self::resolve(&name, path, &empty, platform, None)?;
            return Ok((config, Default::default(), window));
        };
        let fail = |e: String| format!("{}: {e}", path.display());
        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| fail(e.to_string()))?;
        let object = json
            .as_object()
            .ok_or_else(|| fail("not an object".into()))?;
        // Its named windows, and which opens first.
        // (`named_windows`, not `windows`: that is Windows' platform block.)
        let windows = match object.get("named_windows") {
            None => serde_json::Map::new(),
            Some(serde_json::Value::Object(windows)) => {
                for (name, window) in windows {
                    if !window.is_object() {
                        return Err(fail(format!(
                            "`named_windows.{name}` is not an object of window keys"
                        )));
                    }
                }
                windows.clone()
            }
            Some(_) => {
                return Err(fail(
                    "`named_windows` is not an object of windows by name".into(),
                ));
            }
        };
        let names = || {
            let names: Vec<&str> = windows.keys().map(String::as_str).collect();
            if names.is_empty() {
                "none".to_owned()
            } else {
                names.join(", ")
            }
        };
        let first = match object.get("first") {
            None => None,
            Some(serde_json::Value::String(first)) if windows.contains_key(first) => {
                Some(first.as_str())
            }
            Some(serde_json::Value::String(first)) => {
                return Err(fail(format!(
                    "`first` is `{first}`, which `named_windows` doesn't name (it names {})",
                    names()
                )));
            }
            Some(_) => return Err(fail("`first` is not a window's name".into())),
        };
        let chosen = match window {
            Some(window) if !windows.contains_key(window) => {
                return Err(fail(format!(
                    "no window `{window}`: `named_windows` names {}",
                    names()
                )));
            }
            Some(window) => Some(window),
            None => first,
        };
        // Every window, on every platform: what any says wrong is told
        // here, wherever this runs.
        for named in std::iter::once(None).chain(windows.keys().map(|name| Some(name.as_str()))) {
            for other in PLATFORMS {
                let window = named.and_then(|name| windows.get(name)?.as_object());
                Self::resolve(&name, path, object, other, window).map_err(|e| match named {
                    Some(named) => format!("{e} (window `{named}`, on {other})"),
                    None => format!("{e} (on {other})"),
                })?;
            }
        }
        let named = chosen.and_then(|name| windows.get(name)?.as_object());
        let (config, window) = Self::resolve(&name, path, object, platform, named)?;
        let app = zgx_script::app_config::from_object(dir, object).map_err(fail)?;
        Ok((config, app, window))
    }
    /// The window `object` (a `zgx.json`) says on `platform`: `named`, one
    /// of its `named_windows`, if given. Each window key is read from the first of
    /// its layers that has it, the most specific first: the named window's
    /// platform block, the named window, the top level's platform block,
    /// the top level (shallow: a table such as `glass` is the layer's whole).
    fn resolve(
        name: &str,
        path: &Path,
        object: &serde_json::Map<String, serde_json::Value>,
        platform: &str,
        named: Option<&serde_json::Map<String, serde_json::Value>>,
    ) -> Result<(Self, WindowConfig), String> {
        use zgx_script::window_look::{Given, LOOK_KEYS, PLATFORMS, WindowLook, check_block};
        let mut config = AppConfig {
            title: name.to_owned(),
            width: 960.,
            height: 640.,
            min_size: None,
            titlebar: zgx_desktop::Titlebar::Standard,
            glass: false,
            app_id: name.to_owned(),
            application_id: None,
        };
        let fail = |e: String| format!("{}: {e}", path.display());
        // The window's keys a platform's block may hold; every block is
        // checked, this platform's or not.
        let window_keys: Vec<&str> = WINDOW_KEYS
            .iter()
            .copied()
            .chain(LOOK_KEYS.iter().map(|key| key.name))
            .collect();
        let blocks_of =
            |within: &serde_json::Map<String, serde_json::Value>, at: &str| -> Result<(), String> {
                for name in PLATFORMS {
                    match within.get(name) {
                        None => {}
                        Some(serde_json::Value::Object(block)) => {
                            check_block(name, block.keys().map(String::as_str), &window_keys)
                                .map_err(|e| fail(format!("{at}{e}")))?
                        }
                        Some(_) => {
                            return Err(fail(format!(
                                "{at}`{name}` is not an object of window keys"
                            )));
                        }
                    }
                }
                Ok(())
            };
        blocks_of(object, "")?;
        for key in object.keys() {
            if !window_keys.contains(&key.as_str())
                && !APP_KEYS.contains(&key.as_str())
                && !PLATFORMS.contains(&key.as_str())
                && !zgx_script::app_config::KEYS.contains(&key.as_str())
            {
                return Err(fail(format!("unknown key `{key}`")));
            }
        }
        if let Some(named) = named {
            blocks_of(named, "a named window's ")?;
            for key in named.keys() {
                if !window_keys.contains(&key.as_str())
                    && !PLATFORMS.contains(&key.as_str())
                    && key != "script"
                {
                    return Err(fail(format!(
                        "a named window has `{key}`, which isn't a window key"
                    )));
                }
            }
        }
        fn block_of<'a>(
            within: &'a serde_json::Map<String, serde_json::Value>,
            platform: &str,
        ) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
            within.get(platform).and_then(serde_json::Value::as_object)
        }
        let mut layers: Vec<&serde_json::Map<String, serde_json::Value>> = Vec::with_capacity(4);
        if let Some(named) = named {
            layers.extend(block_of(named, platform));
            layers.push(named);
        }
        layers.extend(block_of(object, platform));
        layers.push(object);
        let get = |key: &str| layers.iter().find_map(|layer| layer.get(key));
        let number = |key: &str| -> Result<Option<f64>, String> {
            match get(key) {
                None => Ok(None),
                Some(v) => v
                    .as_f64()
                    .map(Some)
                    .ok_or_else(|| fail(format!("`{key}` is not a number"))),
            }
        };
        let string = |key: &str| -> Result<Option<String>, String> {
            match get(key) {
                None => Ok(None),
                Some(v) => v
                    .as_str()
                    .map(|s| Some(s.to_owned()))
                    .ok_or_else(|| fail(format!("`{key}` is not a string"))),
            }
        };
        if let Some(title) = string("title")? {
            config.title = title;
        }
        config.width = number("width")?.unwrap_or(config.width);
        config.height = number("height")?.unwrap_or(config.height);
        if let (Some(w), Some(h)) = (number("min_width")?, number("min_height")?) {
            config.min_size = Some((w, h));
        } else if number("min_width")?.is_some() || number("min_height")?.is_some() {
            return Err(fail("`min_width` and `min_height` go together".into()));
        }
        let max_size = match (number("max_width")?, number("max_height")?) {
            (Some(w), Some(h)) => Some((w, h)),
            (None, None) => None,
            _ => return Err(fail("`max_width` and `max_height` go together".into())),
        };
        let caption_height = match number("caption_height")? {
            Some(height) if !(height.is_finite() && height > 0.) => {
                return Err(fail(format!("`caption_height` is {height}: not positive")));
            }
            height => height,
        };
        // `{ "x": 18, "y": 22 }`: the close button's top left.
        let traffic_lights = match get("traffic_lights") {
            None => None,
            Some(value) => {
                let at = value.as_object().filter(|at| at.len() == 2);
                let coordinate = |key: &str| {
                    at.and_then(|at| at.get(key)?.as_f64())
                        .filter(|v| v.is_finite())
                };
                match (coordinate("x"), coordinate("y")) {
                    (Some(x), Some(y)) => Some((x, y)),
                    _ => {
                        return Err(fail(
                            "`traffic_lights` is not `{ \"x\": <points>, \"y\": <points> }`".into(),
                        ));
                    }
                }
            }
        };
        config.titlebar = match string("titlebar")?.as_deref() {
            None | Some("standard") => zgx_desktop::Titlebar::Standard,
            Some("inset") => zgx_desktop::Titlebar::Inset,
            Some("hidden") => zgx_desktop::Titlebar::Hidden,
            Some(other) => {
                return Err(fail(format!(
                    "`titlebar` is `{other}`: one of standard, inset, hidden"
                )));
            }
        };
        let look = WindowLook::parse(|key| get(key).map(Given::from)).map_err(fail)?;
        config.glass = look.glass.is_some();
        let visible = match get("visible") {
            None => true,
            Some(v) => v
                .as_bool()
                .ok_or_else(|| fail("`visible` is not a boolean".into()))?,
        };
        if let Some(id) = string("app_id")? {
            config.app_id = id;
        }
        config.application_id = string("application_id")?;
        let frame_autosave = match get("frame_autosave") {
            None | Some(serde_json::Value::Bool(false)) => None,
            Some(serde_json::Value::Bool(true)) => Some(config.app_id.clone()),
            Some(serde_json::Value::String(name)) if !name.is_empty() => Some(name.clone()),
            Some(_) => {
                return Err(fail("`frame_autosave` is not true, false or a name".into()));
            }
        };
        let script = match named.and_then(|named| named.get("script")) {
            None => None,
            Some(serde_json::Value::String(script)) if !script.is_empty() => {
                Some(PathBuf::from(script))
            }
            Some(_) => {
                return Err(fail(
                    "a named window's `script` is not a file's name".into(),
                ));
            }
        };
        Ok((
            config,
            WindowConfig {
                look,
                frame_autosave,
                visible,
                max_size,
                caption_height,
                traffic_lights,
                script,
            },
        ))
    }
}

/// The script `path` names: a file, or a directory's `app.luau`.
pub fn script_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join("app.luau")
    } else {
        path.to_path_buf()
    }
}

/// What `zgx run` hands the runtime: each module's library by name, built
/// and cached for this run (`--module words=<library>`), in place of where
/// zgx.json's `modules` would find it; and a line to show as the app
/// starts (`--notice`: "module `words` changed: restarted").
#[derive(Default)]
struct Launch {
    modules: Vec<(String, PathBuf)>,
    notice: Option<String>,
    window: Option<String>,
}
static LAUNCH: std::sync::Mutex<Launch> = std::sync::Mutex::new(Launch {
    modules: Vec::new(),
    notice: None,
    window: None,
});

/// The window to open first, by its name among zgx.json's `named_windows`
/// (`zgx-run --window <name>`): set before [`run`]. One it doesn't name is
/// an error that lists those it does.
pub fn set_window(name: impl Into<String>) {
    LAUNCH.lock().unwrap().window = Some(name.into());
}

/// The library to load for the module `name` instead of the one zgx.json's
/// `modules` names (as `zgx run` builds it): set before [`run`].
pub fn set_module(name: impl Into<String>, library: impl Into<PathBuf>) {
    let mut launch = LAUNCH.lock().unwrap();
    let name = name.into();
    launch.modules.retain(|(n, _)| *n != name);
    launch.modules.push((name, library.into()));
}

/// A line the app shows in its banner once its scripts have loaded, and on
/// stderr: what `zgx run` says of why it started it again.
pub fn set_notice(notice: impl Into<String>) {
    LAUNCH.lock().unwrap().notice = Some(notice.into());
}

/// The native modules the app at `dir` loads, by name: each library `zgx
/// run` handed over, else the one zgx.json's `modules` names (a library's
/// path, or a crate whose newest build is taken). A module zgx.json names
/// with no library to load is an error: its scripts would call it.
#[cfg(feature = "modules")]
fn modules_of(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let given = std::mem::take(&mut LAUNCH.lock().unwrap().modules);
    let named = zgx_script::module_types::named_in(dir)?;
    let (found, missing) = zgx_script::module_types::of_app(dir);
    let mut modules = Vec::new();
    for name in named {
        if let Some((_, library)) = given.iter().find(|(n, _)| *n == name) {
            modules.push((name, library.clone()));
            continue;
        }
        let library = found
            .iter()
            .find(|(_, manifest)| {
                zgx_script::module_types::name_of(manifest).as_deref() == Some(name.as_str())
            })
            .map(|(library, _)| library.clone());
        match library {
            Some(library) => modules.push((name, library)),
            None => {
                let why = missing
                    .iter()
                    .find(|why| why.starts_with(&format!("module `{name}`")))
                    .cloned()
                    .unwrap_or_else(|| format!("module `{name}`: no library to load"));
                return Err(why);
            }
        }
    }
    // One `zgx run` built that zgx.json doesn't name is loaded too.
    for (name, library) in given {
        if !modules.iter().any(|(n, _)| *n == name) {
            modules.push((name, library));
        }
    }
    Ok(modules)
}

/// Run the script at `path` (a file, or a directory with `app.luau`) as an
/// app, until its window closes: a window from its `zgx.json`, its
/// commands as the menu bar, hot reload as its files change, and
/// `ui.persist` saved as it quits.
pub fn run(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    run_with(path, Vec::new())
}

/// The local path a `file://` URL names (`None` for another URL).
fn file_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let mut bytes = Vec::with_capacity(rest.len());
    let mut input = rest.bytes();
    while let Some(b) = input.next() {
        if b == b'%' {
            let hex = |c: u8| (c as char).to_digit(16);
            let (a, b) = (hex(input.next()?)?, hex(input.next()?)?);
            bytes.push((a * 16 + b) as u8);
        } else {
            bytes.push(b);
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    // `file:///C:/a.txt` on Windows.
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if cfg!(windows) && drive.is_ascii_alphabetic() => &path[1..],
        _ => &path,
    };
    Some(PathBuf::from(path))
}

/// As [`run`], with `args` after the script: the script's `ui.args`, and
/// those that name files opened with it (its `ui.on_open`), as a file
/// opened with an app is.
pub fn run_with(path: &Path, args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    run_hosting(path, args, |_| {})
}

/// As [`run_with`], `host` given the script's host before it loads: an app
/// that serves its script (a fetcher, actions, shared state) keeps
/// everything `run` gives it, the menu bar of its commands included. A
/// fetcher it sets replaces the default one (`install_fetcher`).
pub fn run_hosting(
    path: &Path,
    args: Vec<String>,
    host: impl Fn(&ScriptHost) + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::{cell::RefCell, rc::Rc};
    let configure = host;
    // A bundle (`zgx build`'s), checked against its manifest: its scripts
    // as the build compiled them (or, as source, its folder run as any is),
    // its modules' libraries for this platform.
    let bundle = if zgx_script::app_bundle::is_bundle(path) {
        let opened = zgx_script::app_bundle::open(path)?;
        // Its scripts read and checked while the window is made.
        opened.read_ahead();
        if zgx::env::var_os("ZGX_BUNDLE_TIMING").is_some() {
            eprintln!(
                "zgx-run: bundle checked in {:.2} ms",
                opened.checked_in.as_secs_f64() * 1e3
            );
        }
        Some(opened)
    } else {
        None
    };
    let mut script = match &bundle {
        Some(bundle) => bundle.ui.join(&bundle.entry),
        None => script_path(path),
    };
    if !script.is_file() {
        return Err(format!("{}: no such script", script.display()).into());
    }
    // The window to open: the one `--window` names among zgx.json's
    // `named_windows`, else its
    // `first`, else its top level's; a named window may open a script of
    // its own, beside the app's.
    let window = LAUNCH.lock().unwrap().window.take();
    let (config, app_host, mut window_config) = match &bundle {
        // A bundle's zgx.json, checked as its other files are.
        Some(bundle) => AppConfig::read_bundle(bundle, window.as_deref())?,
        None => AppConfig::read_window(&script, window.as_deref())?,
    };
    let mut entry = bundle.as_ref().map(|bundle| bundle.entry.clone());
    if let Some(own) = window_config.script.take() {
        let dir = script.parent().unwrap_or(Path::new(".")).to_path_buf();
        script = dir.join(&own);
        if !script.is_file() {
            return Err(format!("{}: no such script", script.display()).into());
        }
        if let Some(entry) = &mut entry {
            *entry = own.to_string_lossy().into_owned();
        }
    }
    // Its modules compile on a worker while AppKit, the GPU and the window
    // are set up, not one by one on the main thread at each `require` (a
    // bundle's are compiled already, unless they are source).
    let checked = bundle.as_ref().and_then(|bundle| bundle.files());
    if checked.is_none() {
        zgx_script::compile_ahead(&script, &[]);
    }
    // The app's native modules, found before its window opens: one it names
    // that isn't built is said now, not as a script calls it.
    #[cfg(feature = "modules")]
    let modules = match &bundle {
        Some(bundle) => bundle.modules.clone(),
        None => modules_of(script.parent().unwrap_or(Path::new(".")))?,
    };
    let checked = checked.zip(entry);
    let notice = LAUNCH.lock().unwrap().notice.take();
    let options = options_of(&config, window_config);
    let visible = options.visible;
    let launched: Vec<PathBuf> = args
        .iter()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .map(|path| path.canonicalize().unwrap_or(path))
        .collect();
    // What the system opens goes to the host once there is one (before,
    // it waits).
    type Opened = (Vec<PathBuf>, Vec<String>);
    let host_slot: Rc<RefCell<Option<ScriptHost>>> = Rc::default();
    let early: Rc<RefCell<Vec<Opened>>> = Rc::default();
    // Its own app on the Linux desktop, not every script one "zgx-run": its
    // windows' Wayland app_id and X11 class, matched to `app_id.desktop`.
    let mut app = Application::new()
        .window(options)
        .desktop_id(config.app_id.clone());
    if let Some(id) = &config.application_id {
        let (slot, early) = (host_slot.clone(), early.clone());
        app = app
            .application_id(id.clone())?
            .on_application_event(move |event, _| {
                let zgx_desktop::ApplicationEvent::OpenUrls(urls) = event else {
                    return;
                };
                let (files, urls): (Vec<_>, Vec<_>) =
                    urls.into_iter().partition(|url| file_path(url).is_some());
                let files = files.iter().filter_map(|url| file_path(url)).collect();
                match &*slot.borrow() {
                    Some(host) => host.open(files, urls),
                    None => early.borrow_mut().push((files, urls)),
                }
            });
    }
    app.run(move |cx| {
        // Its packages, and the scripts it loads beside its own.
        let host = match &checked {
            Some((files, entry)) => ScriptHost::from_files(
                &cx.ui.runtime,
                files.clone(),
                entry,
                zgx_script::Code::VerifiedBytecode,
            ),
            None => ScriptHost::new(&cx.ui.runtime, script.clone()),
        };
        let host = app_host.apply(host.limits(zgx_script::Limits::app_budgets()));
        let host = host.app_id(&config.app_id).args(args.clone());
        // The launch's files, and what the system opened meanwhile:
        // they wait for the script's `ui.on_open`.
        host.open(launched.clone(), Vec::new());
        for (files, urls) in early.borrow_mut().drain(..) {
            host.open(files, urls);
        }
        *host_slot.borrow_mut() = Some(host.clone());
        // `ui.window.titlebar_area`: where the title bar leaves the script room.
        host.window_chrome(
            cx.window_info.titlebar_area.clone(),
            cx.window_info.controls.clone(),
        );
        // `ui.window.zoom`: the window's zoom, as its keys or the script set it.
        host.window_zoom(&cx.ui);
        // `ui.window.hold_frames`: the frame held while what it changes to comes.
        host.window_hold(&cx.ui);
        // `ui.controls` and `ui.press`.
        host.window_controls(&cx.ui);
        // `ui.window.active` and `ui.window.shown`.
        host.window_state(&cx.ui);
        // `ui.window.size`.
        host.window_size(cx.viewport.clone());
        // `ui.window.maximized`.
        host.window_maximized(cx.maximized.clone());
        // `ui.window.full_screen` and `ui.window.full_screen_transition`.
        {
            let transition = cx.full_screen_transition.clone();
            host.window_full_screen(cx.window_info.fullscreen.clone(), move || {
                transition.get().map(|transition| match transition {
                    zgx_desktop::FullScreenTransition::Entering(duration) => {
                        (true, duration.as_secs_f64())
                    }
                    zgx_desktop::FullScreenTransition::Leaving(duration) => {
                        (false, duration.as_secs_f64())
                    }
                })
            });
        }
        // `ui.window.visible`: shown as zgx.json says, and as the script sets it.
        {
            let window = cx.window.clone();
            host.window_visible(visible, move |visible| window.set_visible(visible));
        }
        // The network, unless the host's setup brings its own transport.
        #[cfg(feature = "fetch")]
        fetch::install(&host);
        #[cfg(feature = "socket")]
        socket::install(&host);
        #[cfg(feature = "audio")]
        play_through(&host, cx);
        // Its modules, before the scripts that call them. One that won't
        // load (another interface, a library that isn't what its manifest
        // says) is shown, and the scripts don't run.
        #[cfg(feature = "modules")]
        let loaded = modules.iter().try_for_each(|(name, library)| {
            // SAFETY: The app's own module: zgx.json, the app's own file,
            // names it, or `zgx run` built it from what zgx.json names.
            unsafe { host.load_module(library) }.map_err(|why| format!("module `{name}`: {why}"))
        });
        #[cfg(not(feature = "modules"))]
        let loaded: Result<(), String> = Ok(());
        configure(&host);
        // What `zgx run` says of why it started the app (before its view,
        // which shows it).
        if let Some(notice) = &notice
            && loaded.is_ok()
        {
            host.notice(notice.clone());
        }
        match loaded {
            Ok(()) => {
                let _ = host.reload();
                cx.tasks.spawn(host.watch(Duration::from_millis(250)));
            }
            Err(why) => {
                eprintln!("zgx-run: {why}");
                host.error().set(Some(why));
            }
        }
        let menus_follow = serve_commands(&host, cx);
        // The script's windows (`ui.window`): each open while its state is.
        let windows_follow = follow_windows(&host, cx);
        let saving = host.clone();
        let devtools_asked = inspect_asked();
        #[cfg(feature = "inspect")]
        if devtools_asked {
            zgx_script::inspect::prepare();
        }
        let handle = cx.render(host.view());
        let devtools = devtools_asked
            .then(|| inspect(&host, cx, handle, &script))
            .flatten();

        let keep = std::cell::RefCell::new(Some((menus_follow, windows_follow, devtools)));
        cx.on_closed(move || {
            saving.save();
            drop(keep.borrow_mut().take());
        });
    })?;
    Ok(())
}

/// `host`'s scripts play through the application's engine
/// (`WindowFactory::audio`), made at their first `audio` call: their
/// `playing` states follow voices ending, and `audio.ui_sounds_enabled` the
/// user's setting. For a Rust host that makes its own window. The listener
/// it keeps lives with the host's engine, the app's lifetime.
#[cfg(feature = "audio")]
pub fn play_through(host: &ScriptHost, cx: &zgx_desktop::WindowContext) {
    let heard = host.clone();
    let listening = cx.windows.on_audio(move |event| match event {
        zgx_desktop::AudioEvent::VoicesEnded(ids) => heard.voices_ended(ids),
        zgx_desktop::AudioEvent::UiSounds(on) => heard.set_ui_sounds_enabled(*on),
    });
    let factory = cx.windows.clone();
    host.set_audio(move || {
        let _ = &listening;
        factory.audio()
    });
}

static INSPECT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Serve devtools (`zgx inspect`) from the next window `run` opens, as
/// `zgx run --inspect` asks (`ZGX_INSPECT_SOCKET=1` asks too). Built
/// without the `inspect` feature, it says so and does nothing.
pub fn set_inspect(on: bool) {
    INSPECT.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Devtools for the window, if asked: the socket that serves them, alive
/// while what this returns is.
/// Whether devtools are asked for: `--inspect`, or `ZGX_INSPECT_SOCKET=1`.
fn inspect_asked() -> bool {
    #[cfg(feature = "inspect")]
    let env = zgx_script::inspect::asked();
    #[cfg(not(feature = "inspect"))]
    let env = false;
    INSPECT.load(std::sync::atomic::Ordering::Relaxed) || env
}

#[cfg(feature = "inspect")]
fn inspect(
    host: &ScriptHost,
    cx: &mut zgx_desktop::WindowContext,
    handle: zgx::compose::ViewHandle,
    script: &Path,
) -> Option<zgx_script::inspect::Server> {
    let session =
        zgx_script::inspect::Session::new(&cx.ui, handle, host, Some(cx.inspector.clone()));
    let tasks = cx.tasks.clone();
    match zgx_script::inspect::serve(session, &script.display().to_string(), move |task| {
        tasks.spawn(task)
    }) {
        Ok(server) => {
            eprintln!(
                "zgx run: devtools at {} (zgx inspect help)",
                server.path().display()
            );
            Some(server)
        }
        Err(error) => {
            eprintln!("zgx run: devtools: {error}");
            None
        }
    }
}
#[cfg(not(feature = "inspect"))]
fn inspect(
    _: &ScriptHost,
    _: &mut zgx_desktop::WindowContext,
    _: zgx::compose::ViewHandle,
    _: &Path,
) -> Option<()> {
    eprintln!("zgx run: built without devtools (the `inspect` feature)");
    None
}

/// `host`'s commands (`ui.command`) as the application's menu bar, kept
/// current as a reload changes them, each item running its command: their
/// shortcuts work wherever focus is, as `run` gives them, and those the
/// scripts reserve (`reserved = true`) come before a focused page or
/// terminal. For a Rust host that makes its own window; keep the effect it
/// returns while the window lives.
pub fn serve_commands(
    host: &ScriptHost,
    cx: &mut zgx_desktop::WindowContext,
) -> zgx::reactive::Effect {
    let (commands, reserved) = (host.commands(), host.reserved_keys());
    let windows = cx.windows.clone();
    let follow = cx.ui.runtime.effect(move || {
        if let Err(error) = windows.set_menus(menus(&commands.get())) {
            eprintln!("zgx run: menus: {error}");
        }
        let keys = reserved
            .get()
            .iter()
            .filter_map(|key| accelerator(key))
            .collect();
        if let Err(error) = windows.set_reserved_shortcuts(keys) {
            eprintln!("zgx run: reserved shortcuts: {error}");
        }
    });
    let run = host.clone();
    cx.on_menu_action(move |action| {
        run.run_command(&action.0);
    });
    follow
}

/// Open and close `host`'s windows (`ui.window`) as their `open` states
/// say, and set a state false when its window is closed. The effect it
/// returns keeps this going.
pub fn follow_windows(host: &ScriptHost, cx: &zgx_desktop::WindowContext) -> zgx::reactive::Effect {
    use std::{cell::RefCell, collections::HashMap, rc::Rc};
    use zgx_script::Value;
    let factory = cx.windows.clone();
    let runtime = cx.ui.runtime.clone();
    let open: Rc<RefCell<HashMap<String, zgx_desktop::WindowHandle>>> = Rc::default();
    let (list, host) = (host.windows(), host.clone());
    cx.ui.runtime.effect(move || {
        let windows = list.get();
        // Gone from the list (an item's window, its item let go): closed.
        let gone: Vec<String> = open
            .borrow()
            .keys()
            .filter(|id| !windows.iter().any(|w| &w.id == *id))
            .cloned()
            .collect();
        for id in gone {
            if let Some(handle) = open.borrow_mut().remove(&id) {
                runtime.untracked(|| handle.close());
            }
        }
        for window in windows {
            let wanted = window.open.get().truthy();
            let shown = open
                .borrow()
                .get(&window.id)
                .is_some_and(|handle| !handle.is_closed());
            if wanted && !shown {
                let (min_size, max_size) = host.window_size_limits(&window.id);
                let extras = host.window_extras(&window.id);
                let mut options = WindowOptions {
                    title: window.title.clone(),
                    min_size,
                    max_size,
                    width: window.width,
                    height: window.height,
                    frame_autosave: window.frame_autosave.clone(),
                    under_pointer: extras.under_pointer,
                    tabbing: match window.tabbing.as_deref() {
                        Some("preferred") => zgx_desktop::Tabbing::Preferred,
                        Some("disallowed") => zgx_desktop::Tabbing::Disallowed,
                        _ => zgx_desktop::Tabbing::Automatic,
                    },
                    visible: extras
                        .visible
                        .as_ref()
                        .is_none_or(|visible| visible.with_untracked(Value::truthy)),
                    ..Default::default()
                };
                apply_look(&mut options, &extras.look);
                let (view_host, state, id) = (host.clone(), window.open.clone(), window.id.clone());
                let own = extras.state.clone();
                let visible = extras.visible.clone();
                // Devtools reach it too, while it is open (`zgx inspect`'s
                // `window`).
                let devtools = inspect_asked().then(|| window.title.clone());
                let handle = factory.open(options, move |wcx| {
                    // The window's own state, as its content reads it (`win`).
                    if let Some(own) = own {
                        let runtime = wcx.ui.runtime.clone();
                        let (active, shown, viewport) = (
                            wcx.ui.observe_window_active(),
                            wcx.ui.observe_shown(),
                            wcx.viewport.clone(),
                        );
                        let keep = runtime.effect(move || {
                            let pairs = [
                                (&own.active, Value::Bool(active.get())),
                                (&own.shown, Value::Bool(shown.get())),
                            ];
                            for (state, now) in pairs {
                                if state.with_untracked(|was| *was != now) {
                                    state.set(now);
                                }
                            }
                            let (width, height) = viewport.get();
                            let now = zgx_script::window_size_value(width, height);
                            if own.size.with_untracked(|was| *was != now) {
                                own.size.set(now);
                            }
                        });
                        wcx.on_closed(move || drop(keep));
                    }
                    // Shown and hidden as its `visible` state says.
                    if let Some(visible) = visible {
                        let window = wcx.window.clone();
                        let keep = wcx.ui.runtime.effect(move || {
                            window.set_visible(visible.get().truthy());
                        });
                        wcx.on_closed(move || drop(keep));
                    }
                    let closing = id.clone();
                    wcx.on_closed(move || {
                        if state.with_untracked(Value::truthy) {
                            state.set(Value::Bool(false));
                        }
                        #[cfg(feature = "inspect")]
                        zgx_script::inspect::forget_window(&closing);
                        let _ = &closing;
                    });
                    let handle = wcx.render(view_host.window_view(&id));
                    #[cfg(feature = "inspect")]
                    if let Some(title) = devtools {
                        let session = zgx_script::inspect::Session::new(
                            &wcx.ui,
                            handle,
                            &view_host,
                            Some(wcx.inspector.clone()),
                        );
                        zgx_script::inspect::register_window(&id, &title, session);
                    }
                    #[cfg(not(feature = "inspect"))]
                    let _ = (handle, devtools);
                });
                open.borrow_mut().insert(window.id.clone(), handle);
            } else if !wanted
                && shown
                && let Some(handle) = open.borrow_mut().remove(&window.id)
            {
                runtime.untracked(|| handle.close());
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_zgx_json_configures_the_window_and_rejects_what_it_does_not_know() {
        let dir = std::env::temp_dir().join(format!("zgx-run-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("app.luau");
        std::fs::write(&script, "return ui.text 'x'").unwrap();
        let config = AppConfig::for_script(&script).unwrap();
        assert_eq!(config.width, 960.);
        std::fs::write(
            dir.join("zgx.json"),
            r#"{ "title": "Mail", "width": 1100, "min_width": 600, "min_height": 400, "titlebar": "inset", "app_id": "mail" }"#,
        )
        .unwrap();
        let config = AppConfig::for_script(&script).unwrap();
        assert_eq!(
            (config.title.as_str(), config.width, config.min_size),
            ("Mail", 1100., Some((600., 400.)))
        );
        assert_eq!(config.titlebar, zgx_desktop::Titlebar::Inset);
        // A fixed size that stays resizable: min and max the same.
        std::fs::write(
            dir.join("zgx.json"),
            r#"{ "min_width": 400, "min_height": 300, "max_width": 400, "max_height": 300 }"#,
        )
        .unwrap();
        let (config, _, window) = AppConfig::read(&script).unwrap();
        assert_eq!(
            (config.min_size, window.max_size),
            (Some((400., 300.)), Some((400., 300.)))
        );
        std::fs::write(dir.join("zgx.json"), r#"{ "max_width": 400 }"#).unwrap();
        let error = AppConfig::for_script(&script).unwrap_err();
        assert!(
            error.contains("`max_width` and `max_height` go together"),
            "{error}"
        );
        std::fs::write(dir.join("zgx.json"), "{}").unwrap();
        let visible = |script: &Path| AppConfig::read(script).unwrap().2.visible;
        assert!(visible(&script));
        // Opened hidden, for the script to show.
        std::fs::write(dir.join("zgx.json"), r#"{ "visible": false }"#).unwrap();
        assert!(!visible(&script));
        std::fs::write(dir.join("zgx.json"), r#"{ "visible": "no" }"#).unwrap();
        let error = AppConfig::for_script(&script).unwrap_err();
        assert!(error.contains("`visible` is not a boolean"), "{error}");
        std::fs::write(dir.join("zgx.json"), r#"{ "widht": 3 }"#).unwrap();
        let error = AppConfig::for_script(&script).unwrap_err();
        assert!(error.contains("unknown key `widht`"), "{error}");
        assert!(accelerator("cmd+shift+d").is_some());
        // Cmd+N is Ctrl+N off macOS: there, Win+N is Windows' own.
        let new = accelerator("cmd+n").unwrap().0;
        assert_eq!(new.modifiers.meta, cfg!(target_os = "macos"));
        assert_eq!(new.modifiers.control, !cfg!(target_os = "macos"));
        assert!(accelerator("super+n").unwrap().0.modifiers.meta);
        std::fs::write(
            dir.join("zgx.json"),
            r#"{ "app_id": "code", "application_id": "dev.zgx.code" }"#,
        )
        .unwrap();
        let config = AppConfig::for_script(&script).unwrap();
        assert_eq!(config.application_id.as_deref(), Some("dev.zgx.code"));
        let frame = |script: &Path| AppConfig::read(script).unwrap().2.frame_autosave;
        assert_eq!(frame(&script), None);
        // The window's frame kept: under the app's id, or a name given.
        std::fs::write(
            dir.join("zgx.json"),
            r#"{ "app_id": "assets", "frame_autosave": true }"#,
        )
        .unwrap();
        assert_eq!(frame(&script).as_deref(), Some("assets"));
        std::fs::write(dir.join("zgx.json"), r#"{ "frame_autosave": "library" }"#).unwrap();
        assert_eq!(frame(&script).as_deref(), Some("library"));
        std::fs::write(dir.join("zgx.json"), r#"{ "frame_autosave": 1 }"#).unwrap();
        let error = AppConfig::for_script(&script).unwrap_err();
        assert!(error.contains("`frame_autosave` is not"), "{error}");
        // Packages, from the config's directory, are folders.
        std::fs::create_dir_all(dir.join("kit")).unwrap();
        std::fs::write(
            dir.join("zgx.json"),
            r#"{ "packages": { "kit": "./kit" } }"#,
        )
        .unwrap();
        let (_, app, _) = AppConfig::read(&script).unwrap();
        assert_eq!(app.packages, [("kit".to_string(), dir.join("./kit"))]);
        std::fs::write(
            dir.join("zgx.json"),
            r#"{ "packages": { "kit": "./none" } }"#,
        )
        .unwrap();
        let error = AppConfig::read(&script).unwrap_err();
        assert!(error.contains("is not a folder"), "{error}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_zgx_json_places_an_inset_title_bars_traffic_lights() {
        let dir = std::env::temp_dir().join(format!("zgx-run-lights-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("app.luau");
        std::fs::write(&script, "return ui.text 'x'").unwrap();
        let lights = |script: &Path| AppConfig::read(script).map(|read| read.2.traffic_lights);
        assert_eq!(lights(&script), Ok(None));
        std::fs::write(
            dir.join("zgx.json"),
            r#"{ "titlebar": "inset", "traffic_lights": { "x": 18, "y": 22.5 } }"#,
        )
        .unwrap();
        assert_eq!(lights(&script), Ok(Some((18., 22.5))));
        for wrong in [
            r#"{ "traffic_lights": [18, 22] }"#,
            r#"{ "traffic_lights": { "x": 18 } }"#,
            r#"{ "traffic_lights": { "x": 18, "y": "22" } }"#,
            r#"{ "traffic_lights": { "x": 18, "y": 22, "z": 1 } }"#,
        ] {
            std::fs::write(dir.join("zgx.json"), wrong).unwrap();
            let error = lights(&script).unwrap_err();
            assert!(
                error.contains("`traffic_lights` is not"),
                "{wrong}: {error}"
            );
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// A platform's block replaces the window's keys on that platform,
    /// shallow and per key: a table (`glass`) is the block's whole, and
    /// the keys it doesn't have stay the window's.
    #[test]
    fn a_platforms_block_replaces_the_window_keys_it_has() {
        use zgx_script::window_look::{Glass, Level, Material};
        let dir = std::env::temp_dir().join(format!("zgx-run-platforms-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("app.luau");
        std::fs::write(&script, "return ui.text 'x'").unwrap();
        std::fs::write(
            dir.join("zgx.json"),
            r#"{ "title": "A", "decorations": false, "level": "always_on_top",
                 "glass": { "material": "sidebar", "plain": true },
                 "macos": { "decorations": true, "glass": { "material": "hud" } },
                 "windows": { "title": "B", "caption_height": 38 } }"#,
        )
        .unwrap();
        let on = |platform: &str| AppConfig::read_for(&script, platform, None).unwrap();
        let (mac, _, mac_window) = on("macos");
        assert_eq!(mac_window.look.decorations, Some(true));
        assert_eq!(
            mac_window.look.glass,
            Some(Glass {
                material: Material::Hud,
                plain: false
            }),
            "the block's glass whole: its `plain` isn't the window's"
        );
        assert_eq!(
            (mac.title.as_str(), mac_window.look.level),
            ("A", Some(Level::AlwaysOnTop))
        );
        let (linux, _, linux_window) = on("linux");
        assert_eq!(linux_window.look.decorations, Some(false));
        assert_eq!(
            linux_window.look.glass,
            Some(Glass {
                material: Material::Sidebar,
                plain: true
            })
        );
        assert_eq!(
            (linux.title.as_str(), linux_window.caption_height),
            ("A", None)
        );
        let (windows, _, windows_window) = on("windows");
        assert_eq!(
            (windows.title.as_str(), windows_window.caption_height),
            ("B", Some(38.))
        );
        assert_eq!(windows_window.look.decorations, Some(false));
        // What a block may not hold, told on every platform (a slip in
        // `"windows"` on a Mac too).
        for (json, said) in [
            (
                r#"{ "macos": { "app_id": "x" } }"#,
                "`macos` has `app_id`, which isn't a window key that can differ by platform",
            ),
            (
                r#"{ "linux": { "windows": {} } }"#,
                "`linux` holds `windows`: a platform's block holds window keys, not another block",
            ),
            (
                r#"{ "windows": 3 }"#,
                "`windows` is not an object of window keys",
            ),
            (
                r#"{ "windows": { "titel": "x" } }"#,
                "`windows` has `titel`",
            ),
            (r#"{ "ios": { "title": "x" } }"#, "unknown key `ios`"),
            (
                r#"{ "linux": { "glass": "yes" } }"#,
                "`glass` is not a boolean or { material, plain }",
            ),
        ] {
            std::fs::write(dir.join("zgx.json"), json).unwrap();
            for platform in zgx_script::window_look::PLATFORMS {
                let error = AppConfig::read_for(&script, platform, None).unwrap_err();
                assert!(error.contains(said), "{json} on {platform}: {error}");
            }
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// The look's keys reach the window's options as the platform's calls
    /// take them; what they don't say keeps the options' defaults.
    #[test]
    fn a_windows_look_reaches_its_options() {
        let dir = std::env::temp_dir().join(format!("zgx-run-look-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("app.luau");
        std::fs::write(&script, "return ui.text 'x'").unwrap();
        let options = |json: &str| {
            std::fs::write(dir.join("zgx.json"), json).unwrap();
            let (_, _, window) = AppConfig::read(&script).unwrap();
            let mut options = WindowOptions::default();
            apply_look(&mut options, &window.look);
            options
        };
        let plain = options("{}");
        let defaults = WindowOptions::default();
        assert_eq!(
            (
                plain.glass,
                plain.decorations,
                plain.resizable,
                plain.transparent,
                plain.shadow,
                plain.covers_display,
                plain.level
            ),
            (
                defaults.glass,
                defaults.decorations,
                defaults.resizable,
                defaults.transparent,
                defaults.shadow,
                defaults.covers_display,
                defaults.level
            )
        );
        let intro = options(
            r#"{ "transparent": true, "decorations": false, "resizable": false, "covers_display": true,
                 "level": "always_on_top", "shadow": false,
                 "glass": { "material": "under_window_background", "plain": true } }"#,
        );
        assert!(
            intro.transparent
                && !intro.decorations
                && !intro.resizable
                && intro.covers_display
                && !intro.shadow
        );
        assert_eq!(intro.level, zgx_desktop::WindowLevel::AlwaysOnTop);
        assert_eq!(
            intro.glass,
            Some(
                zgx_desktop::Glass::new(zgx_desktop::GlassMaterial::UnderWindowBackground).plain()
            )
        );
        assert_eq!(
            options(r#"{ "glass": true }"#).glass,
            Some(zgx_desktop::Glass::default())
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// A named window's key comes from the first layer that has it: its
    /// platform's block, the window, the top level's platform block, the
    /// top level.
    #[test]
    fn a_named_windows_keys_come_from_its_layers_in_order() {
        let dir = std::env::temp_dir().join(format!("zgx-run-named-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("app.luau");
        std::fs::write(&script, "return ui.text 'x'").unwrap();
        // `title` at all four layers; `width` at three; `height` at two;
        // `shadow` at the top alone.
        std::fs::write(
            dir.join("zgx.json"),
            r#"{ "title": "top", "width": 100, "height": 10, "shadow": false,
                 "macos": { "title": "top macos", "width": 200 },
                 "named_windows": {
                     "main": { "title": "main", "width": 300, "height": 30,
                               "macos": { "title": "main macos" },
                               "linux": { "width": 400 } },
                     "intro": {}
                 } }"#,
        )
        .unwrap();
        let read = |platform: &str, window: Option<&str>| {
            let (config, _, window) = AppConfig::read_for(&script, platform, window).unwrap();
            (
                config.title,
                config.width,
                config.height,
                window.look.shadow,
            )
        };
        let main = |platform| read(platform, Some("main"));
        assert_eq!(main("macos"), ("main macos".into(), 300., 30., Some(false)));
        assert_eq!(main("linux"), ("main".into(), 400., 30., Some(false)));
        assert_eq!(main("windows"), ("main".into(), 300., 30., Some(false)));
        // A named window with nothing of its own: the top level's, by platform.
        assert_eq!(
            read("macos", Some("intro")),
            ("top macos".into(), 200., 10., Some(false))
        );
        assert_eq!(
            read("linux", Some("intro")),
            ("top".into(), 100., 10., Some(false))
        );
        // None named, no `first`: the top level's.
        assert_eq!(read("linux", None), ("top".into(), 100., 10., Some(false)));
        std::fs::remove_dir_all(dir).ok();
    }

    /// `first` names the window that opens unless `--window` names
    /// another; a name zgx.json doesn't have is an error listing those it
    /// does. A named window may open a script of its own.
    #[test]
    fn the_first_window_is_the_one_asked_for_or_first() {
        let dir = std::env::temp_dir().join(format!("zgx-run-first-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("app.luau");
        std::fs::write(&script, "return ui.text 'x'").unwrap();
        std::fs::write(
            dir.join("zgx.json"),
            r#"{ "first": "main", "named_windows": {
                   "main": { "title": "Main" },
                   "intro": { "title": "Intro", "transparent": true, "script": "intro.luau" } } }"#,
        )
        .unwrap();
        let title =
            |window: Option<&str>| AppConfig::read_window(&script, window).map(|read| read.0.title);
        assert_eq!(title(None).as_deref(), Ok("Main"), "first");
        assert_eq!(title(Some("intro")).as_deref(), Ok("Intro"), "asked for");
        let error = title(Some("gallery")).unwrap_err();
        assert!(
            error.contains("no window `gallery`: `named_windows` names intro, main"),
            "{error}"
        );
        let (options, own) = window_options(&script, Some("intro")).unwrap();
        assert!(options.transparent);
        assert_eq!(own, Some(dir.join("intro.luau")));
        assert_eq!(
            window_options(&script, Some("main")).unwrap().1,
            None,
            "the app's own script"
        );
        for (json, said) in [
            (
                r#"{ "first": "nope", "named_windows": { "main": {} } }"#,
                "`first` is `nope`, which `named_windows` doesn't name (it names main)",
            ),
            (
                r#"{ "named_windows": { "main": 3 } }"#,
                "`named_windows.main` is not an object of window keys",
            ),
            (
                r#"{ "named_windows": { "main": { "app_id": "x" } } }"#,
                "a named window has `app_id`, which isn't a window key",
            ),
            (
                r#"{ "named_windows": { "main": { "script": 3 } } }"#,
                "a named window's `script` is not a file's name",
            ),
            // A slip in a window that isn't opening, on another platform:
            // told all the same.
            (
                r#"{ "first": "main", "named_windows": { "main": {}, "intro": { "windows": { "level": "top" } } } }"#,
                "`level` is `top`: one of normal, always_on_top (window `intro`, on windows)",
            ),
        ] {
            std::fs::write(dir.join("zgx.json"), json).unwrap();
            let error = AppConfig::read_window(&script, None).unwrap_err();
            assert!(error.contains(said), "{json}: {error}");
        }
        std::fs::remove_file(dir.join("zgx.json")).unwrap();
        let error = AppConfig::read_window(&script, Some("main")).unwrap_err();
        assert!(error.contains("names a window `main`"), "{error}");
        std::fs::remove_dir_all(dir).ok();
    }

    /// A bundle's windows are read through its checked files: a zgx.json
    /// changed, or removed, on the disk since it was built is refused (it
    /// would be another app's window), and a bundle with none opens the
    /// defaults.
    #[test]
    fn a_bundles_windows_are_read_checked() {
        let dir =
            std::env::temp_dir().join(format!("zgx-run-bundle-window-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let app = dir.join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("app.luau"), "return ui.text 'x'").unwrap();
        std::fs::write(
            app.join("zgx.json"),
            r#"{ "title": "Bundled", "first": "intro", "named_windows": { "intro": { "transparent": true, "decorations": false } } }"#,
        )
        .unwrap();
        let made = |name: &str| {
            let out = dir.join(name);
            zgx_script::app_bundle::write(
                &app,
                &zgx_script::app_bundle::Options {
                    out: Some(out.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
            zgx_script::app_bundle::open(&out).unwrap()
        };
        let bundle = made("whole");
        let (options, own) = window_options_in(&bundle, None).unwrap();
        assert!(options.transparent && !options.decorations, "first's");
        assert_eq!((options.title.as_str(), own), ("Bundled", None));
        // Changed on the disk: refused, whatever it says now.
        let on_disk = bundle.ui.join("zgx.json");
        std::fs::write(&on_disk, r#"{ "title": "Other", "named_windows": {} }"#).unwrap();
        let error = window_options_in(&bundle, None).unwrap_err();
        assert!(
            error.contains("isn't the file its bundle's manifest names"),
            "{error}"
        );
        // Removed: refused too, not read as none.
        std::fs::remove_file(&on_disk).unwrap();
        assert!(window_options_in(&bundle, None).is_err(), "removed");
        // A bundle built without one: the defaults, and no window by name.
        std::fs::remove_file(app.join("zgx.json")).unwrap();
        let bare = made("bare");
        assert_eq!(window_options_in(&bare, None).unwrap().0.title, "ui");
        let error = window_options_in(&bare, Some("intro")).unwrap_err();
        assert!(error.contains("names a window `intro`"), "{error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A bundle of source's windows are its folder's zgx.json as it is, as
    /// an app's run from its folder are.
    #[test]
    fn a_source_bundles_windows_are_its_folders() {
        let dir =
            std::env::temp_dir().join(format!("zgx-run-source-window-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let app = dir.join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("app.luau"), "return ui.text 'x'").unwrap();
        std::fs::write(app.join("zgx.json"), r#"{ "title": "Built" }"#).unwrap();
        let out = dir.join("out");
        let options = zgx_script::app_bundle::Options {
            out: Some(out.clone()),
            ui: zgx_script::app_bundle::Ui::Source,
            ..Default::default()
        };
        zgx_script::app_bundle::write(&app, &options).unwrap();
        let bundle = zgx_script::app_bundle::open(&out).unwrap();
        let title = || window_options_in(&bundle, None).map(|(options, _)| options.title);
        assert_eq!(title().unwrap(), "Built");
        let on_disk = bundle.ui.join("zgx.json");
        std::fs::write(&on_disk, r#"{ "title": "Edited" }"#).unwrap();
        assert_eq!(title().unwrap(), "Edited");
        // Broken is said, and gone is none, as in any folder.
        std::fs::write(&on_disk, "{").unwrap();
        assert!(title().is_err());
        std::fs::remove_file(&on_disk).unwrap();
        assert_eq!(title().unwrap(), "ui");
        // A window's script, or a package, out of its folder is refused.
        std::fs::write(dir.join("out.luau"), "return ui.text 'out'").unwrap();
        for outside in [
            r#"{ "first": "w", "named_windows": { "w": { "script": "../../out.luau" } } }"#,
            r#"{ "packages": { "kit": "../../app" } }"#,
        ] {
            std::fs::write(&on_disk, outside).unwrap();
            let error = title().unwrap_err();
            assert!(error.contains("ui/"), "{outside}: {error}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_config_under_its_name_from_before_the_rename_still_configures() {
        let dir = std::env::temp_dir().join(format!("zgx-run-old-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("app.luau");
        std::fs::write(&script, "return ui.text 'x'").unwrap();
        let old = dir.join("zgui.json"); // rename-to-zgx: keep
        std::fs::write(&old, r#"{ "title": "Old" }"#).unwrap();
        assert_eq!(AppConfig::for_script(&script).unwrap().title, "Old");
        std::fs::write(dir.join("zgx.json"), r#"{ "title": "New" }"#).unwrap();
        assert_eq!(AppConfig::for_script(&script).unwrap().title, "New");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn file_urls_the_system_opens_are_paths_and_others_are_not() {
        assert_eq!(
            file_path("file:///tmp/a%20b.rs"),
            Some(PathBuf::from("/tmp/a b.rs"))
        );
        assert_eq!(
            file_path("file://localhost/tmp/x"),
            Some(PathBuf::from("/tmp/x"))
        );
        assert_eq!(file_path("https://example.org/a"), None);
        assert_eq!(file_path("file:///bad%zz"), None);
        if cfg!(windows) {
            assert_eq!(
                file_path("file:///C:/a%20b.txt"),
                Some(PathBuf::from("C:/a b.txt"))
            );
        }
    }
}
