//! What a reload keeps (#1369). While a host watches its files, each load
//! records its modules: what each requires, the globals its top level sets,
//! what it starts there that a kept virtual machine couldn't carry over yet
//! (`top_level`), and what it declares for each load (`per_load`). Before
//! the next load, the reload decides from that record (`judge`): keep the
//! app's virtual machine and the modules a save didn't change, running
//! again only what changed and what requires it (a partial reload), or load
//! everything again, and why (`zgx inspect reloads`).
//!
//! Until it's proven on real apps a reload keeps nothing unless asked
//! (`ZGX_RELOAD=partial`, `ScriptHost::reload_partially`): otherwise every
//! reload loads everything again, and the record says what it would have
//! kept. A host that never watches records nothing.
use std::{
    any::Any,
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
};

thread_local! {
    /// Whether a host on this thread records its loads: until one does,
    /// `top_level` and `of` read this alone.
    static RECORDING: Cell<bool> = const { Cell::new(false) };
    /// Each host on this thread that records its loads, with its record:
    /// kept here rather than in the host, whose size a host that never
    /// watches would pay for (where the heap places what follows it).
    static HOSTS: RefCell<Vec<(std::rc::Weak<crate::Inner>, Rc<Reloads>)>> =
        const { RefCell::new(Vec::new()) };
}

/// Whether a host on this thread records its loads: one thread-local read,
/// before anything else is looked at.
#[inline]
pub(crate) fn recording() -> bool {
    RECORDING.get()
}
/// `host`'s record, if it records its loads (`start`).
pub(crate) fn of(host: &Rc<crate::Inner>) -> Option<Rc<Reloads>> {
    if !RECORDING.get() {
        return None;
    }
    HOSTS.with_borrow(|hosts| {
        hosts
            .iter()
            .find(|(h, _)| h.strong_count() > 0 && std::ptr::eq(h.as_ptr(), Rc::as_ptr(host)))
            .map(|(_, reloads)| reloads.clone())
    })
}
/// From `host`'s next load on, its loads are recorded (`ScriptHost::watch`).
pub(crate) fn start(host: &Rc<crate::Inner>) -> Rc<Reloads> {
    RECORDING.set(true);
    if let Some(reloads) = of(host) {
        return reloads;
    }
    let reloads = Rc::new(Reloads::default());
    reloads.on.set(true);
    reloads
        .keep
        .set(zgx::env::var("ZGX_RELOAD").is_ok_and(|v| v == "partial"));
    HOSTS.with_borrow_mut(|hosts| {
        // Hosts gone: their records go with them.
        hosts.retain(|(h, _)| h.strong_count() > 0);
        hosts.push((Rc::downgrade(host), reloads.clone()));
    });
    reloads
}

/// The app's require cache: each module's path and what it returned
/// (`None` while it runs).
pub(crate) type Cache = Rc<RefCell<HashMap<PathBuf, Option<mlua::Value>>>>;

/// What a VM whose loads are recorded keeps for a partial reload (its app
/// data): the tracker, its require cache, and what each module's top level
/// made to last as long as it (an effect, a timer: `crate::node::retain`),
/// which goes when the module runs again.
pub(crate) struct Tracked {
    pub tracker: Rc<Tracker>,
    pub cache: RefCell<Option<Cache>>,
    owned: RefCell<HashMap<PathBuf, Vec<Box<dyn Any>>>>,
}

/// The modules' resources set aside while they run again (`Tracked::set_aside`).
pub(crate) type SetAside = Vec<(PathBuf, Vec<Box<dyn Any>>)>;

impl Tracked {
    pub fn new(tracker: Rc<Tracker>) -> Self {
        Self {
            tracker,
            cache: RefCell::default(),
            owned: RefCell::default(),
        }
    }
    /// Keeps `resource` as long as the module whose top level is running,
    /// or hands it back when none is.
    pub fn retain(&self, resource: Box<dyn Any>) -> Result<(), Box<dyn Any>> {
        match self.tracker.current() {
            Some(module) => {
                self.owned
                    .borrow_mut()
                    .entry(module)
                    .or_default()
                    .push(resource);
                Ok(())
            }
            None => Err(resource),
        }
    }
    /// What `modules` made, out of the way of their next run: dropped once
    /// it succeeds, put back if it fails (`put_back`).
    pub fn set_aside(&self, modules: &[PathBuf]) -> SetAside {
        let mut owned = self.owned.borrow_mut();
        modules
            .iter()
            .filter_map(|m| owned.remove(m).map(|r| (m.clone(), r)))
            .collect()
    }
    /// A partial reload failed: what `modules` made as they ran again goes,
    /// and what they made before comes back.
    pub fn put_back(&self, modules: &[PathBuf], aside: SetAside) {
        let made: Vec<_> = {
            let mut owned = self.owned.borrow_mut();
            modules.iter().filter_map(|m| owned.remove(m)).collect()
        };
        // Dropped outside the borrow: a cleanup may retain again.
        drop(made);
        self.owned.borrow_mut().extend(aside);
    }
}

/// `api`, called at a top level (no component building, no effect running),
/// started something nothing carries over to a kept VM yet (a task, an
/// animation, an `on_open` handler, actions, a router): recorded for the
/// module running, while its host records its loads. Otherwise one
/// thread-local read.
#[inline]
pub(crate) fn top_level(lua: &mlua::Lua, api: &'static str) {
    if RECORDING.get() {
        record_top_level(lua, api, false);
    }
}

/// `api`, called at a top level, declared something for this load alone (a
/// command, a window, a contribution, an `on_mount`): what a module kept
/// wouldn't declare again. Otherwise one thread-local read.
#[inline]
pub(crate) fn per_load(lua: &mlua::Lua, api: &'static str) {
    if RECORDING.get() {
        record_top_level(lua, api, true);
    }
}

/// `api` was called while its host records its loads: the module running
/// recorded it, if at a top level (no component building, no effect
/// running). Out of line: its callers keep their shape.
#[cold]
#[inline(never)]
fn record_top_level(lua: &mlua::Lua, api: &'static str, per_load: bool) {
    let Some(tracked) = lua.app_data_ref::<Tracked>() else {
        return;
    };
    let Ok(env) = crate::node::env(lua) else {
        return;
    };
    if env.scope.borrow().is_none() && env.runtime.running().is_none() {
        tracked.tracker.registered(api, per_load);
    }
}

/// One module of a load.
#[derive(Clone, Debug, Default)]
pub(crate) struct Module {
    /// The modules it requires, in the order it first required them.
    pub requires: Vec<PathBuf>,
    /// What its top level started that nothing carries over yet, once each.
    pub unmanaged: Vec<&'static str>,
    /// Globals its top level set (the sandbox's one table, every chunk's).
    pub globals: Vec<String>,
    /// What its top level declared for each load (a command, a window, an
    /// `on_mount`), once each: kept, it wouldn't declare it again.
    pub per_load: Vec<&'static str>,
}

/// A load's record, as it happens.
#[derive(Default)]
pub(crate) struct Tracker {
    /// The module whose top level is running, innermost last; the app's
    /// script when none.
    stack: RefCell<Vec<PathBuf>>,
    app: RefCell<Option<PathBuf>>,
    modules: RefCell<HashMap<PathBuf, Module>>,
}

impl Tracker {
    /// The load of `app` begins: what the last load recorded goes.
    pub fn begin(&self, app: Option<PathBuf>) {
        self.stack.borrow_mut().clear();
        self.modules.borrow_mut().clear();
        if let Some(app) = &app {
            self.modules.borrow_mut().entry(app.clone()).or_default();
        }
        *self.app.borrow_mut() = app;
    }
    /// A partial load of `app` begins: the modules that `ran` names run
    /// again and are recorded anew, the others' records kept from `last`.
    pub fn begin_kept(
        &self,
        app: Option<PathBuf>,
        last: &HashMap<PathBuf, Module>,
        ran: &[PathBuf],
    ) {
        self.stack.borrow_mut().clear();
        let mut modules = self.modules.borrow_mut();
        modules.clear();
        modules.extend(
            last.iter()
                .filter(|(path, _)| !ran.contains(path))
                .map(|(path, module)| (path.clone(), module.clone())),
        );
        for path in ran {
            modules.insert(path.clone(), Module::default());
        }
        drop(modules);
        *self.app.borrow_mut() = app;
    }
    /// The module running now: the innermost required, else the app's.
    fn current(&self) -> Option<PathBuf> {
        self.stack
            .borrow()
            .last()
            .cloned()
            .or_else(|| self.app.borrow().clone())
    }
    /// `path` was required (found or run) by the module running.
    pub fn required(&self, path: &Path) {
        let Some(caller) = self.current() else {
            return;
        };
        let mut modules = self.modules.borrow_mut();
        modules.entry(path.to_path_buf()).or_default();
        let requires = &mut modules.entry(caller).or_default().requires;
        if !requires.iter().any(|p| p == path) {
            requires.push(path.to_path_buf());
        }
    }
    /// `path`'s top level begins to run.
    pub fn enter(&self, path: &Path) {
        self.stack.borrow_mut().push(path.to_path_buf());
    }
    /// It ended, having set `globals`.
    pub fn leave(&self, globals: Vec<String>) {
        let Some(path) = self.stack.borrow_mut().pop() else {
            return;
        };
        if !globals.is_empty() {
            self.modules
                .borrow_mut()
                .entry(path)
                .or_default()
                .globals
                .extend(globals);
        }
    }
    /// The module running started `api` at its top level (`top_level`), or
    /// declared it for this load (`per_load`).
    pub fn registered(&self, api: &'static str, per_load: bool) {
        let Some(module) = self.current() else {
            return;
        };
        let mut modules = self.modules.borrow_mut();
        let module = modules.entry(module).or_default();
        let list = if per_load {
            &mut module.per_load
        } else {
            &mut module.unmanaged
        };
        if !list.contains(&api) {
            list.push(api);
        }
    }
    /// What this load recorded.
    pub fn modules(&self) -> HashMap<PathBuf, Module> {
        self.modules.borrow().clone()
    }
}

/// What a reload would have done, and why.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Verdict {
    /// Kept: these modules run again (the ones a save changed and what
    /// requires them), the others kept.
    Partial { ran: Vec<PathBuf>, kept: usize },
    /// Everything loaded again, for this reason.
    Full(String),
}

/// A reload, as `zgx inspect reloads` says it.
#[derive(Clone, Debug)]
pub(crate) struct Reload {
    /// Which, counting from the host's first.
    pub number: u64,
    /// The files that changed, as this load found them.
    pub changed: Vec<PathBuf>,
    pub verdict: Verdict,
    /// Whether a partial verdict was taken; else it is what the reload
    /// would have done (no `ZGX_RELOAD=partial`, or another script loaded).
    pub taken: bool,
}

/// A host's tracking: on while it watches, and its last reloads.
#[derive(Default)]
pub(crate) struct Reloads {
    /// Whether loads are recorded (`ScriptHost::watch`).
    pub on: Cell<bool>,
    /// Whether a partial verdict is taken (`ZGX_RELOAD=partial`).
    pub keep: Cell<bool>,
    /// A partial reload failed: the next one loads everything again.
    pub failed: Cell<bool>,
    pub tracker: Rc<Tracker>,
    /// The last load's record: what a reload is judged against.
    last: RefCell<Option<HashMap<PathBuf, Module>>>,
    /// Whether plugins loaded with the last load.
    pub plugins: Cell<bool>,
    count: Cell<u64>,
    history: RefCell<VecDeque<Reload>>,
}

/// How many reloads `zgx inspect reloads` keeps.
const KEPT: usize = 32;

impl Reloads {
    /// The tracker for a load, if loads are recorded.
    pub fn tracking(&self) -> Option<Rc<Tracker>> {
        self.on.get().then(|| self.tracker.clone())
    }
    /// A load is done: its record is what the next reload is judged by.
    pub fn loaded(&self, plugins: bool) {
        if self.on.get() {
            *self.last.borrow_mut() = Some(self.tracker.modules());
            self.plugins.set(plugins);
            self.failed.set(false);
        }
    }
    /// The host asked for a reload (`ScriptHost::reload`: plugins set, a
    /// template switched): everything loads again, recorded.
    pub fn asked(&self) {
        self.record(
            Vec::new(),
            Verdict::Full("the host asked for it (ScriptHost::reload)".into()),
            false,
        );
    }
    /// The last load's record.
    pub fn last(&self) -> Option<HashMap<PathBuf, Module>> {
        self.last.borrow().clone()
    }
    /// A reload of `changed` begins: the modules to run again, if it keeps
    /// the others (`can_keep`: the app's VM alone is loaded and can), else
    /// none: it loads everything again.
    pub fn judge(
        &self,
        app: Option<&Path>,
        changed: Vec<PathBuf>,
        can_keep: bool,
    ) -> Option<Vec<PathBuf>> {
        let verdict = if self.failed.get() {
            Verdict::Full("the last reload failed partway: this one loads everything again".into())
        } else {
            judge(
                self.last.borrow().as_ref(),
                self.plugins.get(),
                app,
                &changed,
            )
        };
        let taken = can_keep && self.keep.get() && matches!(verdict, Verdict::Partial { .. });
        let ran = match (&verdict, taken) {
            (Verdict::Partial { ran, .. }, true) => Some(ran.clone()),
            _ => None,
        };
        self.record(changed, verdict, taken);
        ran
    }
    fn record(&self, changed: Vec<PathBuf>, verdict: Verdict, taken: bool) {
        let number = self.count.get() + 1;
        self.count.set(number);
        let mut history = self.history.borrow_mut();
        history.push_back(Reload {
            number,
            changed,
            verdict,
            taken,
        });
        while history.len() > KEPT {
            history.pop_front();
        }
    }
    /// The reloads kept, oldest first.
    pub fn history(&self) -> Vec<Reload> {
        self.history.borrow().iter().cloned().collect()
    }
}

/// What keeping the modules `changed` didn't touch would have done, from
/// the last load's record.
fn judge(
    last: Option<&HashMap<PathBuf, Module>>,
    plugins: bool,
    app: Option<&Path>,
    changed: &[PathBuf],
) -> Verdict {
    let Some(modules) = last else {
        return Verdict::Full(
            "no record of the last load (it ran before the host watched its files)".into(),
        );
    };
    if plugins {
        return Verdict::Full("plugins are loaded: they load again in full".into());
    }
    if let Some(file) = changed.iter().find(|c| !modules.contains_key(*c)) {
        return Verdict::Full(format!(
            "{} isn't a module the last load ran (an asset, a new file)",
            file.display()
        ));
    }
    // What runs again: what changed, and whatever requires it, to the app.
    let mut dependents: HashMap<&Path, Vec<&Path>> = HashMap::new();
    for (path, module) in modules {
        for required in &module.requires {
            dependents.entry(required).or_default().push(path);
        }
    }
    let mut ran: Vec<&Path> = Vec::new();
    let mut seen: HashSet<&Path> = HashSet::new();
    let mut queue: Vec<&Path> = changed.iter().map(PathBuf::as_path).collect();
    while let Some(path) = queue.pop() {
        if !seen.insert(path) {
            continue;
        }
        ran.push(path);
        queue.extend(dependents.get(path).into_iter().flatten().copied());
    }
    // The app's script runs again every time: what it makes is the tree.
    if let Some(app) = app
        && modules.contains_key(app)
        && !seen.contains(app)
    {
        ran.push(app);
        seen.insert(app);
    }
    // What runs again starts nothing it can't stop, and sets no global.
    for path in &ran {
        let module = &modules[*path];
        if let Some(global) = module.globals.first() {
            return Verdict::Full(format!(
                "{} sets the global `{global}` at its top level",
                path.display()
            ));
        }
        if let Some(api) = module.unmanaged.first() {
            return Verdict::Full(format!(
                "{} calls {api} at its top level, which nothing yet carries over",
                path.display()
            ));
        }
    }
    // A whole tree changing (a template swapped): nothing to keep.
    if ran.len() == modules.len() && modules.len() > 1 {
        return Verdict::Full("every module changed or requires one that did".into());
    }
    // What's kept declared nothing a load alone keeps.
    let mut kept: Vec<_> = modules
        .iter()
        .filter(|(path, _)| !seen.contains(path.as_path()))
        .collect();
    kept.sort_by(|a, b| a.0.cmp(b.0));
    if let Some((path, module)) = kept.iter().find(|(_, m)| !m.per_load.is_empty()) {
        return Verdict::Full(format!(
            "{} declares {} at its top level: kept, it wouldn't declare it again",
            path.display(),
            module.per_load[0]
        ));
    }
    let mut ran: Vec<PathBuf> = ran.into_iter().map(Path::to_path_buf).collect();
    ran.sort();
    let kept = modules.len() - ran.len();
    Verdict::Partial { ran, kept }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(
        edges: &[(&str, &[&str])],
        unmanaged: &[(&str, &'static str)],
    ) -> HashMap<PathBuf, Module> {
        let mut modules = HashMap::new();
        for (path, requires) in edges {
            modules.insert(
                PathBuf::from(path),
                Module {
                    requires: requires.iter().map(PathBuf::from).collect(),
                    ..Default::default()
                },
            );
        }
        for (path, api) in unmanaged {
            modules
                .get_mut(Path::new(path))
                .unwrap()
                .unmanaged
                .push(api);
        }
        modules
    }

    #[test]
    fn a_change_runs_again_with_what_requires_it() {
        let modules = record(
            &[
                ("app", &["screen", "button"]),
                ("screen", &["button"]),
                ("button", &[]),
                ("theme", &[]),
            ],
            &[],
        );
        let app = Path::new("app");
        assert_eq!(
            judge(Some(&modules), false, Some(app), &[PathBuf::from("button")]),
            Verdict::Partial {
                ran: vec!["app".into(), "button".into(), "screen".into()],
                kept: 1,
            }
        );
        // The app's own script: it alone.
        assert_eq!(
            judge(Some(&modules), false, Some(app), &[PathBuf::from("app")]),
            Verdict::Partial {
                ran: vec!["app".into()],
                kept: 3
            }
        );
    }

    #[test]
    fn what_nothing_carries_over_loads_everything_again() {
        let app = Path::new("app");
        let modules = record(&[("app", &["net"]), ("net", &[])], &[("net", "ui.task")]);
        let full = |v: Verdict| match v {
            Verdict::Full(why) => why,
            other => panic!("{other:?}"),
        };
        assert!(full(judge(Some(&modules), false, Some(app), &["net".into()])).contains("ui.task"));
        assert!(full(judge(None, false, Some(app), &["net".into()])).contains("no record"));
        assert!(full(judge(Some(&modules), true, Some(app), &["app".into()])).contains("plugins"));
        assert!(
            full(judge(
                Some(&modules),
                false,
                Some(app),
                &["logo.png".into()]
            ))
            .contains("asset")
        );
        // A kept module may make anything: it doesn't run again.
        assert!(matches!(
            judge(Some(&modules), false, Some(app), &["app".into()]),
            Verdict::Partial { .. }
        ));
        let mut globals = record(&[("app", &["g"]), ("g", &[])], &[]);
        globals
            .get_mut(Path::new("g"))
            .unwrap()
            .globals
            .push("Config".into());
        assert!(full(judge(Some(&globals), false, Some(app), &["g".into()])).contains("Config"));
    }

    #[test]
    fn a_load_records_requires_registrations_and_globals_by_module() {
        let tracker = Tracker::default();
        tracker.begin(Some("app".into()));
        tracker.required(Path::new("a"));
        tracker.enter(Path::new("a"));
        tracker.registered("anime.animate", false);
        tracker.required(Path::new("b"));
        tracker.enter(Path::new("b"));
        tracker.registered("ui.task", false);
        tracker.registered("ui.task", false);
        tracker.leave(vec!["G".into()]);
        tracker.leave(Vec::new());
        tracker.registered("ui.router", false);
        let modules = tracker.modules();
        assert_eq!(modules[Path::new("app")].requires, [PathBuf::from("a")]);
        assert_eq!(modules[Path::new("a")].requires, [PathBuf::from("b")]);
        assert_eq!(modules[Path::new("a")].unmanaged, ["anime.animate"]);
        assert_eq!(modules[Path::new("b")].unmanaged, ["ui.task"], "once each");
        assert_eq!(modules[Path::new("b")].globals, ["G"]);
        assert_eq!(modules[Path::new("app")].unmanaged, ["ui.router"]);
        // A new load forgets the last.
        tracker.begin(Some("app".into()));
        assert_eq!(tracker.modules().len(), 1);
    }
}
