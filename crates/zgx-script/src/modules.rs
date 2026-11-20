//! Native modules (`ScriptHost::load_module`): an app's own Rust, built
//! apart from zgx as a library and loaded here, its functions registered as
//! a host's own are (`ScriptHost::function`, `ScriptHost::action`).
//!
//! What crosses is `zgx_module::abi`: C types and functions. A [`Value`]
//! crosses as three words, its strings, lists and maps the very `Rc`s
//! state holds, counted not copied, so a call through a module costs the
//! call a linked-in function costs plus a jump (docs/design/runtime.md has
//! the measurements).
//!
//! Only an app loads modules, by a path it names in Rust: nothing a script
//! or a plugin can say loads one.

use std::{ffi::c_void, path::Path, rc::Rc};

use zgx_module::abi::{self, Api, Bytes, Raw, Registrar};

use crate::{ScriptHost, Value};

const NIL: Raw = Raw {
    tag: abi::NIL,
    a: 0,
    b: 0,
};

/// `value` as a module owns it: what it counted, the module now does.
#[inline]
fn give(value: Value) -> Raw {
    match value {
        Value::Nil => NIL,
        Value::Bool(b) => Raw {
            tag: abi::BOOL,
            a: u64::from(b),
            b: 0,
        },
        Value::Num(n) => Raw {
            tag: abi::NUM,
            a: n.to_bits(),
            b: 0,
        },
        Value::Str(s) => {
            let len = s.len();
            Raw {
                tag: abi::STR,
                a: Rc::into_raw(s) as *const u8 as u64,
                b: len as u64,
            }
        }
        Value::List(items) => {
            let len = items.len();
            Raw {
                tag: abi::LIST,
                a: Rc::into_raw(items) as *const Value as u64,
                b: len as u64,
            }
        }
        Value::Map(fields) => {
            let len = fields.len();
            Raw {
                tag: abi::MAP,
                a: Rc::into_raw(fields) as *const (Rc<str>, Value) as u64,
                b: len as u64,
            }
        }
        other => Raw {
            tag: abi::OTHER,
            a: Box::into_raw(Box::new(other)) as u64,
            b: 0,
        },
    }
}

/// `value` as a module reads it for as long as `value` lives: nothing
/// counted.
#[inline]
fn lend(value: &Value) -> Raw {
    match value {
        Value::Nil => NIL,
        Value::Bool(b) => Raw {
            tag: abi::BOOL,
            a: u64::from(*b),
            b: 0,
        },
        Value::Num(n) => Raw {
            tag: abi::NUM,
            a: n.to_bits(),
            b: 0,
        },
        Value::Str(s) => Raw {
            tag: abi::STR,
            a: Rc::as_ptr(s) as *const u8 as u64,
            b: s.len() as u64,
        },
        Value::List(items) => Raw {
            tag: abi::LIST,
            a: Rc::as_ptr(items) as *const Value as u64,
            b: items.len() as u64,
        },
        Value::Map(fields) => Raw {
            tag: abi::MAP,
            a: Rc::as_ptr(fields) as *const (Rc<str>, Value) as u64,
            b: fields.len() as u64,
        },
        other => Raw {
            tag: abi::OTHER,
            a: std::ptr::from_ref(other) as u64,
            b: 0,
        },
    }
}

fn str_ptr(raw: &Raw) -> *const str {
    std::ptr::slice_from_raw_parts(raw.a as *const u8, raw.b as usize) as *const str
}
fn list_ptr(raw: &Raw) -> *const [Value] {
    std::ptr::slice_from_raw_parts(raw.a as *const Value, raw.b as usize)
}
fn map_ptr(raw: &Raw) -> *const [(Rc<str>, Value)] {
    std::ptr::slice_from_raw_parts(raw.a as *const (Rc<str>, Value), raw.b as usize)
}

/// An owned value a module hands back, as the [`Value`] it was.
///
/// # Safety
/// `raw` is what [`give`] or one of [`API`]'s makers returned, or a clone
/// of a lent value ([`clone`]), owned by nothing else.
#[inline]
unsafe fn take(raw: Raw) -> Value {
    // SAFETY: Each pointer is the `Rc::into_raw` (or `Box::into_raw`) of
    // its kind, with the length it had.
    unsafe {
        match raw.tag {
            abi::BOOL => Value::Bool(raw.a != 0),
            abi::NUM => Value::Num(f64::from_bits(raw.a)),
            abi::STR => Value::Str(Rc::from_raw(str_ptr(&raw))),
            abi::LIST => Value::List(Rc::from_raw(list_ptr(&raw))),
            abi::MAP => Value::Map(Rc::from_raw(map_ptr(&raw))),
            abi::OTHER => *Box::from_raw(raw.a as *mut Value),
            _ => Value::Nil,
        }
    }
}

unsafe extern "C" fn string(ptr: *const u8, len: usize) -> Raw {
    // SAFETY: The module's string, UTF-8 (a `&str` of its own).
    let text = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) };
    give(Value::Str(Rc::from(text)))
}
unsafe extern "C" fn list(items: *mut Raw, len: usize) -> Raw {
    // SAFETY: `len` owned values, taken once each.
    let items: Rc<[Value]> = (0..len).map(|i| unsafe { take(*items.add(i)) }).collect();
    give(Value::List(items))
}
unsafe extern "C" fn map(keys: *const Bytes, values: *mut Raw, len: usize) -> Raw {
    let field = |i: usize| {
        // SAFETY: `len` keys (UTF-8) and as many owned values, taken once.
        unsafe {
            let key = *keys.add(i);
            let key = std::str::from_utf8_unchecked(std::slice::from_raw_parts(key.ptr, key.len));
            (key, take(*values.add(i)))
        }
    };
    give(Value::record((0..len).map(field)))
}
unsafe extern "C" fn clone(value: *const Raw) -> Raw {
    // SAFETY: A value lent or owned, alive for this call: its `Rc` is
    // counted once more, or (another kind) the value cloned into a box.
    unsafe {
        let raw = *value;
        match raw.tag {
            abi::STR => Rc::increment_strong_count(str_ptr(&raw)),
            abi::LIST => Rc::increment_strong_count(list_ptr(&raw)),
            abi::MAP => Rc::increment_strong_count(map_ptr(&raw)),
            abi::OTHER => return give((*(raw.a as *const Value)).clone()),
            _ => {}
        }
        raw
    }
}
unsafe extern "C" fn release(value: Raw) {
    // SAFETY: An owned value, given back once.
    drop(unsafe { take(value) });
}
unsafe extern "C" fn item(list: *const Raw, index: usize, out: *mut Raw) -> bool {
    // SAFETY: A list alive for this call, and room for one value.
    unsafe {
        let raw = *list;
        if raw.tag != abi::LIST {
            return false;
        }
        match (&*list_ptr(&raw)).get(index) {
            Some(value) => {
                *out = lend(value);
                true
            }
            None => false,
        }
    }
}
unsafe extern "C" fn field(map: *const Raw, key: *const u8, len: usize, out: *mut Raw) -> bool {
    // SAFETY: A map alive for this call, a key's UTF-8, room for a value.
    unsafe {
        let raw = *map;
        if raw.tag != abi::MAP {
            return false;
        }
        let key = std::str::from_utf8_unchecked(std::slice::from_raw_parts(key, len));
        let fields = &*map_ptr(&raw);
        match crate::value::find_key(fields, key) {
            Ok(at) => {
                *out = lend(&fields[at].1);
                true
            }
            Err(_) => false,
        }
    }
}
unsafe extern "C" fn entry(
    map: *const Raw,
    index: usize,
    key: *mut Bytes,
    value: *mut Raw,
) -> bool {
    // SAFETY: A map alive for this call, room for a key and a value.
    unsafe {
        let raw = *map;
        if raw.tag != abi::MAP {
            return false;
        }
        match (&*map_ptr(&raw)).get(index) {
            Some((k, v)) => {
                *key = Bytes {
                    ptr: k.as_ptr(),
                    len: k.len(),
                };
                *value = lend(v);
                true
            }
            None => false,
        }
    }
}
unsafe extern "C" fn open(value: *const Raw, out: *mut Raw) -> bool {
    // SAFETY: A value alive for this call, room for one.
    unsafe {
        let raw = *value;
        if raw.tag != abi::OTHER {
            return false;
        }
        let held = &*(raw.a as *const Value);
        match held.read_through() {
            Some(read) => {
                *out = lend(read);
                true
            }
            None => false,
        }
    }
}

/// The value functions every module is given.
static API: Api = Api {
    abi: abi::ABI,
    size: size_of::<Api>() as u32,
    string,
    list,
    map,
    clone,
    release,
    item,
    field,
    entry,
    open,
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
        // SAFETY: The module's function and its own `user`; the argument is
        // given, and what it returns is owned.
        unsafe { take(call(user.ptr, give(arg))) }
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
        // SAFETY: The module's action and its own `user`.
        unsafe { call(user.ptr, give(arg)) }
    });
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
            action: register_action,
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
