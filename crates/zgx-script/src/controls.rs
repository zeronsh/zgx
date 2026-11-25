//! `ui.controls`, `ui.press` and `ui.text_of`: what a script's own tests
//! and guides read of the window and do in it, as a person would (the
//! accessibility tree's controls, a press at one's middle, the text one
//! shows), through `driver`.
use crate::{host_views::HostView, value::Store};
use mlua::{Lua, Table};
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};
use zgx::{
    reactive::Runtime,
    scene::{NodeId, Scene},
    widgets::{Ui, WeakUi},
};

/// The windows hosts gave (`ScriptHost::window_controls`), each with its
/// host's shared state, which names the host. With the runtime, made by the
/// first window given: a host that gives none holds nothing for it.
#[derive(Default)]
pub(crate) struct Windows(RefCell<Vec<(Weak<Store>, WeakUi)>>);
impl Windows {
    pub(crate) fn of(runtime: &Runtime) -> Rc<Self> {
        runtime.extension(|_| Self::default())
    }
    pub(crate) fn add(&self, host: &Rc<Store>, ui: &Ui) {
        let mut windows = self.0.borrow_mut();
        windows.retain(|(host, window)| host.strong_count() > 0 && window.upgrade().is_some());
        windows.push((Rc::downgrade(host), ui.downgrade()));
    }
    /// `host`'s: the active one, else the last given.
    fn window(&self, host: &Rc<Store>) -> Option<Ui> {
        let mut windows: Vec<Ui> = self
            .0
            .borrow()
            .iter()
            .filter(|(given, _)| std::ptr::eq(given.as_ptr(), Rc::as_ptr(host)))
            .filter_map(|(_, window)| window.upgrade())
            .collect();
        match windows.iter().position(Ui::window_is_active) {
            Some(active) => Some(windows.swap_remove(active)),
            None => windows.pop(),
        }
    }
}

/// The window `ui.controls`, `ui.press` and `ui.text_of` act on: the active one, else the
/// last given (`ScriptHost::window_controls`); a plugin has none.
fn control_window(lua: &Lua, what: &str) -> mlua::Result<Option<Ui>> {
    let env = crate::node::env(lua)?;
    if env.meter.is_some() {
        return Err(mlua::Error::runtime(format!(
            "{what} is not available to plugins"
        )));
    }
    Ok(Windows::of(&env.runtime).window(&env.shared))
}

/// What a script under a guard may list, press and read: its own view (its
/// popups, menus and dialogs with it), without what a `ui.host_view` in it
/// holds, which is the host's.
struct Own {
    root: Option<NodeId>,
    portals: Vec<(NodeId, NodeId)>,
}
impl Own {
    /// What the script may use of `ui` (`None`: all of it, no guard is set).
    fn of(lua: &Lua, ui: &Ui) -> Option<Self> {
        let root = crate::rules::guarded_root(lua)?;
        Some(Self {
            root: root.filter(|root| root.is_in(ui)).map(|root| root.node()),
            portals: ui.portals(),
        })
    }
    fn holds(&self, ui: &Ui, scene: &Scene, mut node: NodeId) -> bool {
        // As far as a tree is deep; what isn't a tree isn't the script's.
        for _ in 0..100_000 {
            // The host's view is the host's, as is what it shows.
            if ui.retains::<HostView>(node) {
                return false;
            }
            if Some(node) == self.root {
                return true;
            }
            let owner = self.portals.iter().find(|(_, overlay)| *overlay == node);
            node = match owner {
                Some((owner, _)) => *owner,
                None => match scene.parent(node) {
                    Some(parent) => parent,
                    None => return false,
                },
            };
        }
        false
    }
}
/// Refuses `what` of a control that is not the script's own, under a guard.
fn not_own(lua: &Lua, ui: &Ui, what: &str, handle: f64) -> mlua::Result<()> {
    let Some(own) = Own::of(lua, ui) else {
        return Ok(());
    };
    let node = NodeId::from_bits(handle as u64);
    let scene = ui.scene.borrow();
    if scene.contains(node) && !own.holds(ui, &scene, node) {
        return Err(mlua::Error::runtime(format!(
            "{what}: that control isn't this app's own, and the host's guard keeps a script to its own view"
        )));
    }
    Ok(())
}

/// `ui.controls { limit? }`: the controls on screen as a list, in document
/// order with popups and modals last: `{ role, name, x, y, w, h, disabled,
/// selected, checked, expanded, pressed, haspopup, focused, handle }`, window
/// points, a menu's or list box's scroll view adding `scroll_offset`,
/// `scroll_range`, `at_start` and `at_end`. `{}` where the host gave no
/// window (`ScriptHost::window_controls`).
fn controls(lua: &Lua, options: Option<Table>) -> mlua::Result<Table> {
    let limit = match &options {
        Some(options) => options
            .get::<Option<f64>>("limit")?
            .map(|n| n.max(0.) as usize),
        None => None,
    };
    let out = lua.create_table()?;
    let Some(ui) = control_window(lua, "ui.controls")? else {
        return Ok(out);
    };
    let own = Own::of(lua, &ui);
    let listed = crate::driver::controls_where(
        &ui,
        limit,
        |scene, node| own.as_ref().is_none_or(|own| own.holds(&ui, scene, node)),
        |node| own.is_some() && ui.retains::<HostView>(node),
    );
    for (i, control) in listed.into_iter().enumerate() {
        let item = lua.create_table()?;
        item.set("role", control.role)?;
        item.set("name", control.name)?;
        item.set("x", control.bounds.x)?;
        item.set("y", control.bounds.y)?;
        item.set("w", control.bounds.width)?;
        item.set("h", control.bounds.height)?;
        item.set("disabled", control.disabled)?;
        item.set("selected", control.selected)?;
        item.set("checked", control.checked)?;
        item.set("expanded", control.expanded)?;
        item.set("pressed", control.pressed)?;
        item.set("haspopup", control.has_popup)?;
        if let Some((offset, range)) = control.scroll {
            item.set("scroll_offset", offset)?;
            item.set("scroll_range", range)?;
            item.set("at_start", offset <= 0.5)?;
            item.set("at_end", offset >= range - 0.5)?;
        }
        item.set("focused", control.focused)?;
        // A number a script can hold exactly (2^53): a slot reused two million times wraps.
        item.set("handle", (control.handle & ((1 << 53) - 1)) as f64)?;
        out.raw_set(i + 1, item)?;
    }
    Ok(out)
}

/// `ui.press(handle)`: the press and release `zgx inspect click` makes at
/// the middle of the control `handle` (from `ui.controls`) names, through
/// the window's input; false, doing nothing, for a handle that is stale,
/// hidden or covered, and while a handler is handling input (press from a
/// command, a timer or a task).
fn press(lua: &Lua, handle: f64) -> mlua::Result<bool> {
    let Some(mut ui) = control_window(lua, "ui.press")? else {
        return Ok(false);
    };
    // Input within input would go to what the outer press captured.
    if ui.is_dispatching() {
        return Ok(false);
    }
    if !(handle.is_finite() && handle >= 0. && handle.fract() == 0.) {
        return Ok(false);
    }
    not_own(lua, &ui, "ui.press", handle)?;
    // The same window, to ask of while it is pressed.
    let Some(at) = ui.downgrade().upgrade() else {
        return Ok(false);
    };
    let own = Own::of(lua, &ui);
    // A script's press, not a person's: it takes no pointer lock.
    Ok(zgx::input::untrusted(|| {
        crate::driver::press_control_where(&mut ui, handle as u64, |scene, hit| {
            own.as_ref().is_none_or(|own| own.holds(&at, scene, hit))
        })
    }))
}

/// `ui.text_of(handle, cap?)`: the text a control (`ui.controls`' `handle`)
/// shows, a word apart, at most `cap` (400) characters; "" when it is gone.
fn text_of(lua: &Lua, (handle, cap): (f64, Option<f64>)) -> mlua::Result<String> {
    let Some(ui) = control_window(lua, "ui.text_of")? else {
        return Ok(String::new());
    };
    if !(handle.is_finite() && handle >= 0. && handle.fract() == 0.) {
        return Ok(String::new());
    }
    not_own(lua, &ui, "ui.text_of", handle)?;
    let cap = cap
        .filter(|c| c.is_finite() && *c >= 0.)
        .map_or(400, |c| c as usize);
    let guarded = Own::of(lua, &ui).is_some();
    Ok(
        crate::driver::subtree_text_where(&ui, handle as u64, cap, |node| {
            guarded && ui.retains::<HostView>(node)
        })
        .unwrap_or_default(),
    )
}

/// `ui.controls`, `ui.press` and `ui.text_of`, made the first time a script
/// names one (`lazy`).
pub(crate) fn install(lua: &Lua, ui: &Table) -> mlua::Result<()> {
    ui.set("controls", lua.create_function(controls)?)?;
    ui.set("press", lua.create_function(press)?)?;
    ui.set("text_of", lua.create_function(text_of)?)?;
    Ok(())
}
