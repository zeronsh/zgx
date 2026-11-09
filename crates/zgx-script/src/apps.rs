//! Programs scripts start and show: `ui.app(id, opts)` starts one (no
//! shell) with an external surface's connection (zgx-surface), or a
//! WebAssembly app in this process (`wasm = "app.wasm"`, zgx-wasm), and
//! returns its session; `ui.surface { app = a }` shows its frames,
//! forwarding the element's input to it. Scripts treat both alike.
//!
//! Sessions belong to the host, as terminals do: reloading keeps every
//! program running, and the run that asks for `id` again gets the same one
//! (its `on_exit` replaces the last run's). Sessions are the script's who
//! started them: a plugin reaches only those it started (two plugins'
//! `"game"` are two sessions, and neither is the app's). A program ends with
//! `app:close()`, the host, or its plugin's removal
//! (`ScriptHost::set_plugins`); one that exited starts again when `ui.app`
//! is given options for it.
//!
//! What may run is the host's: a plugin needs `Capability::Apps` for a
//! program, and the whole spec asked for (command, args, cwd, env) goes to
//! the host's hook (`ScriptHost::vet_apps`), which refuses it or says
//! exactly what to start (a resolved path, pinned arguments, a filtered
//! environment); the app's VM asks its guard too (`GuardKind::App`, named by
//! the spec as JSON). A WebAssembly app needs `Capability::Wasm` (or
//! `Apps`), its module inside the plugin's own folder, and goes to
//! `ScriptHost::vet_wasm` (and `GuardKind::Wasm`), which says its limits and
//! grants. Both are asked again each time a script reaches a running
//! session, so a permission taken back holds from then on.
use mlua::{Lua, Table};
use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

/// A program to start: no shell, `command` run with `args`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppSpec {
    /// The program: a path, or a name looked up on `PATH`.
    pub command: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Where it starts; the host's own directory if `None`.
    pub cwd: Option<PathBuf>,
    /// Variables added to the host's environment for it, sorted by name.
    pub env: Vec<(String, String)>,
}
impl AppSpec {
    /// `command` with `args`, no `cwd` or `env`.
    pub fn new(
        command: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }
    /// With `cwd`.
    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }
    /// With `env` (sorted by name).
    pub fn env(mut self, env: impl IntoIterator<Item = (String, String)>) -> Self {
        self.env = env.into_iter().collect();
        self.env.sort();
        self
    }
    /// As JSON, `{ "command", "args"?, "cwd"?, "env"? }` (an object of
    /// names to values), leaving out what is empty: what `GuardKind::App`
    /// is named by.
    pub fn to_json(&self) -> String {
        let mut object = serde_json::Map::new();
        object.insert("command".into(), self.command.clone().into());
        if !self.args.is_empty() {
            object.insert("args".into(), self.args.clone().into());
        }
        if let Some(cwd) = &self.cwd {
            object.insert("cwd".into(), cwd.to_string_lossy().into_owned().into());
        }
        if !self.env.is_empty() {
            let env: serde_json::Map<String, serde_json::Value> = self
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone().into()))
                .collect();
            object.insert("env".into(), env.into());
        }
        serde_json::Value::Object(object).to_string()
    }
}

/// What a script asks to start or reach, as the host's hook
/// (`ScriptHost::vet_apps`) is told.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppLaunch {
    /// The plugin asking, by its name; `None` for the app's own scripts.
    pub plugin: Option<String>,
    /// The session's id, as the script gave it.
    pub id: String,
    /// What it asked to start (for a running session: what it asked as
    /// the session started).
    pub asked: AppSpec,
    /// The session is running and the script is reaching it again (a
    /// reload, a lookup): an `Err` refuses it; what an `Ok` returns is
    /// not started.
    pub running: bool,
}

pub(crate) type Vet = dyn Fn(&AppLaunch) -> Result<AppSpec, String>;

/// A WebAssembly app to run in the host's process (`ui.app(id, { wasm })`):
/// its component and its sandbox. A script asks for the module and lower
/// limits; only the host's hook (`ScriptHost::vet_wasm`) grants more (a
/// data folder, the time of day, higher limits).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WasmSpec {
    /// The component, resolved: a plugin's is inside its own folder.
    pub module: PathBuf,
    /// Its linear memories' most, in bytes (1 GiB unless asked lower).
    pub memory_limit: u64,
    /// The longest any call into it may run before the session fails (5 s).
    pub timeout: Duration,
    /// The most its GPU buffers and textures hold, in bytes (1 GiB).
    pub gpu_bytes: u64,
    /// The most GPU objects it holds at once (16384).
    pub gpu_objects: u32,
    /// A folder of its own, seen as `/data`: none unless the hook grants it
    /// (it then sees no other file).
    pub data_dir: Option<PathBuf>,
    /// Whether it reads the time of day (it reads the Unix epoch if not).
    pub wall_clock: bool,
}
impl WasmSpec {
    /// `module` with the default limits and nothing granted.
    pub fn new(module: impl Into<PathBuf>) -> Self {
        Self {
            module: module.into(),
            memory_limit: 1 << 30,
            timeout: Duration::from_secs(5),
            gpu_bytes: 1 << 30,
            gpu_objects: 16384,
            data_dir: None,
            wall_clock: false,
        }
    }
    /// With `memory_limit`.
    pub fn memory_limit(mut self, bytes: u64) -> Self {
        self.memory_limit = bytes;
        self
    }
    /// With `timeout`.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    /// With `gpu_bytes`.
    pub fn gpu_bytes(mut self, bytes: u64) -> Self {
        self.gpu_bytes = bytes;
        self
    }
    /// With `gpu_objects`.
    pub fn gpu_objects(mut self, objects: u32) -> Self {
        self.gpu_objects = objects;
        self
    }
    /// With a folder of its own (`/data`).
    pub fn data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(dir.into());
        self
    }
    /// With the time of day granted, or not.
    pub fn wall_clock(mut self, granted: bool) -> Self {
        self.wall_clock = granted;
        self
    }
}

/// What a script asks to run or reach as a WebAssembly app, as the host's
/// hook (`ScriptHost::vet_wasm`) is told.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WasmLaunch {
    /// The plugin asking, by its name; `None` for the app's own scripts.
    pub plugin: Option<String>,
    /// The session's id, as the script gave it.
    pub id: String,
    /// What it asked to run (for a running session: as it started).
    pub asked: WasmSpec,
    /// The session is running and the script reaches it again: an `Err`
    /// refuses it (it is closed); what an `Ok` returns is not started.
    pub running: bool,
}

pub(crate) type VetWasm = dyn Fn(&WasmLaunch) -> Result<WasmSpec, String>;

/// The host's sessions, and what it vets them with.
#[derive(Default)]
pub(crate) struct Apps {
    vet: RefCell<Option<Rc<Vet>>>,
    vet_wasm: RefCell<Option<Rc<VetWasm>>>,
    #[cfg(feature = "apps")]
    sessions: running::Sessions,
}
impl Apps {
    pub fn set_vet(&self, vet: Rc<Vet>) {
        *self.vet.borrow_mut() = Some(vet);
    }
    pub fn set_vet_wasm(&self, vet: Rc<VetWasm>) {
        *self.vet_wasm.borrow_mut() = Some(vet);
    }
    /// What to run for `launch`: the hook's answer, or with no hook what
    /// was asked.
    #[cfg(feature = "apps")]
    fn vet_wasm(&self, launch: &WasmLaunch) -> Result<WasmSpec, String> {
        let vet = self.vet_wasm.borrow().clone();
        match vet {
            Some(vet) => vet(launch),
            None => Ok(launch.asked.clone()),
        }
    }
    /// What to start for `launch`: the hook's answer, or with no hook what
    /// was asked.
    #[cfg(feature = "apps")]
    fn vet(&self, launch: &AppLaunch) -> Result<AppSpec, String> {
        let vet = self.vet.borrow().clone();
        match vet {
            Some(vet) => vet(launch),
            None => Ok(launch.asked.clone()),
        }
    }
    /// Where sessions read their programs' messages: the host view's tasks.
    #[allow(unused_variables)]
    pub fn set_runner(&self, runner: &zgx::compose::TaskRunner) {
        #[cfg(feature = "apps")]
        self.sessions.set_runner(runner);
    }
    /// End the sessions of plugins not in `kept`.
    #[allow(unused_variables)]
    pub fn keep_plugins(&self, kept: &[String]) {
        #[cfg(feature = "apps")]
        self.sessions.keep_plugins(kept);
    }
}

/// Installs `ui.app` and `ui.surface`.
pub(crate) fn install(lua: &Lua, ui: &Table, apps: Rc<Apps>) -> mlua::Result<()> {
    #[cfg(feature = "apps")]
    return running::install(lua, ui, apps);
    #[cfg(not(feature = "apps"))]
    {
        let _ = apps;
        let missing = |what: &'static str| {
            lua.create_function(move |_, _: mlua::MultiValue| -> mlua::Result<()> {
                Err(mlua::Error::runtime(format!(
                    "{what}: this build runs no programs (zgx-script's `apps` feature)"
                )))
            })
        };
        ui.set("app", missing("ui.app")?)?;
        ui.set("surface", missing("ui.surface")?)
    }
}

#[cfg(feature = "apps")]
mod running {
    use super::{AppLaunch, AppSpec, Apps, WasmLaunch, WasmSpec};
    use crate::{Capability, Vm, node::env};
    use mlua::{Function, Lua, Table, UserData, Value as LuaValue};
    use std::{
        cell::RefCell,
        collections::HashMap,
        path::{Path, PathBuf},
        rc::{Rc, Weak},
        time::Duration,
    };
    use zgx_surface::host::{App, AppOptions, AppStatus};

    /// A session by who started it (a plugin's name, or `None` for the
    /// app's scripts) and the id it gave: a script reaches only its own.
    type Key = (Option<Rc<str>>, String);

    /// A program in another process, or a WebAssembly app in this one:
    /// scripts reach both alike.
    #[derive(Clone)]
    pub(crate) enum Session {
        Process(App),
        #[cfg(feature = "wasm")]
        Wasm(zgx_wasm::WasmApp),
    }
    impl Session {
        fn ptr_eq(&self, other: &Session) -> bool {
            match (self, other) {
                (Session::Process(a), Session::Process(b)) => a.ptr_eq(b),
                #[cfg(feature = "wasm")]
                (Session::Wasm(a), Session::Wasm(b)) => a.ptr_eq(b),
                #[cfg(feature = "wasm")]
                _ => false,
            }
        }
        /// `"starting"`, `"running"`, `"exited"` or `"failed"` (tracked).
        fn status(&self) -> &'static str {
            match self {
                Session::Process(app) => app.status().with(AppStatus::name),
                #[cfg(feature = "wasm")]
                Session::Wasm(app) => app.status().with(zgx_wasm::WasmStatus::name),
            }
        }
        /// It exited or failed (untracked).
        fn ended(&self) -> bool {
            match self {
                Session::Process(app) => matches!(
                    app.status().with_untracked(Clone::clone),
                    AppStatus::Exited(_) | AppStatus::Failed(_)
                ),
                #[cfg(feature = "wasm")]
                Session::Wasm(app) => matches!(
                    app.status().with_untracked(Clone::clone),
                    zgx_wasm::WasmStatus::Exited(_) | zgx_wasm::WasmStatus::Failed(_)
                ),
            }
        }
        fn title(&self) -> String {
            match self {
                Session::Process(app) => app.title().get(),
                #[cfg(feature = "wasm")]
                Session::Wasm(app) => app.title().get(),
            }
        }
        fn pid(&self) -> Option<u32> {
            match self {
                Session::Process(app) => app.pid(),
                #[cfg(feature = "wasm")]
                Session::Wasm(_) => None,
            }
        }
        fn exit_code(&self) -> Option<i32> {
            match self {
                Session::Process(app) => app.status().with(|s| match s {
                    AppStatus::Exited(code) => *code,
                    _ => None,
                }),
                #[cfg(feature = "wasm")]
                Session::Wasm(app) => app.status().with(|s| match s {
                    zgx_wasm::WasmStatus::Exited(code) => *code,
                    _ => None,
                }),
            }
        }
        fn error(&self) -> Option<String> {
            match self {
                Session::Process(app) => app.status().with(|s| match s {
                    AppStatus::Failed(why) => Some(why.clone()),
                    _ => None,
                }),
                #[cfg(feature = "wasm")]
                Session::Wasm(app) => app.status().with(|s| match s {
                    zgx_wasm::WasmStatus::Failed(why) => Some(why.clone()),
                    _ => None,
                }),
            }
        }
        fn close(&self) {
            match self {
                Session::Process(app) => app.close(),
                #[cfg(feature = "wasm")]
                Session::Wasm(app) => app.close(),
            }
        }
        fn revoke(&self) {
            match self {
                Session::Process(app) => app.revoke(),
                #[cfg(feature = "wasm")]
                Session::Wasm(app) => app.revoke(),
            }
        }
        fn is_revoked(&self) -> bool {
            match self {
                Session::Process(app) => app.is_revoked(),
                #[cfg(feature = "wasm")]
                Session::Wasm(app) => app.is_revoked(),
            }
        }
        fn attach(&self, runner: &zgx::compose::TaskRunner) {
            match self {
                Session::Process(app) => app.attach(runner),
                #[cfg(feature = "wasm")]
                Session::Wasm(app) => app.attach(runner),
            }
        }
        fn on_exit(&self, f: impl FnMut(Option<i32>) + 'static) {
            match self {
                Session::Process(app) => app.on_exit(f),
                #[cfg(feature = "wasm")]
                Session::Wasm(app) => app.on_exit(f),
            }
        }
        /// The element showing it (`ui.surface`).
        pub(crate) fn element(&self) -> zgx::compose::View {
            match self {
                Session::Process(app) => zgx_surface::host::surface(app),
                #[cfg(feature = "wasm")]
                Session::Wasm(app) => zgx_wasm::surface(app),
            }
        }
    }

    /// What a script asked to start.
    #[derive(Clone)]
    enum Asked {
        Process(AppSpec),
        Wasm(WasmSpec),
    }

    struct Entry {
        app: Session,
        /// What the script asked to start: vetted again on each reuse.
        asked: Asked,
        handlers: Rc<RefCell<Handlers>>,
    }
    #[derive(Default)]
    struct Handlers {
        vm: Weak<Vm>,
        exit: Option<Function>,
    }

    #[derive(Default)]
    pub(super) struct Sessions {
        entries: RefCell<HashMap<Key, Entry>>,
        runner: RefCell<Option<zgx::compose::TaskRunner>>,
    }
    impl Sessions {
        pub fn set_runner(&self, runner: &zgx::compose::TaskRunner) {
            *self.runner.borrow_mut() = Some(runner.clone());
            for entry in self.entries.borrow().values() {
                entry.app.attach(runner);
            }
        }
        pub fn keep_plugins(&self, kept: &[String]) {
            let gone: Vec<Entry> = {
                let mut entries = self.entries.borrow_mut();
                let keys: Vec<Key> = entries
                    .keys()
                    .filter(|(plugin, _)| {
                        plugin
                            .as_ref()
                            .is_some_and(|p| !kept.iter().any(|k| **k == **p))
                    })
                    .cloned()
                    .collect();
                keys.iter().filter_map(|k| entries.remove(k)).collect()
            };
            for entry in gone {
                entry.app.close();
            }
        }
    }

    /// `ui.app(id)`'s value: a session.
    #[derive(Clone)]
    pub(crate) struct AppUd {
        key: Key,
        pub(crate) app: Session,
        apps: Weak<Apps>,
    }
    impl AppUd {
        /// Its session, unless revoked (then every read is an error).
        fn live(&self) -> mlua::Result<&Session> {
            match self.app.is_revoked() {
                true => Err(mlua::Error::runtime(format!(
                    "ui.app `{}`: revoked",
                    self.key.1
                ))),
                false => Ok(&self.app),
            }
        }
    }
    impl UserData for AppUd {
        fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
            // Tracked: a text or computed reading one follows it.
            fields.add_field_method_get("id", |_, a| Ok(a.key.1.clone()));
            fields.add_field_method_get("status", |_, a| Ok(a.live()?.status()));
            fields.add_field_method_get("title", |_, a| Ok(a.live()?.title()));
            fields.add_field_method_get("pid", |_, a| Ok(a.live()?.pid()));
            fields.add_field_method_get("exit_code", |_, a| Ok(a.live()?.exit_code()));
            fields.add_field_method_get("error", |_, a| Ok(a.live()?.error()));
            fields.add_field_method_get("kind", |_, a| {
                Ok(match &a.app {
                    Session::Process(_) => "process",
                    #[cfg(feature = "wasm")]
                    Session::Wasm(_) => "wasm",
                })
            });
        }
        fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
            methods.add_method("close", |_, a, ()| {
                a.app.close();
                if let Some(apps) = a.apps.upgrade() {
                    let mut entries = apps.sessions.entries.borrow_mut();
                    // Only this session: the id may since name another.
                    if entries.get(&a.key).is_some_and(|e| e.app.ptr_eq(&a.app)) {
                        let removed = entries.remove(&a.key);
                        drop(entries);
                        drop(removed);
                    }
                }
                Ok(())
            });
        }
    }

    /// What `opts` asks for: a program (`command`) or a WebAssembly app
    /// (`wasm`, resolved by the caller), and its exit handler.
    fn options(id: &str, opts: &Table) -> mlua::Result<(Asked, Option<Function>)> {
        for (key, _) in opts.clone().pairs::<String, LuaValue>().flatten() {
            if !matches!(
                key.as_str(),
                "command" | "args" | "cwd" | "env" | "on_exit" | "wasm" | "memory" | "timeout"
            ) {
                return Err(mlua::Error::runtime(format!(
                    "ui.app: unknown option `{key}`"
                )));
            }
        }
        let exit = opts.get::<Option<Function>>("on_exit")?;
        if let Some(module) = opts.get::<Option<String>>("wasm")? {
            for program in ["command", "args", "cwd", "env"] {
                if !opts.get::<LuaValue>(program)?.is_nil() {
                    return Err(mlua::Error::runtime(format!(
                        "ui.app `{id}`: `wasm` runs in this process; `{program}` is a program's"
                    )));
                }
            }
            let mut spec = WasmSpec::new(module);
            if let Some(mib) = opts.get::<Option<f64>>("memory")? {
                if !(1. ..=4096.).contains(&mib) {
                    return Err(mlua::Error::runtime(format!(
                        "ui.app `{id}`: `memory` is MiB from 1 to 4096, not {mib}"
                    )));
                }
                spec.memory_limit = (mib * 1048576.) as u64;
            }
            if let Some(seconds) = opts.get::<Option<f64>>("timeout")? {
                if !(0.01..=60.).contains(&seconds) {
                    return Err(mlua::Error::runtime(format!(
                        "ui.app `{id}`: `timeout` is seconds from 0.01 to 60, not {seconds}"
                    )));
                }
                spec.timeout = Duration::from_secs_f64(seconds);
            }
            return Ok((Asked::Wasm(spec), exit));
        }
        for wasm in ["memory", "timeout"] {
            if !opts.get::<LuaValue>(wasm)?.is_nil() {
                return Err(mlua::Error::runtime(format!(
                    "ui.app `{id}`: `{wasm}` is a WebAssembly app's (`wasm`)"
                )));
            }
        }
        let Some(command) = opts.get::<Option<String>>("command")? else {
            return Err(mlua::Error::runtime(format!(
                "ui.app `{id}`: `command` (or `wasm`) is needed to start it"
            )));
        };
        let mut args = Vec::new();
        if let Some(list) = opts.get::<Option<Table>>("args")? {
            for arg in list.sequence_values::<String>() {
                args.push(arg?);
            }
        }
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let cwd =
            opts.get::<Option<String>>("cwd")?
                .map(|dir| match (dir.strip_prefix('~'), &home) {
                    (Some(rest), Some(home)) => home.join(rest.trim_start_matches('/')),
                    _ => PathBuf::from(dir),
                });
        let mut env = Vec::new();
        if let Some(table) = opts.get::<Option<Table>>("env")? {
            for pair in table.pairs::<String, String>() {
                env.push(pair?);
            }
        }
        env.sort();
        Ok((
            Asked::Process(AppSpec {
                command,
                args,
                cwd,
                env,
            }),
            exit,
        ))
    }

    /// A module's path as a script gave it, from the script's folder: a
    /// plugin's must be a file inside its own folder (links followed).
    fn resolve_module(
        id: &str,
        asked: &Path,
        base: Option<&Path>,
        plugin: bool,
    ) -> mlua::Result<PathBuf> {
        let refuse = |why: String| mlua::Error::runtime(format!("ui.app `{id}`: {why}"));
        let path = match base {
            Some(base) if asked.is_relative() => base.join(asked),
            _ => asked.to_path_buf(),
        };
        let real = std::fs::canonicalize(&path)
            .map_err(|error| refuse(format!("`wasm` {}: {error}", path.display())))?;
        if !real.is_file() {
            return Err(refuse(format!("`wasm` {} is not a file", path.display())));
        }
        if plugin {
            let inside = base
                .and_then(|base| std::fs::canonicalize(base).ok())
                .is_some_and(|base| real.starts_with(base));
            if !inside {
                return Err(refuse(format!(
                    "`wasm` {} is not inside the plugin's folder",
                    asked.display()
                )));
            }
        }
        Ok(real)
    }

    /// What the plugin's grants, the app's guard and the host's hook say of
    /// `asked`: what to start, or the script's error.
    fn check(
        apps: &Apps,
        lua: &Lua,
        plugin: &Option<Rc<str>>,
        id: &str,
        asked: &Asked,
        running: bool,
    ) -> mlua::Result<Asked> {
        let env = env(lua)?;
        match asked {
            Asked::Process(asked) => {
                env.may(Capability::Apps, "ui.app")?;
                crate::rules::guard(lua, crate::GuardKind::App, &asked.to_json())?;
                apps.vet(&AppLaunch {
                    plugin: plugin.as_deref().map(str::to_owned),
                    id: id.to_owned(),
                    asked: asked.clone(),
                    running,
                })
                .map(Asked::Process)
            }
            Asked::Wasm(asked) => {
                // Sandboxed: less than a program may do, so either grant.
                if !env.grants.allows(&Capability::Apps) {
                    env.may(Capability::Wasm, "ui.app { wasm }")?;
                }
                crate::rules::guard(lua, crate::GuardKind::Wasm, &asked.module.to_string_lossy())?;
                apps.vet_wasm(&WasmLaunch {
                    plugin: plugin.as_deref().map(str::to_owned),
                    id: id.to_owned(),
                    asked: asked.clone(),
                    running,
                })
                .map(Asked::Wasm)
            }
        }
        .map_err(|why| mlua::Error::runtime(format!("ui.app `{id}`: {why}")))
    }

    /// A refusal of a session reached again ends it: its program is
    /// killed, its entry goes, and every handle to it is dead (`revoke`).
    fn refused_ends<T>(
        apps: &Apps,
        key: &Key,
        app: &Session,
        checked: mlua::Result<T>,
    ) -> mlua::Result<T> {
        if checked.is_err() {
            let mut entries = apps.sessions.entries.borrow_mut();
            if entries.get(key).is_some_and(|e| e.app.ptr_eq(app)) {
                let removed = entries.remove(key);
                drop(entries);
                drop(removed);
            }
            app.revoke();
        }
        checked
    }

    /// A handle used again (`ui.surface`): its session asked again as a
    /// lookup is; refused, or revoked before, an error.
    fn revalidate(lua: &Lua, ud: &AppUd) -> mlua::Result<()> {
        if ud.app.is_revoked() {
            return Err(mlua::Error::runtime(format!(
                "ui.app `{}`: revoked",
                ud.key.1
            )));
        }
        let Some(apps) = ud.apps.upgrade() else {
            return Ok(());
        };
        let asked = apps
            .sessions
            .entries
            .borrow()
            .get(&ud.key)
            .filter(|e| e.app.ptr_eq(&ud.app))
            .map(|e| e.asked.clone());
        // Closed and forgotten: nothing runs to ask about.
        let Some(asked) = asked else { return Ok(()) };
        let running = !ud.app.ended();
        let checked = check(&apps, lua, &ud.key.0, &ud.key.1, &asked, running);
        refused_ends(&apps, &ud.key, &ud.app, checked).map(drop)
    }

    /// Start what was vetted.
    fn start(runtime: &zgx::reactive::Runtime, start: Asked) -> mlua::Result<Session> {
        Ok(match start {
            Asked::Process(start) => {
                let command = start.command.clone();
                Session::Process(
                    App::spawn(
                        runtime,
                        AppOptions {
                            command: start.command,
                            args: start.args,
                            cwd: start.cwd,
                            env: start.env,
                        },
                    )
                    .unwrap_or_else(|error| App::failed(runtime, format!("{command}: {error}"))),
                )
            }
            #[cfg(feature = "wasm")]
            Asked::Wasm(start) => {
                let mut options = zgx_wasm::WasmOptions::new(start.module)
                    .memory_limit(start.memory_limit)
                    .timeout(start.timeout)
                    .gpu_bytes(start.gpu_bytes)
                    .gpu_objects(start.gpu_objects)
                    .wall_clock(start.wall_clock);
                if let Some(dir) = start.data_dir {
                    options = options.data_dir(dir);
                }
                Session::Wasm(zgx_wasm::WasmApp::spawn(runtime, options))
            }
            #[cfg(not(feature = "wasm"))]
            Asked::Wasm(_) => {
                return Err(mlua::Error::runtime(
                    "ui.app { wasm }: this build runs no WebAssembly apps (zgx-script's `wasm` feature)",
                ));
            }
        })
    }

    fn session(
        apps: &Rc<Apps>,
        lua: &Lua,
        id: String,
        opts: Option<Table>,
    ) -> mlua::Result<Option<AppUd>> {
        let env = env(lua)?;
        let plugin: Option<Rc<str>> = match *env.grants {
            crate::Grants::All => None,
            crate::Grants::Only(_) => Some(env.owner.clone()),
        };
        let key: Key = (plugin.clone(), id.clone());
        let mut parsed = opts.as_ref().map(|opts| options(&id, opts)).transpose()?;
        if let Some((Asked::Wasm(spec), _)) = &mut parsed {
            let base = env.vm.upgrade().and_then(|vm| vm.base.clone());
            spec.module = resolve_module(&id, &spec.module, base.as_deref(), plugin.is_some())?;
        }
        let ud = |app: Session| AppUd {
            key: key.clone(),
            app,
            apps: Rc::downgrade(apps),
        };
        let handlers = |exit: Option<Function>| Handlers {
            vm: env.vm.clone(),
            exit,
        };
        let existing = apps
            .sessions
            .entries
            .borrow()
            .get(&key)
            .map(|e| (e.app.clone(), e.asked.clone(), e.handlers.clone()));
        if let Some((app, asked, entry_handlers)) = existing {
            let ended = app.ended();
            // Ended, and asked with options: started again below.
            if !(ended && parsed.is_some()) {
                // Reached again: what it runs is asked again, so a
                // permission taken back holds.
                refused_ends(
                    apps,
                    &key,
                    &app,
                    check(apps, lua, &plugin, &id, &asked, !ended),
                )?;
                if let Some((_, exit)) = &parsed {
                    *entry_handlers.borrow_mut() = handlers(exit.clone());
                }
                return Ok(Some(ud(app)));
            }
        }
        let Some((asked, exit)) = parsed else {
            return Ok(None);
        };
        let vetted = check(apps, lua, &plugin, &id, &asked, false)?;
        let app = start(&env.runtime, vetted)?;
        if let Some(runner) = apps.sessions.runner.borrow().as_ref() {
            app.attach(runner);
        }
        let handlers = Rc::new(RefCell::new(handlers(exit)));
        {
            let handlers = Rc::downgrade(&handlers);
            app.on_exit(move |code| {
                let Some(handlers) = handlers.upgrade() else {
                    return;
                };
                let (vm, exit) = {
                    let handlers = handlers.borrow();
                    (handlers.vm.upgrade(), handlers.exit.clone())
                };
                if let (Some(vm), Some(exit)) = (vm, exit) {
                    let result = vm.runtime.clone().batch(|| vm.call::<()>(&exit, code));
                    if let Err(error) = result {
                        vm.report(error);
                    }
                }
            });
        }
        let old = apps.sessions.entries.borrow_mut().insert(
            key.clone(),
            Entry {
                app: app.clone(),
                asked,
                handlers,
            },
        );
        drop(old);
        Ok(Some(ud(app)))
    }

    pub(super) fn install(lua: &Lua, ui: &Table, apps: Rc<Apps>) -> mlua::Result<()> {
        ui.set(
            "app",
            lua.create_function(move |lua, (id, opts): (String, Option<Table>)| {
                // Asked again with what it starts (`check`); a lookup needs
                // either grant.
                let env = crate::node::env(lua)?;
                if !env.grants.allows(&Capability::Wasm) {
                    env.may(Capability::Apps, "ui.app")?;
                }
                session(&apps, lua, id, opts)
            })?,
        )?;
        ui.set(
            "surface",
            lua.create_function(|lua, table: Table| {
                let app = match table.get::<LuaValue>("app")? {
                    LuaValue::UserData(ud) if ud.is::<AppUd>() => {
                        let ud = ud.borrow::<AppUd>()?;
                        // Shown again: asked again, as a lookup is.
                        revalidate(lua, &ud)?;
                        ud.app.clone()
                    }
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "ui.surface needs `app = ui.app(id, …)`, not {}",
                            other.type_name()
                        )));
                    }
                };
                table.raw_set("app", LuaValue::Nil)?;
                let node = crate::node::fill(
                    lua,
                    crate::node::Node::new(
                        crate::node::owner(lua)?,
                        crate::node::Kind::Surface(app),
                    ),
                    Some(&table),
                )?;
                crate::node::node_ud(lua, node)
            })?,
        )
    }
}
#[cfg(feature = "apps")]
pub(crate) use running::Session;
