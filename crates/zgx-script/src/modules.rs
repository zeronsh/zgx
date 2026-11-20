//! Native modules (`ScriptHost::load_module`): an app's own Rust, built
//! apart from zgx as a library and loaded here, its functions registered as
//! a host's own are (`ScriptHost::function`, `ScriptHost::action`).
//!
//! What crosses is `zgx_module::abi`: C functions, and a [`Value`]'s own
//! layout. A module is lent a value where it is and reads its text, its
//! items and its fields there, with no call, as code linked into the app
//! does; it calls here to make a value and to free the last hold on one
//! (docs/design/runtime.md has the measurements).
//!
//! Only an app loads modules, by a path it names in Rust: nothing a script
//! or a plugin can say loads one.

use std::{ffi::c_void, mem::MaybeUninit, path::Path, rc::Rc};

use zgx_module::abi::{self, Api, Bytes, Raw, Registrar};

use crate::{Field, List, Map, ScriptHost, Str, Value};

// What a module reads values by (`zgx_module::abi`) is what they are here,
// stated in their own definitions (`Value`: `repr(u8)`; `Str`, `Slice` and
// `Field`: `repr(C)`, in counted.rs, whose test reads their words) and
// held to here, where the two meet.
const _: () = {
    assert!(size_of::<Value>() == abi::VALUE_SIZE && align_of::<Value>() == 8);
    assert!(size_of::<Raw>() == abi::VALUE_SIZE && align_of::<Raw>() == 8);
    assert!(size_of::<Field>() == abi::FIELD_SIZE);
    assert!(std::mem::offset_of!(Field, 0) == abi::FIELD_KEY_AT);
    assert!(std::mem::offset_of!(Field, 1) == abi::FIELD_VALUE_AT);
    assert!(size_of::<Str>() == 16 && size_of::<List>() == 16 && size_of::<Map>() == 16);
};

/// `value` written to `out`, a module's from then.
///
/// # Safety
/// `out` has room for a value, and holds none.
#[inline(always)]
unsafe fn give(value: Value, out: *mut Raw) {
    // SAFETY: A `Raw` is a value's size and alignment (asserted above).
    unsafe { out.cast::<Value>().write(value) }
}
/// The value a module lent, read where it is.
///
/// # Safety
/// `raw` points at a value alive for `'a`.
#[inline(always)]
unsafe fn lent<'a>(raw: *const Raw) -> &'a Value {
    // SAFETY: The caller's.
    unsafe { &*raw.cast::<Value>() }
}
/// An owned value a module hands over, taken.
///
/// # Safety
/// `raw` points at an owned value, not used again.
#[inline(always)]
unsafe fn take(raw: *mut Raw) -> Value {
    // SAFETY: The caller's.
    unsafe { raw.cast::<Value>().read() }
}

unsafe extern "C" fn string(ptr: *const u8, len: usize, out: *mut Raw) {
    // SAFETY: The module's string, UTF-8 (a `&str` of its own).
    unsafe {
        let text = std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len));
        give(Value::Str(Str::from(text)), out);
    }
}
unsafe extern "C" fn list(items: *mut Raw, len: usize, out: *mut Raw) {
    // SAFETY: `len` owned values, taken once each.
    unsafe {
        let items: List = (0..len).map(|i| take(items.add(i))).collect();
        give(Value::List(items), out);
    }
}
unsafe extern "C" fn map(keys: *const Bytes, values: *mut Raw, len: usize, out: *mut Raw) {
    let field = |i: usize| {
        // SAFETY: `len` keys (UTF-8) and as many owned values, taken once.
        unsafe {
            let key = *keys.add(i);
            let key = std::str::from_utf8_unchecked(std::slice::from_raw_parts(key.ptr, key.len));
            (key, take(values.add(i)))
        }
    };
    // SAFETY: Room for a value.
    unsafe { give(Value::record((0..len).map(field)), out) }
}
unsafe extern "C" fn slice(list: *const Raw, from: usize, to: usize, out: *mut Raw) {
    // SAFETY: A value alive for this call, and room for one.
    unsafe {
        let part = match lent(list) {
            Value::List(items) => items.get(from..to),
            _ => None,
        };
        give(
            part.map_or(Value::Nil, |items| {
                Value::List(items.iter().cloned().collect())
            }),
            out,
        );
    }
}
unsafe extern "C" fn clone(value: *const Raw, out: *mut Raw) {
    // SAFETY: A value alive for this call, and room for one.
    unsafe { give(lent(value).clone(), out) }
}
unsafe extern "C" fn release(value: *mut Raw) {
    // SAFETY: An owned value, given back once.
    drop(unsafe { take(value) });
}
unsafe extern "C" fn open(value: *const Raw) -> *const Raw {
    // SAFETY: A value alive for this call.
    match unsafe { lent(value) }.read_through() {
        Some(opened) => std::ptr::from_ref(opened).cast(),
        None => std::ptr::null(),
    }
}

unsafe extern "C" fn flat(value: *const Raw, out: *mut Raw) {
    // SAFETY: A value alive for this call, and room for one.
    unsafe {
        let whole = match lent(value) {
            Value::Text(text) => Value::Str(Str::from(text.to_string())),
            _ => Value::Nil,
        };
        give(whole, out);
    }
}

/// The value functions every module is given.
static API: Api = Api {
    abi: abi::ABI,
    size: size_of::<Api>() as u32,
    string,
    list,
    map,
    slice,
    clone,
    release,
    open,
    flat,
};

/// A registration's `user`, freed with the host's hold on it.
struct User {
    ptr: *mut c_void,
    free: abi::Free,
}
impl Drop for User {
    fn drop(&mut self) {
        // SAFETY: The module's own pointer and its own function for it,
        // called once.
        unsafe { (self.free)(self.ptr) }
    }
}

unsafe fn name_of(name: *const u8, len: usize) -> String {
    // SAFETY: The module's `&str`.
    unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(name, len)) }.to_owned()
}
unsafe extern "C" fn register_function(
    host: *mut c_void,
    name: *const u8,
    len: usize,
    call: abi::Function,
    user: *mut c_void,
    free: abi::Free,
) {
    // SAFETY: `host` is the `ScriptHost` `enter` is loading into.
    let (host, name) = unsafe { (&*(host as *const ScriptHost), name_of(name, len)) };
    let user = User { ptr: user, free };
    host.function(name, move |arg| {
        // (All of `user`, not its pointer alone: it frees with the closure.)
        let user = &user;
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: The module's function and its own `user`; the argument is
        // lent for the call, where it is, and what it writes is owned.
        unsafe {
            call(user.ptr, std::ptr::from_ref(&arg).cast(), out.as_mut_ptr());
            take(out.as_mut_ptr())
        }
    });
}
unsafe extern "C" fn register_function_taking(
    host: *mut c_void,
    name: *const u8,
    len: usize,
    call: abi::FunctionTaking,
    user: *mut c_void,
    free: abi::Free,
) {
    // SAFETY: As `register_function`'s.
    let (host, name) = unsafe { (&*(host as *const ScriptHost), name_of(name, len)) };
    let user = User { ptr: user, free };
    host.function(name, move |arg| {
        let user = &user;
        // (The module's from here: it frees it, or hands it back.)
        let mut arg = std::mem::ManuallyDrop::new(arg);
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: The module's function and its own `user`; the argument is
        // given, and what it writes is owned.
        unsafe {
            call(
                user.ptr,
                std::ptr::from_mut::<Value>(&mut arg).cast(),
                out.as_mut_ptr(),
            );
            take(out.as_mut_ptr())
        }
    });
}
unsafe extern "C" fn register_action(
    host: *mut c_void,
    name: *const u8,
    len: usize,
    call: abi::Action,
    user: *mut c_void,
    free: abi::Free,
) {
    // SAFETY: As `register_function`'s.
    let (host, name) = unsafe { (&*(host as *const ScriptHost), name_of(name, len)) };
    let user = User { ptr: user, free };
    host.action(name, move |arg| {
        let user = &user;
        // SAFETY: The module's action and its own `user`, the argument lent.
        unsafe { call(user.ptr, std::ptr::from_ref(&arg).cast()) }
    });
}

/// A module's state on this thread: its memory, and what frees it.
struct ModuleState {
    at: *mut u8,
    layout: std::alloc::Layout,
    free: unsafe extern "C" fn(*mut u8),
}
impl Drop for ModuleState {
    fn drop(&mut self) {
        // SAFETY: What `state` made: the module's own value, dropped by its
        // own function, then its memory.
        unsafe {
            (self.free)(self.at);
            if self.layout.size() != 0 {
                std::alloc::dealloc(self.at, self.layout);
            }
        }
    }
}
thread_local! {
    /// Modules' states on this thread, by the names they gave.
    static STATES: std::cell::RefCell<std::collections::HashMap<String, ModuleState>> = Default::default();
}
unsafe extern "C" fn state(
    _host: *mut c_void,
    name: *const u8,
    len: usize,
    size: usize,
    align: usize,
    init: unsafe extern "C" fn(*mut u8),
    free: unsafe extern "C" fn(*mut u8),
) -> *mut u8 {
    // SAFETY: The module's `&str`.
    let name = unsafe { name_of(name, len) };
    STATES.with_borrow_mut(|states| {
        if let Some(state) = states.get(&name) {
            return state.at;
        }
        let layout =
            std::alloc::Layout::from_size_align(size, align).expect("a module's state's layout");
        // SAFETY: A layout of the module's own type; `init` writes it.
        let at = unsafe {
            let at = if size == 0 {
                std::ptr::without_provenance_mut(align)
            } else {
                std::alloc::alloc(layout)
            };
            assert!(!at.is_null(), "no memory for a module's state");
            init(at);
            at
        };
        states.insert(name, ModuleState { at, layout, free });
        at
    })
}

impl ScriptHost {
    /// Loads a native module (a library built with the `zgx-module` crate)
    /// and lets it register its functions and actions with this host, as
    /// `function` and `action` would: scripts call them the same way, and
    /// plugins reach them only if exposed (`expose_action`).
    ///
    /// The library stays loaded for as long as the process runs: what it
    /// registered may be called, and values it made freed, at any time.
    ///
    /// # Safety
    /// Loading a library runs its code: `path` must be a module the app
    /// itself built or shipped, never one a script, a plugin or a person
    /// named.
    pub unsafe fn load_module(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        let fail = |why: &dyn std::fmt::Display| format!("{}: {why}", path.display());
        // SAFETY: The caller's promise.
        let library = unsafe { libloading::Library::new(path) }.map_err(|e| fail(&e))?;
        // SAFETY: A module's entry is an `abi::Entry` by that name.
        let entry = *unsafe { library.get::<abi::Entry>(abi::ENTRY) }
            .map_err(|e| fail(&format_args!("not a zgx module ({e})")))?;
        // (Never unloaded: closures and values of its code outlive this.)
        std::mem::forget(library);
        // SAFETY: The module's entry, by the interface it was built to.
        unsafe { self.enter_module(entry) }.map_err(|e| fail(&e))
    }

    /// Registers a module linked into the program itself, by its entry
    /// (`zgx_module::module!`'s `zgx_module_v1`): the same registration
    /// `load_module` does, with no library to load.
    ///
    /// # Safety
    /// `entry` is a `zgx_module::module!`'s.
    pub unsafe fn enter_module(&self, entry: abi::Entry) -> Result<(), String> {
        let registrar = Registrar {
            size: size_of::<Registrar>() as u32,
            host: std::ptr::from_ref(self) as *mut c_void,
            function: register_function,
            function_taking: register_function_taking,
            action: register_action,
            state,
        };
        // SAFETY: The tables an entry is called with, `registrar` alive
        // through the call.
        match unsafe { entry(&API, &registrar) } {
            0 => Ok(()),
            abi::REFUSED_ABI => Err(format!(
                "built for another module interface than this runtime's ({})",
                abi::ABI
            )),
            code => Err(format!("the module refused to load ({code})")),
        }
    }

    /// The function registered as `name` (`function`, or a module's), for
    /// the host's own Rust to call: what a module answers scripts with,
    /// asked from Rust.
    pub fn registered_function(&self, name: &str) -> Option<Rc<dyn Fn(Value) -> Value>> {
        self.0.functions.borrow().get(name).cloned()
    }
    /// The action registered as `name` (`action`, or a module's), for the
    /// host's own Rust to call.
    pub fn registered_action(&self, name: &str) -> Option<Rc<dyn Fn(Value)>> {
        self.0.actions.borrow().get(name).cloned()
    }
}
