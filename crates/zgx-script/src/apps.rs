//! Programs scripts start and show: `ui.app(id, opts)` starts one (no
//! shell) with an external surface's connection (zgx-surface) and returns
//! its session; `ui.surface { app = a }` shows its frames, forwarding the
//! element's input to it.
//!
//! Sessions belong to the host, as terminals do: reloading keeps every
//! program running, and the run that asks for `id` again gets the same one
//! (its `on_exit` replaces the last run's). A plugin's ids are its own (two
//! plugins' `"game"` are two sessions). A program ends with `app:close()`,
//! the host, or its plugin's removal (`ScriptHost::set_plugins`); one that
//! exited starts again when `ui.app` is given options for it.
//!
//! A plugin starts programs only if granted `Capability::Apps`, and every
//! start (the app's own too) is asked of the host's vetting hook
//! (`ScriptHost::vet_apps`) if it set one; the app's VM asks its guard too
//! (`GuardKind::App`, by the command).
use mlua::{Lua, Table};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

/// What a script asks to start, as the host's vetting hook
/// (`ScriptHost::vet_apps`) is told.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppLaunch {
    /// The plugin asking, by its name; `None` for the app's own scripts.
    pub plugin: Option<String>,
    /// The session's id, as the script gave it.
    pub id: String,
    /// The program, as given (a path, or a name looked up on `PATH`).
    pub command: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Where it starts, if given.
    pub cwd: Option<PathBuf>,
    /// The variables it adds to the environment.
    pub env: Vec<(String, String)>,
}

pub(crate) type Vet = dyn Fn(&AppLaunch) -> Result<(), String>;

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
    #[cfg(feature = "apps")]
    fn vet(&self, launch: &AppLaunch) -> Result<(), String> {
        let vet = self.vet.borrow().clone();
        match vet {
            Some(vet) => vet(launch),
            None => Ok(()),
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
    use super::{AppLaunch, Apps};
    use crate::{Vm, node::env};
    use mlua::{Function, Lua, Table, UserData, Value as LuaValue};
    use std::{
        cell::RefCell,
        collections::HashMap,
        path::PathBuf,
        rc::{Rc, Weak},
    };
    use zgx_surface::host::{App, AppOptions, AppStatus};

    struct Entry {
        app: App,
        /// The plugin that started it; `None` for the app's.
        plugin: Option<Rc<str>>,
        handlers: Rc<RefCell<Handlers>>,
    }
    #[derive(Default)]
    struct Handlers {
        vm: Weak<Vm>,
        exit: Option<Function>,
    }

    #[derive(Default)]
    pub(super) struct Sessions {
        entries: RefCell<HashMap<String, Entry>>,
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
                let keys: Vec<String> = entries
                    .iter()
                    .filter(|(_, e)| {
                        e.plugin
                            .as_ref()
                            .is_some_and(|p| !kept.iter().any(|k| **k == **p))
                    })
                    .map(|(k, _)| k.clone())
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
        id: String,
        key: String,
        pub(crate) app: App,
        apps: Weak<Apps>,
    }
    impl UserData for AppUd {
        fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
            // Tracked: a text or computed reading one follows it.
            fields.add_field_method_get("id", |_, a| Ok(a.id.clone()));
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
                    let removed = apps.sessions.entries.borrow_mut().remove(&a.key);
                    drop(removed);
                }
                Ok(())
            });
        }
    }

    fn options(id: &str, opts: &Table) -> mlua::Result<(AppOptions, Option<Function>)> {
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
            AppOptions {
                command,
                args,
                cwd,
                env,
            },
            opts.get::<Option<Function>>("on_exit")?,
        ))
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
        // A plugin's ids are its own.
        let key = match &plugin {
            Some(plugin) => format!("{plugin}/{id}"),
            None => id.clone(),
        };
        let parsed = opts.as_ref().map(|opts| options(&id, opts)).transpose()?;
        let ud = |app: App| AppUd {
            id: id.clone(),
            key: key.clone(),
            app,
            apps: Rc::downgrade(apps),
        };
        let handlers = |exit: Option<Function>| Handlers {
            vm: env.vm.clone(),
            exit,
        };
        {
            let entries = apps.sessions.entries.borrow();
            if let Some(entry) = entries.get(&key) {
                let ended = matches!(
                    entry.app.status().with_untracked(Clone::clone),
                    AppStatus::Exited(_) | AppStatus::Failed(_)
                );
                match &parsed {
                    // A lookup, or a run asking again for one still going:
                    // the same session, the run's handler replacing the last.
                    None => return Ok(Some(ud(entry.app.clone()))),
                    Some((_, exit)) if !ended => {
                        *entry.handlers.borrow_mut() = handlers(exit.clone());
                        return Ok(Some(ud(entry.app.clone())));
                    }
                    // Ended, and asked with options: started again.
                    Some(_) => {}
                }
            }
        }
        let Some((options, exit)) = parsed else {
            return Ok(None);
        };
        crate::rules::guard(lua, crate::GuardKind::App, &options.command)?;
        apps.vet(&AppLaunch {
            plugin: plugin.as_deref().map(str::to_owned),
            id: id.clone(),
            command: options.command.clone(),
            args: options.args.clone(),
            cwd: options.cwd.clone(),
            env: options.env.clone(),
        })
        .map_err(|why| mlua::Error::runtime(format!("ui.app `{id}`: {why}")))?;
        let command = options.command.clone();
        let app = App::spawn(&env.runtime, options)
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
                plugin,
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
