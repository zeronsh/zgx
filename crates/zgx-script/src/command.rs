//! App commands scripts declare: `ui.command { id, title, menu, key, run }`.
//!
//! Each run declares its commands afresh; after a successful load the host
//! publishes them ([`crate::ScriptHost::commands`]) for the application to
//! show as menu items with key equivalents, and calls them back with
//! [`crate::ScriptHost::run_command`]. Menus reach the app wherever focus is,
//! a terminal or a web page included.
//!
//! Every script's commands share one list, their ids too. A shortcut the
//! root script's commands (or an earlier script's) have stays theirs, so the
//! menu bar never has a key twice.
use crate::{Vm, node::env};
use mlua::{Function, Lua, Table};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};
use zgx::reactive::{Effect, Runtime, Signal};

/// A command as the application shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    /// Unique; empty for a separator.
    pub id: String,
    /// What the menu item says.
    pub title: String,
    /// The menu it goes in ("File", "Edit", "View", "Window", or any other).
    pub menu: String,
    /// Its shortcut: modifiers and a key joined by `+`, as `cmd+shift+d`,
    /// `cmd+alt+arrowleft`, `cmd+1`, `ctrl+tab`.
    pub key: Option<String>,
    /// A separator line, not a command.
    pub separator: bool,
}

/// A command's shortcut zgx dropped because a command declared before it
/// holds the same one (`Shift+Cmd+D` is `cmd+shift+d`), as
/// `ScriptHost::dropped_shortcuts` reports it: the command stays, without
/// the key, so the app can tell the script's author (or the person
/// rebinding) why.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct DroppedShortcut {
    /// The script beside the root whose command lost the key; empty for the
    /// root script's.
    pub script: String,
    /// The command's id.
    pub command: String,
    /// The shortcut as the script wrote it.
    pub key: String,
    /// "taken by the root script", or "taken by the script `<name>`".
    pub reason: String,
    /// The id of the command that holds the shortcut.
    pub holder: String,
}

type Runner = (Function, Weak<Vm>);
/// A declared command, its runner and the script beside the root that
/// declared it (none for the root script's).
type Pending = (Command, Option<Runner>, Option<Rc<str>>);
/// A shortcut that follows what it reads (`key = state` or a function):
/// the list is published again as it changes, no reload needed.
#[derive(Clone)]
enum KeySource {
    State(Signal<crate::Value>),
    Function(Function, Weak<Vm>),
}
impl KeySource {
    /// The shortcut now (tracked: read in the commands' effect).
    fn read(&self) -> Option<String> {
        match self {
            KeySource::State(state) => match state.get() {
                crate::Value::Str(key) if !key.is_empty() => Some(key.to_string()),
                _ => None,
            },
            KeySource::Function(f, vm) => {
                let vm = vm.upgrade()?;
                match vm.call::<Option<String>>(f, ()) {
                    Ok(key) => key.filter(|key| !key.is_empty()),
                    Err(error) => {
                        vm.report(error);
                        None
                    }
                }
            }
        }
    }
}
/// What a load declared, kept to publish again as dynamic shortcuts change.
struct Declared {
    commands: Vec<(Command, Option<Rc<str>>)>,
    /// The commands whose shortcut follows what it reads, by index.
    keys: Vec<(usize, KeySource)>,
    reserved: Vec<String>,
}
pub(crate) struct Commands {
    pending: RefCell<Vec<Pending>>,
    /// Pending commands' dynamic shortcuts, by id.
    pending_keys: RefCell<Vec<(String, KeySource)>>,
    /// While a load's commands have dynamic shortcuts: what publishes them
    /// again as those change.
    following: RefCell<Option<Effect>>,
    runtime: Runtime,
    /// The ids of the commands declared `reserved` in the load under way.
    pending_reserved: RefCell<Vec<String>>,
    runners: RefCell<HashMap<String, Runner>>,
    pub list: Signal<Vec<Command>>,
    /// The shortcuts the last publish dropped, set before `list`.
    pub dropped: RefCell<Vec<DroppedShortcut>>,
    /// The shortcuts of the commands declared `reserved`: the app's whatever
    /// has the keyboard (`ScriptHost::reserved_keys`).
    pub reserved: Signal<Vec<String>>,
}
impl Commands {
    pub fn new(runtime: &Runtime) -> Self {
        Self {
            pending: RefCell::default(),
            pending_keys: RefCell::default(),
            following: RefCell::default(),
            runtime: runtime.clone(),
            pending_reserved: RefCell::default(),
            runners: RefCell::default(),
            dropped: RefCell::default(),
            list: runtime.signal(Vec::new()),
            reserved: runtime.signal(Vec::new()),
        }
    }
    /// A load begins: what it declares replaces what is published.
    pub fn begin(&self) {
        self.pending.borrow_mut().clear();
        self.pending_keys.borrow_mut().clear();
        self.pending_reserved.borrow_mut().clear();
    }
    /// Where the load's declarations are, before a script runs.
    pub fn mark(&self) -> (usize, usize, usize) {
        (
            self.pending.borrow().len(),
            self.pending_keys.borrow().len(),
            self.pending_reserved.borrow().len(),
        )
    }
    /// A script failed as it loaded: what it declared since `mark` goes, so
    /// no command of a failed script stays in the menu bar unable to run.
    pub fn discard_since(&self, (commands, keys, reserved): (usize, usize, usize)) {
        self.pending.borrow_mut().truncate(commands);
        self.pending_keys.borrow_mut().truncate(keys);
        self.pending_reserved.borrow_mut().truncate(reserved);
    }
    /// The load succeeded: publish what it declared, and again whenever a
    /// shortcut that follows a state changes.
    pub fn publish(self: &Rc<Self>) {
        let pending = std::mem::take(&mut *self.pending.borrow_mut());
        let mut dynamic = std::mem::take(&mut *self.pending_keys.borrow_mut());
        let mut runners = HashMap::new();
        let mut declared = Declared {
            commands: Vec::with_capacity(pending.len()),
            keys: Vec::new(),
            reserved: std::mem::take(&mut *self.pending_reserved.borrow_mut()),
        };
        for (command, runner, script) in pending {
            if let Some(runner) = runner {
                runners.insert(command.id.clone(), runner);
            }
            if !dynamic.is_empty()
                && let Some(at) = dynamic.iter().position(|(id, _)| *id == command.id)
            {
                let (_, source) = dynamic.swap_remove(at);
                declared.keys.push((declared.commands.len(), source));
            }
            declared.commands.push((command, script));
        }
        *self.runners.borrow_mut() = runners;
        // The last load's following, if any, is done with.
        drop(self.following.borrow_mut().take());
        if declared.keys.is_empty() {
            // Published once, as declared.
            let pending = declared
                .commands
                .into_iter()
                .map(|(command, script)| (command, None, script))
                .collect();
            self.publish_list(pending, &declared.reserved, false);
            return;
        }
        let commands = Rc::downgrade(self);
        let effect = self.runtime.effect(move || {
            if let Some(commands) = commands.upgrade() {
                commands.publish_declared(&declared);
            }
        });
        *self.following.borrow_mut() = Some(effect);
    }
    /// Publish `declared`, its dynamic shortcuts read now. A shortcut a
    /// command before it holds is dropped (and said so in `dropped`): the
    /// root script's first, then each other script's, so neither another
    /// script nor a rebinding gives the menu bar a key twice.
    fn publish_declared(&self, declared: &Declared) {
        let mut pending: Vec<Pending> = declared
            .commands
            .iter()
            .map(|(command, script)| (command.clone(), None, script.clone()))
            .collect();
        for (at, source) in &declared.keys {
            pending[*at].0.key = source.read();
        }
        self.publish_list(pending, &declared.reserved, true);
    }
    /// Publish `pending`. A `rebinding`'s key may be one an earlier command
    /// holds: the root script's own are checked then too (as declared, they
    /// are the root's to keep apart).
    fn publish_list(&self, mut pending: Vec<Pending>, reserved: &[String], rebinding: bool) {
        let dropped = match rebinding || pending.iter().any(|(_, _, script)| script.is_some()) {
            false => Vec::new(),
            true => drop_taken(&mut pending, rebinding),
        };
        if *self.dropped.borrow() != dropped {
            *self.dropped.borrow_mut() = dropped;
        }
        let list: Vec<Command> = pending.into_iter().map(|(command, _, _)| command).collect();
        let reserved: Vec<String> = list
            .iter()
            .filter(|command| reserved.contains(&command.id))
            .filter_map(|command| command.key.clone())
            .collect();
        if self.list.with_untracked(|old| *old != list) {
            self.list.set(list);
        }
        if self.reserved.with_untracked(|old| *old != reserved) {
            self.reserved.set(reserved);
        }
    }
    /// The command holding shortcut `keys` now, if any.
    pub fn owner(&self, keys: &str) -> Option<String> {
        let wanted = shortcut(keys);
        self.list.with_untracked(|list| {
            list.iter()
                .find(|command| {
                    command
                        .key
                        .as_deref()
                        .is_some_and(|key| shortcut(key) == wanted)
                })
                .map(|command| command.id.clone())
        })
    }
    /// Run the command `id`; whether one ran.
    pub fn run(&self, id: &str) -> bool {
        let runner = self.runners.borrow().get(id).cloned();
        let Some((f, vm)) = runner else {
            return false;
        };
        let Some(vm) = vm.upgrade() else {
            return false;
        };
        if let Err(error) = vm
            .runtime
            .clone()
            .batch(|| vm.call_as::<()>("the command's run function", &f, ()))
        {
            vm.report(error);
        }
        true
    }
}

/// Take each command's shortcut that a command before it holds: the root
/// script's shortcuts first, then each other script's not yet taken. What
/// was taken.
#[cold]
#[inline(never)]
fn drop_taken(pending: &mut [Pending], apps_too: bool) -> Vec<DroppedShortcut> {
    // Shortcut → the id of its command and the script that declared it.
    let mut taken: HashMap<String, (String, Option<Rc<str>>)> = HashMap::new();
    let mut dropped = Vec::new();
    // The root's first, in order: one rebound to a key an earlier one has
    // loses it (`apps_too`; otherwise both keep it, as declared).
    for (command, _, script) in pending.iter_mut() {
        let (None, Some(key)) = (&script, &command.key) else {
            continue;
        };
        match taken.entry(shortcut(key)) {
            std::collections::hash_map::Entry::Vacant(free) => {
                free.insert((command.id.clone(), None));
            }
            std::collections::hash_map::Entry::Occupied(_) if !apps_too => {}
            std::collections::hash_map::Entry::Occupied(held) => {
                dropped.push(DroppedShortcut {
                    script: String::new(),
                    command: command.id.clone(),
                    key: command.key.take().unwrap_or_default(),
                    reason: format!("taken by the command `{}`", held.get().0),
                    holder: held.get().0.clone(),
                });
            }
        }
    }
    for (command, _, script) in pending.iter_mut() {
        let (Some(script), Some(key)) = (script, &command.key) else {
            continue;
        };
        match taken.entry(shortcut(key)) {
            std::collections::hash_map::Entry::Vacant(free) => {
                free.insert((command.id.clone(), Some(script.clone())));
            }
            std::collections::hash_map::Entry::Occupied(held) => {
                let (holder, by) = held.get();
                dropped.push(DroppedShortcut {
                    script: script.to_string(),
                    command: command.id.clone(),
                    key: command.key.take().unwrap_or_default(),
                    reason: match by {
                        None => "taken by the root script".to_string(),
                        Some(by) => format!("taken by the script `{by}`"),
                    },
                    holder: holder.clone(),
                });
            }
        }
    }
    dropped
}

/// A shortcut as compared: `Shift+Cmd+D` is `cmd+shift+d`.
fn shortcut(key: &str) -> String {
    let mut parts: Vec<String> = key.split('+').map(|p| p.trim().to_lowercase()).collect();
    let key = parts.pop().unwrap_or_default();
    parts.sort();
    parts.dedup();
    parts.push(key);
    parts.join("+")
}

/// Installs `ui.command`, and `ui.shortcut_text`: keys (`cmd+z`) as this
/// platform shows them (⌘Z, or Ctrl+Z), for a tooltip or a list of keys.
pub(crate) fn install(lua: &Lua, ui: &Table, commands: Rc<Commands>) -> mlua::Result<()> {
    ui.set(
        "shortcut_text",
        lua.create_function(|_, keys: String| Ok(crate::lower::shortcut_label(&keys)))?,
    )?;

    // Made once: a `&str` key is a new string pushed each declaration.
    let reserved_key = lua.create_string("reserved")?;
    ui.set(
        "command",
        lua.create_function(move |lua, table: Table| {
            crate::reloads::per_load(lua, "ui.command");
            let menu = table
                .get::<Option<String>>("menu")?
                .unwrap_or_else(|| "File".into());
            if table.get::<Option<bool>>("separator")?.unwrap_or(false) {
                commands.pending.borrow_mut().push((
                    Command {
                        id: String::new(),
                        title: String::new(),
                        menu,
                        key: None,
                        separator: true,
                    },
                    None,
                    None,
                ));
                return Ok(());
            }
            let env = env(lua)?;
            let script = (!env.root).then(|| env.owner.clone());
            let id: String = table
                .get::<Option<String>>("id")?
                .ok_or_else(|| mlua::Error::runtime("ui.command needs an `id`"))?;
            let title = table
                .get::<Option<String>>("title")?
                .unwrap_or_else(|| id.clone());
            let run: Function = table
                .get::<Option<Function>>("run")?
                .ok_or_else(|| mlua::Error::runtime(format!("ui.command `{id}` needs `run`")))?;
            // A shortcut, or a state or function saying it: one that
            // follows what it reads is published again as that changes.
            let (key, source) = match table.get::<mlua::Value>("key")? {
                mlua::Value::Nil => (None, None),
                mlua::Value::String(key) => (Some(key.to_str()?.to_string()), None),
                mlua::Value::Function(f) => (None, Some(KeySource::Function(f, env.vm.clone()))),
                state => match crate::node::state_of(&state) {
                    Some(state) => (None, Some(KeySource::State(state))),
                    None => {
                        return Err(mlua::Error::runtime(format!(
                            "ui.command `{id}`: `key` is a shortcut (\"cmd+k\"), a state or a function"
                        )));
                    }
                },
            };
            let reserved = table.get::<Option<bool>>(&reserved_key)?.unwrap_or(false);
            let vm = env.vm.clone();
            let mut pending = commands.pending.borrow_mut();
            if pending.iter().any(|(c, _, _)| c.id == id) {
                return Err(mlua::Error::runtime(format!(
                    "ui.command `{id}` is declared twice"
                )));
            }
            if reserved {
                commands.pending_reserved.borrow_mut().push(id.clone());
            }
            if let Some(source) = source {
                commands
                    .pending_keys
                    .borrow_mut()
                    .push((id.clone(), source));
            }
            pending.push((
                Command {
                    id,
                    title,
                    menu,
                    key,
                    separator: false,
                },
                Some((run, vm)),
                script,
            ));
            Ok(())
        })?,
    )
}

/// Installs `ui.shortcut_owner(keys)`: the id of the command (any script's)
/// holding a shortcut now, for a rebinding to show a conflict.
/// Apart from `ui.command`: made only for a script that asks.
pub(crate) fn install_owner(lua: &Lua, ui: &Table, commands: Rc<Commands>) -> mlua::Result<()> {
    ui.set(
        "shortcut_owner",
        lua.create_function(move |_, keys: String| Ok(commands.owner(&keys)))?,
    )
}
