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

use std::{ffi::c_void, mem::MaybeUninit, path::Path, rc::Rc, sync::OnceLock};

use zgx_module::abi::{self, Api, Bytes, Layout, Raw, Registrar};

use crate::{ScriptHost, Value};

/// How this build lays a value out, measured: `Value` (`repr(u8)`: its
/// kind's byte, then what it holds), an `Rc` of a string or a slice (a
/// pointer to its counts and contents, then a length), and a map's field.
/// A module reads values by `abi::LAYOUT`; this must be that, or none
/// loads.
fn measured() -> Result<Layout, String> {
    /// The word at `offset` in `value`.
    fn word<T>(value: &T, offset: usize) -> usize {
        // SAFETY: Callers read inside `value`, where a pointer or a length is.
        unsafe {
            std::ptr::from_ref(value)
                .cast::<u8>()
                .add(offset)
                .cast::<usize>()
                .read()
        }
    }
    fn byte<T>(value: &T, offset: usize) -> u8 {
        // SAFETY: Callers read an initialized byte inside `value`.
        unsafe { std::ptr::from_ref(value).cast::<u8>().add(offset).read() }
    }
    let unlike = |what: &str| {
        Err(format!(
            "this build lays {what} out another way than modules read it"
        ))
    };
    if size_of::<Value>() != abi::VALUE_SIZE
        || align_of::<Value>() != 8
        || size_of::<Raw>() != abi::VALUE_SIZE
    {
        return unlike("a value");
    }
    // Kinds by their numbers, a boolean's byte and a number's place.
    let (yes, number) = (Value::Bool(true), Value::Num(6.5));
    if byte(&Value::Nil, 0) != abi::NIL
        || (byte(&yes, 0), byte(&yes, abi::BOOL_AT)) != (abi::BOOL, 1)
        || byte(&number, 0) != abi::NUM
        || word(&number, abi::HELD_AT) as u64 != 6.5f64.to_bits()
    {
        return unlike("a boolean or a number");
    }
    // A string: its counted pointer, then its length; from that pointer,
    // where its count is and where its bytes are.
    let text: Rc<str> = Rc::from("layout");
    let string = Value::Str(text.clone());
    let pointer = word(&string, abi::HELD_AT);
    let data_at = (Rc::as_ptr(&text) as *const u8 as usize).wrapping_sub(pointer);
    // SAFETY: Read only if the pointer is `DATA_AT` before the text, so
    // inside the `Rc`'s allocation, where its counts are.
    let counts =
        |held: usize| unsafe { ((pointer + abi::COUNT_AT) as *const usize).read() == held };
    if byte(&string, 0) != abi::STR
        || word(&string, abi::HELD_AT + 8) != text.len()
        || data_at != abi::DATA_AT
        || !counts(2)
    {
        return unlike("a string");
    }
    drop(text);
    if !counts(1) {
        return unlike("a string's count");
    }
    // A list and a map: the same shape, their contents a value or a field
    // apart.
    let items: Rc<[Value]> = Rc::from(vec![Value::Num(1.), Value::Num(2.)]);
    let list = Value::List(items.clone());
    if byte(&list, 0) != abi::LIST
        || word(&list, abi::HELD_AT) + abi::DATA_AT != Rc::as_ptr(&items) as *const Value as usize
        || word(&list, abi::HELD_AT + 8) != 2
    {
        return unlike("a list");
    }
    let fields: Rc<[(Rc<str>, Value)]> = Rc::from(vec![(Rc::from("key"), Value::Num(1.))]);
    let map = Value::Map(fields.clone());
    let field = &fields[0];
    let key_at = std::ptr::from_ref(&field.0) as usize - std::ptr::from_ref(field) as usize;
    let value_at = std::ptr::from_ref(&field.1) as usize - std::ptr::from_ref(field) as usize;
    if byte(&map, 0) != abi::MAP
        || word(&map, abi::HELD_AT) + abi::DATA_AT
            != Rc::as_ptr(&fields) as *const (Rc<str>, Value) as usize
        || word(&map, abi::HELD_AT + 8) != 1
        || word(&field.0, 0) + abi::DATA_AT != Rc::as_ptr(&field.0) as *const u8 as usize
        || word(&field.0, 8) != 3
    {
        return unlike("a map");
    }
    Ok(Layout {
        value_size: size_of::<Value>() as u16,
        bool_at: abi::BOOL_AT as u16,
        held_at: abi::HELD_AT as u16,
        count_at: abi::COUNT_AT as u16,
        data_at: data_at as u16,
        field_size: size_of::<(Rc<str>, Value)>() as u16,
        field_key_at: key_at as u16,
        field_value_at: value_at as u16,
    })
}

/// `value` written to `out`, a module's from then.
///
/// # Safety
/// `out` has room for a value, and holds none.
#[inline(always)]
unsafe fn give(value: Value, out: *mut Raw) {
    // SAFETY: A `Raw` is a value's size and alignment (`measured`).
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
        give(Value::Str(Rc::from(text)), out);
    }
}
unsafe extern "C" fn list(items: *mut Raw, len: usize, out: *mut Raw) {
    // SAFETY: `len` owned values, taken once each.
    unsafe {
        let items: Rc<[Value]> = (0..len).map(|i| take(items.add(i))).collect();
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
            Value::Text(text) => Value::Str(Rc::from(text.to_string())),
            _ => Value::Nil,
        };
        give(whole, out);
    }
}

/// The value functions every module is given, once this build's layout is
/// known to be the one modules read.
fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(|| {
        let layout = measured()?;
        if layout != abi::LAYOUT {
            return Err(format!(
                "this build lays values out as {layout:?}, and modules read them as {:?}",
                abi::LAYOUT
            ));
        }
        Ok(Api {
            abi: abi::ABI,
            size: size_of::<Api>() as u32,
            layout,
            string,
            list,
            map,
            slice,
            clone,
            release,
            open,
            flat,
        })
    })
    .as_ref()
    .map_err(Clone::clone)
}

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
            action: register_action,
            state,
        };
        // SAFETY: The tables an entry is called with, `registrar` alive
        // through the call.
        match unsafe { entry(api()?, &registrar) } {
            0 => Ok(()),
            abi::REFUSED_ABI => Err(format!(
                "built for another module interface than this runtime's ({})",
                abi::ABI
            )),
            abi::REFUSED_LAYOUT => {
                Err("built to read values another way than this runtime lays them out".into())
            }
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

#[cfg(test)]
mod tests {
    /// Modules read values by `abi::LAYOUT`: this build's own, measured, is
    /// that. A compiler that laid an `Rc` or a field out another way would
    /// fail here, and at run time load no module rather than misread one.
    #[test]
    fn this_build_lays_values_out_as_modules_read_them() {
        assert_eq!(super::measured(), Ok(zgx_module::abi::LAYOUT));
    }
}
