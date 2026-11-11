//! What a plugin's state holds in Rust, counted with what its VM holds in
//! Lua against its memory limit (`Limits::plugin_memory`). A state's value
//! lives outside the VM, where Lua's limit does not see it: without this, a
//! plugin making state after state held as much as it liked.
//!
//! Each write weighs what it changes (a whole value, or the leaf a
//! `set_key` writes), so a write costs what it converts. What a plugin's
//! states hold is taken off what its VM may allocate; a write that would
//! take the two past the limit is an error, and writes nothing.
use crate::value::Value;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};
use zgx::reactive::{Signal, WeakSignal};

/// Something held in Rust: whether it's still alive, and what it holds.
#[cfg(any(feature = "scene3d", feature = "audio"))]
type Native = (Box<dyn Fn() -> bool>, usize);

/// A plugin VM's meter.
pub(crate) struct Meter {
    lua: mlua::WeakLua,
    /// What the VM's interrupt collects by.
    watch: Rc<crate::interrupt::Watch>,
    limit: usize,
    /// What its states hold, in all.
    total: Cell<usize>,
    /// Each state it counts, by address, with what it holds.
    held: RefCell<HashMap<usize, (WeakSignal<Value>, usize)>>,
    /// What else it holds in Rust (a mesh's own triangles).
    #[cfg(any(feature = "scene3d", feature = "audio"))]
    native: RefCell<Vec<Native>>,
    /// States counted at the last sweep: the next is at twice as many.
    swept: Cell<usize>,
}
impl Meter {
    pub fn new(lua: &mlua::Lua, limit: usize, watch: Rc<crate::interrupt::Watch>) -> Rc<Self> {
        Rc::new(Self {
            lua: lua.weak(),
            watch,
            limit,
            total: Cell::new(0),
            held: RefCell::default(),
            #[cfg(any(feature = "scene3d", feature = "audio"))]
            native: RefCell::default(),
            swept: Cell::new(64),
        })
    }
    /// What `signal` is counted as holding.
    pub fn held_by(&self, signal: &Signal<Value>) -> usize {
        self.held
            .borrow()
            .get(&signal.addr())
            .map_or(0, |(_, bytes)| *bytes)
    }
    /// Whether the plugin has room for its states to hold `more` bytes
    /// beyond what they do, freeing what dropped states were counted for
    /// first if it has not.
    pub fn room(&self, more: usize) -> mlua::Result<()> {
        if more == 0 || self.fits(more) {
            return Ok(());
        }
        // What the VM holds counts what it has not collected yet.
        if let Some(lua) = self.lua.try_upgrade() {
            let _ = lua.gc_collect();
        }
        self.sweep();
        if self.fits(more) {
            return Ok(());
        }
        Err(mlua::Error::runtime(format!(
            "a plugin's memory is {} MB at most; its state would hold {} MB more than it has room for",
            self.limit >> 20,
            (more + self.used())
                .saturating_sub(self.limit)
                .div_ceil(1 << 20),
        )))
    }
    /// `signal` to hold `bytes`: an error (and nothing counted) if there is
    /// no room for them.
    pub fn hold(&self, signal: &Signal<Value>, bytes: usize) -> mlua::Result<()> {
        let was = self.held_by(signal);
        self.room(bytes.saturating_sub(was))?;
        self.set(signal, bytes);
        Ok(())
    }
    /// `signal` counted as holding `bytes`.
    pub fn set(&self, signal: &Signal<Value>, bytes: usize) {
        let mut held = self.held.borrow_mut();
        let was = match held.get_mut(&signal.addr()) {
            Some((_, entry)) => std::mem::replace(entry, bytes),
            None => {
                held.insert(signal.addr(), (signal.downgrade(), bytes));
                0
            }
        };
        let grew = held.len() > self.swept.get();
        drop(held);
        self.total.set(self.total.get() - was + bytes);
        if grew {
            self.sweep();
        } else {
            self.lower_limit();
        }
    }
    /// `bytes` held in Rust for as long as `alive` says, counted as a state's
    /// are. Asked first with `room`.
    #[cfg(any(feature = "scene3d", feature = "audio"))]
    pub fn hold_native(&self, alive: impl Fn() -> bool + 'static, bytes: usize) {
        let mut native = self.native.borrow_mut();
        native.push((Box::new(alive), bytes));
        let grew = native.len() > self.swept.get();
        drop(native);
        self.total.set(self.total.get() + bytes);
        if grew {
            self.sweep();
        } else {
            self.lower_limit();
        }
    }
    /// Forget the states that are gone (and what they held).
    fn sweep(&self) {
        #[cfg(any(feature = "scene3d", feature = "audio"))]
        let freed_native = {
            let mut native = self.native.borrow_mut();
            let mut freed = 0;
            native.retain(|(alive, bytes)| {
                let alive = alive();
                if !alive {
                    freed += *bytes;
                }
                alive
            });
            freed
        };
        #[cfg(not(any(feature = "scene3d", feature = "audio")))]
        let freed_native = 0;
        let mut held = self.held.borrow_mut();
        let mut freed = 0;
        held.retain(|_, (signal, bytes)| {
            let alive = signal.is_alive();
            if !alive {
                freed += *bytes;
            }
            alive
        });
        #[cfg(any(feature = "scene3d", feature = "audio"))]
        let most = held.len().max(self.native.borrow().len());
        #[cfg(not(any(feature = "scene3d", feature = "audio")))]
        let most = held.len();
        self.swept.set((most * 2).max(64));
        drop(held);
        self.total.set(self.total.get() - freed - freed_native);
        self.lower_limit();
    }
    /// Its limit, in bytes.
    #[cfg(feature = "scene3d")]
    pub fn limit(&self) -> usize {
        self.limit
    }
    /// What the VM and the states hold together.
    pub fn used(&self) -> usize {
        let lua = self.lua.try_upgrade().map_or(0, |lua| lua.used_memory());
        lua + self.total.get()
    }
    fn fits(&self, more: usize) -> bool {
        self.used() + more <= self.limit
    }
    /// The VM may allocate what its states leave of the limit (at least a
    /// byte: 0 is no limit to Luau).
    fn lower_limit(&self) {
        let left = self.limit.saturating_sub(self.total.get()).max(1);
        if let Some(lua) = self.lua.try_upgrade() {
            let _ = lua.set_memory_limit(left);
        }
        self.watch.set_limit(left);
    }
}
