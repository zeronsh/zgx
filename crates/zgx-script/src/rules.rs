//! What a host asks of its scripts beyond their grants: the name prefixes
//! only the app's own VM may use (`ScriptHost::reserve_prefix`), and what is
//! asked before the app's VM reaches the host or the network
//! (`ScriptHost::guard`). Kept here, keyed by host, rather than in the host,
//! whose size a host asking neither would pay for (where the heap places what
//! follows it). Until a host on this thread asks, each check reads one flag.
use crate::value::Value;
use mlua::Lua;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};
use zgx::reactive::Signal;

thread_local! {
    /// Whether a host on this thread has asked for anything here.
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static HOSTS: RefCell<Vec<(Weak<crate::Inner>, Rc<Rules>)>> =
        const { RefCell::new(Vec::new()) };
}

/// What the app's VM is about to reach, as `ScriptHost::guard` is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GuardKind {
    /// A host action (`ui.action`, `ui.invoke`, or `ui.call` of a plain one).
    Action,
    /// An async host action (`ui.call`).
    AsyncAction,
    /// A host function (`ui.call_sync`).
    Function,
    /// A request through the host's fetcher (`ui.fetch`): named by its URL.
    /// Refused, its promise rejects, with the guard's text.
    Fetch,
    /// A remote picture (`ui.image`, `ui.svg`, `bg_image`), through the same
    /// fetcher: named by its URL. Apart from `Fetch` so a host can let
    /// pictures show while keeping scripts off the network, knowing a picture's
    /// URL can carry what a script puts in it.
    Picture,
    /// A connection through the host's connector (`ui.socket`): named by its
    /// URL, asked each time it opens.
    Socket,
    /// A terminal session reached (`ui.term`): named by its command, or the
    /// login shell it runs when it has none. Refused, `ui.term` raises and
    /// nothing starts.
    Term,
    /// What a script types into a terminal (`:input`, `:paste`, `:binding`):
    /// named by the session's id.
    TermInput,
    /// A page in a web view (`ui.web`, `ui.webview`): named by its URL, or
    /// `html`'s base URL (`about:blank` if none). Asked as a script loads one
    /// (`src`, `:load`, `:html`) and again as the platform starts each
    /// navigation (a link, a form, a redirect, a page's script, history); see
    /// `docs/plugins.md` for what no platform shows.
    Web,
    /// The clipboard (`ui.clipboard`): named `read`, `write`, `read_image`
    /// or `write_image`.
    Clipboard,
    /// A system dialog (`ui.open_files`, `ui.save_file`, `ui.open_folder`):
    /// named by the function.
    Dialog,
    /// A picked file read or written (`ui.read_file`, `ui.write_file`): named
    /// by its path.
    File,
    /// A folder the host keeps or a script holds, which asks no one
    /// (`ui.remembered_folder`, by its key; a folder's `:read`, `:write` and
    /// `:file`, by the path).
    Folder,
    /// A system notification (`ui.notify`, by its title) or the app's badge
    /// (`ui.badge`, named `badge`).
    Notification,
    /// A link in text opening in the browser, on a click or its Open Link
    /// item: named by its URL. Any link in the host's view, a plugin's too;
    /// asked only if the host had a guard (or `reserve_prefix`) as its view
    /// was made.
    OpenLink,
    /// A window a script declares (`ui.window`): named by its `id`. Refused,
    /// `ui.window` raises and nothing opens.
    Window,
    /// A value kept across runs (`ui.persist`): named by its key. A script
    /// in the app's VM shares the app's keys. Refused, it starts from what is
    /// saved and what it writes is held, in memory: saved by
    /// `ScriptHost::keep_held_state`, else gone with
    /// `ScriptHost::drop_held_state` or the app. So a preview looks as the app
    /// does and overwrites nothing; it can read what is saved.
    Persist,
}

type Guard = dyn Fn(GuardKind, &str) -> Result<(), String>;

/// A `ui.persist` key the guard refused: what a run holds of it, and what
/// that began as.
pub(crate) struct Held {
    pub value: Signal<Value>,
    pub start: Value,
}
/// A host's rules.
#[derive(Default)]
pub(crate) struct Rules {
    /// Prefixes only the app's VM may use, without their last dot.
    reserved: RefCell<Vec<String>>,
    guard: RefCell<Option<Rc<Guard>>>,
    /// What `ui.persist` keys the guard refused hold, by script and key.
    held: RefCell<HashMap<(String, String), Held>>,
}
impl Rules {
    pub fn reserve(&self, prefix: &str) {
        let prefix = prefix.trim_end_matches('.').to_owned();
        let mut reserved = self.reserved.borrow_mut();
        if !reserved.contains(&prefix) {
            reserved.push(prefix);
        }
    }
    pub fn set_guard(&self, guard: Rc<Guard>) {
        *self.guard.borrow_mut() = Some(guard);
    }
    /// The reserved prefix `name` falls under, if any: `zeron` holds `zeron`
    /// and `zeron.x`, not `zeronish`.
    fn reserving(&self, name: &str) -> Option<String> {
        self.reserved
            .borrow()
            .iter()
            .find(|p| {
                name.strip_prefix(p.as_str())
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
            })
            .cloned()
    }
    /// The values held, let go of.
    pub fn take_held(&self) -> HashMap<(String, String), Held> {
        std::mem::take(&mut *self.held.borrow_mut())
    }
    /// What the guard says of reaching `name`. It runs with nothing here
    /// borrowed: it may set another.
    pub fn check(&self, kind: GuardKind, name: &str) -> Result<(), String> {
        let guard = self.guard.borrow().clone();
        match guard {
            Some(guard) => guard(kind, name),
            None => Ok(()),
        }
    }
}

/// `host`'s rules, if it has asked for any.
pub(crate) fn of(host: &Rc<crate::Inner>) -> Option<Rc<Rules>> {
    if !ACTIVE.get() {
        return None;
    }
    HOSTS.with_borrow(|hosts| {
        hosts
            .iter()
            .find(|(h, _)| h.strong_count() > 0 && std::ptr::eq(h.as_ptr(), Rc::as_ptr(host)))
            .map(|(_, rules)| rules.clone())
    })
}
/// `host`'s rules, made if it had none.
pub(crate) fn start(host: &Rc<crate::Inner>) -> Rc<Rules> {
    if let Some(rules) = of(host) {
        return rules;
    }
    let rules = Rc::new(Rules::default());
    attach(host, rules.clone());
    rules
}
/// `host` has `rules`: a host configured later is a new `Rc`
/// (`ScriptHost::configure`), and what was asked of the old one stays asked.
pub(crate) fn attach(host: &Rc<crate::Inner>, rules: Rc<Rules>) {
    ACTIVE.set(true);
    HOSTS.with_borrow_mut(|hosts| {
        // Hosts gone: their rules go with them.
        hosts.retain(|(h, _)| h.strong_count() > 0);
        hosts.push((Rc::downgrade(host), rules));
    });
}

/// The rules of the app's VM (its app data): what `guard` asks.
pub(crate) struct Guarded(pub Rc<Rules>);
/// The rules of a plugin's VM (its app data): what it may not name.
pub(crate) struct Reserving(pub Rc<Rules>);

/// Whether the app's VM may reach `name`, from the host's guard. Otherwise
/// (no host asked, or a plugin's VM) one thread-local read.
#[inline]
pub(crate) fn guard(lua: &Lua, kind: GuardKind, name: &str) -> mlua::Result<()> {
    if !ACTIVE.get() {
        return Ok(());
    }
    guarded(lua, kind, name)
}
#[cold]
#[inline(never)]
fn guarded(lua: &Lua, kind: GuardKind, name: &str) -> mlua::Result<()> {
    let Some(rules) = lua.app_data_ref::<Guarded>().map(|g| g.0.clone()) else {
        return Ok(());
    };
    rules
        .check(kind, name)
        .map_err(|why| mlua::Error::runtime(format!("{kind:?} `{name}`: {why}")))
}

/// Whether a navigation the platform is about to start may (`WebView`'s
/// navigation policy, for a web view of the app's VM). Refused, it says so
/// where a script's error shows. `about:blank` reaches nothing, and is what
/// a page given as `html` loads under, asked as it was given.
pub(crate) fn navigation_allowed(
    vm: &Weak<crate::Vm>,
    request: &zgx::web_view::NavigationRequest,
) -> bool {
    if !ACTIVE.get() || request.url == "about:blank" {
        return true;
    }
    let Some(vm) = vm.upgrade() else {
        return true;
    };
    match guarded(&vm.lua, GuardKind::Web, &request.url) {
        Ok(()) => true,
        Err(error) => {
            vm.report(error);
            false
        }
    }
}
/// `ui.term`'s question: the session's command, or the shell it would run.
/// Otherwise one thread-local read.
#[inline]
pub(crate) fn guard_term(lua: &Lua, command: Option<&str>) -> mlua::Result<()> {
    if !ACTIVE.get() {
        return Ok(());
    }
    let shell;
    let name = match command {
        Some(command) => command,
        None => {
            shell = ["SHELL", "COMSPEC"]
                .into_iter()
                .find_map(|name| std::env::var(name).ok())
                .unwrap_or_else(|| "shell".to_owned());
            &shell
        }
    };
    guarded(lua, GuardKind::Term, name)
}

/// The state `ui.persist` gives where the app VM's guard refuses `key`: what
/// is saved (or `initial`), held here so its writes reach nothing saved.
/// Otherwise (no host asked, a plugin's VM, allowed) `None`, for one
/// thread-local read when no host asked.
#[inline]
pub(crate) fn held_persist(
    lua: &Lua,
    env: &crate::node::Env,
    key: &str,
    initial: &Value,
) -> Option<Signal<Value>> {
    if !ACTIVE.get() {
        return None;
    }
    held_persist_asked(lua, env, key, initial)
}
#[cold]
#[inline(never)]
fn held_persist_asked(
    lua: &Lua,
    env: &crate::node::Env,
    key: &str,
    initial: &Value,
) -> Option<Signal<Value>> {
    let rules = lua.app_data_ref::<Guarded>()?.0.clone();
    let why = rules.check(GuardKind::Persist, key).err()?;
    let id = (env.persist_as.to_string(), key.to_owned());
    if let Some(held) = rules.held.borrow().get(&id) {
        return Some(held.value.clone());
    }
    crate::warning(
        Some(&env.warnings),
        format!("ui.persist `{key}`: held, not saved: {why}"),
    );
    let start = env
        .persist
        .saved(&id.0, key)
        .unwrap_or_else(|| initial.clone());
    let value = env.runtime.signal(start.clone());
    let held = Held {
        value: value.clone(),
        start,
    };
    rules.held.borrow_mut().insert(id, held);
    Some(value)
}

/// What a host's view is told for its links, if it has rules: a refusal
/// goes where a script's error shows.
pub(crate) fn link_guard(
    rules: Rc<Rules>,
    error: zgx::reactive::Signal<Option<String>>,
) -> zgx::platform::LinkGuard {
    zgx::platform::LinkGuard(Rc::new(move |url| {
        match rules.check(GuardKind::OpenLink, url) {
            Ok(()) => true,
            Err(why) => {
                crate::report(&error, format!("OpenLink `{url}`: {why}"));
                false
            }
        }
    }))
}

/// Refuses a plugin's use of a name a host reserved (`what` says which).
pub(crate) fn not_reserved(lua: &Lua, owner: &str, name: &str, what: &str) -> mlua::Result<()> {
    if !ACTIVE.get() {
        return Ok(());
    }
    let Some(rules) = lua.app_data_ref::<Reserving>().map(|r| r.0.clone()) else {
        return Ok(());
    };
    match rules.reserving(name) {
        Some(prefix) => Err(mlua::Error::runtime(format!(
            "{what} `{name}`: `{prefix}.` names are the app's alone, not the plugin `{owner}`'s"
        ))),
        None => Ok(()),
    }
}

/// `ui.slot(name)` in a plugin's VM: it offers its own names alone,
/// `<owner>.…`, so it can't pose as a slot of the app's or another plugin's
/// and take what is contributed there.
#[cold]
#[inline(never)]
pub(crate) fn plugin_offers(lua: &Lua, owner: &str, name: &str) -> mlua::Result<()> {
    not_reserved(lua, owner, name, "ui.slot")?;
    let own = name
        .strip_prefix(owner)
        .is_some_and(|rest| rest.starts_with('.') && rest.len() > 1);
    if own {
        return Ok(());
    }
    Err(mlua::Error::runtime(format!(
        "ui.slot `{name}`: the plugin `{owner}` offers only slots of its own, named `{owner}.…`"
    )))
}

/// What a plugin needs for a shortcut that is a key alone
/// (`Capability::Named`).
pub(crate) const BARE_KEYS: &str = "zgx.command.bare_keys";

/// Whether shortcut `key` takes a key from typing: no modifier, or Shift
/// alone on a character (which types its capital). A plugin's would take that
/// key from every field, page and terminal.
pub(crate) fn is_bare(key: &str) -> bool {
    let mut parts: Vec<String> = key.split('+').map(|p| p.trim().to_lowercase()).collect();
    let key = parts.pop().unwrap_or_default();
    match parts.as_slice() {
        [] => true,
        [only] if only == "shift" => key.chars().count() == 1,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_key_has_no_modifier_or_shift_alone_on_a_character() {
        for key in ["k", "K", "/", "enter", "f5", "shift+k", "Shift + /"] {
            assert!(is_bare(key), "{key}");
        }
        for key in ["cmd+k", "ctrl+shift+k", "alt+k", "shift+f5", "shift+enter"] {
            assert!(!is_bare(key), "{key}");
        }
    }

    #[test]
    fn a_reserved_prefix_holds_its_name_and_what_is_under_it() {
        let rules = Rules::default();
        rules.reserve("zeron.");
        assert!(rules.reserving("zeron").is_some());
        assert!(rules.reserving("zeron.sidebar").is_some());
        assert!(rules.reserving("zeronish").is_none());
        assert!(rules.reserving("git.zeron").is_none());
    }
}
