//! Programs scripts start and show: `ui.app(id, opts)` starts one (no
//! shell) with an external surface's connection (zgx-surface) and returns
//! its session; `ui.surface { app = a }` shows its frames, forwarding the
//! element's input to it.
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
//! What may run is the host's: a plugin needs `Capability::Apps`, and the
//! whole spec asked for (command, args, cwd, env) goes to the host's hook
//! (`ScriptHost::vet_apps`), which refuses it or says exactly what to start
//! (a resolved path, pinned arguments, a filtered environment); the app's VM
//! asks its guard too (`GuardKind::App`, named by the spec as JSON). Both
//! are asked again each time a script reaches a running session, so a
//! permission taken back holds from then on.
use mlua::{Lua, Table};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

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

/// The host's sessions, and what it vets them with.
#[derive(Default)]
pub(crate) struct Apps {
    vet: RefCell<Option<Rc<Vet>>>,
    #[cfg(feature = "apps")]
    sessions: running::Sessions,
}
impl Apps {
    pub fn set_vet(&self, vet: Rc<Vet>) {
        *self.vet.borrow_mut() = Some(vet);
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
    use super::{AppLaunch, AppSpec, Apps};
    use crate::{Vm, node::env};
    use mlua::{Function, Lua, Table, UserData, Value as LuaValue};
    use std::{
        cell::RefCell,
        collections::HashMap,
        path::PathBuf,
        rc::{Rc, Weak},
    };
    use zgx_surface::host::{App, AppOptions, AppStatus};

    /// A session by who started it (a plugin's name, or `None` for the
    /// app's scripts) and the id it gave: a script reaches only its own.
    type Key = (Option<Rc<str>>, String);

    struct Entry {
        app: App,
        /// What the script asked to start: vetted again on each reuse.
        asked: AppSpec,
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
        pub(crate) app: App,
        apps: Weak<Apps>,
    }
    impl UserData for AppUd {
        fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
            // Tracked: a text or computed reading one follows it.
            fields.add_field_method_get("id", |_, a| Ok(a.key.1.clone()));
            fields.add_field_method_get("status", |_, a| Ok(a.app.status().with(AppStatus::name)));
            fields.add_field_method_get("title", |_, a| Ok(a.app.title().get()));
            fields.add_field_method_get("pid", |_, a| Ok(a.app.pid()));
            fields.add_field_method_get("exit_code", |_, a| {
                Ok(a.app.status().with(|s| match s {
                    AppStatus::Exited(code) => *code,
                    _ => None,
                }))
            });
            fields.add_field_method_get("error", |_, a| {
                Ok(a.app.status().with(|s| match s {
                    AppStatus::Failed(why) => Some(why.clone()),
                    _ => None,
                }))
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

    fn options(id: &str, opts: &Table) -> mlua::Result<(AppSpec, Option<Function>)> {
        for (key, _) in opts.clone().pairs::<String, LuaValue>().flatten() {
            if !matches!(key.as_str(), "command" | "args" | "cwd" | "env" | "on_exit") {
                return Err(mlua::Error::runtime(format!(
                    "ui.app: unknown option `{key}`"
                )));
            }
        }
        let Some(command) = opts.get::<Option<String>>("command")? else {
            return Err(mlua::Error::runtime(format!(
                "ui.app `{id}`: `command` is needed to start it"
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
            AppSpec {
                command,
                args,
                cwd,
                env,
            },
            opts.get::<Option<Function>>("on_exit")?,
        ))
    }

    /// What the app's guard and the host's hook say of `asked`: the spec
    /// to start, or the script's error.
    fn check(
        apps: &Apps,
        lua: &Lua,
        plugin: &Option<Rc<str>>,
        id: &str,
        asked: &AppSpec,
        running: bool,
    ) -> mlua::Result<AppSpec> {
        crate::rules::guard(lua, crate::GuardKind::App, &asked.to_json())?;
        apps.vet(&AppLaunch {
            plugin: plugin.as_deref().map(str::to_owned),
            id: id.to_owned(),
            asked: asked.clone(),
            running,
        })
        .map_err(|why| mlua::Error::runtime(format!("ui.app `{id}`: {why}")))
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
        let parsed = opts.as_ref().map(|opts| options(&id, opts)).transpose()?;
        let ud = |app: App| AppUd {
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
            let ended = matches!(
                app.status().with_untracked(Clone::clone),
                AppStatus::Exited(_) | AppStatus::Failed(_)
            );
            // Ended, and asked with options: started again below.
            if !(ended && parsed.is_some()) {
                // Reached again: what it runs is asked again, so a
                // permission taken back holds.
                check(apps, lua, &plugin, &id, &asked, !ended)?;
                if let Some((_, exit)) = &parsed {
                    *entry_handlers.borrow_mut() = handlers(exit.clone());
                }
                return Ok(Some(ud(app)));
            }
        }
        let Some((asked, exit)) = parsed else {
            return Ok(None);
        };
        let start = check(apps, lua, &plugin, &id, &asked, false)?;
        let command = start.command.clone();
        let app = App::spawn(
            &env.runtime,
            AppOptions {
                command: start.command,
                args: start.args,
                cwd: start.cwd,
                env: start.env,
            },
        )
        .unwrap_or_else(|error| App::failed(&env.runtime, format!("{command}: {error}")));
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
                crate::node::env(lua)?.may(crate::Capability::Apps, "ui.app")?;
                session(&apps, lua, id, opts)
            })?,
        )?;
        ui.set(
            "surface",
            lua.create_function(|lua, table: Table| {
                let app = match table.get::<LuaValue>("app")? {
                    LuaValue::UserData(ud) if ud.is::<AppUd>() => ud.borrow::<AppUd>()?.app.clone(),
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
