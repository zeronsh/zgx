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
//! A module says what it registers in a manifest (`abi::MANIFEST`), which
//! is read as it loads and held to: one registering anything else is
//! refused. Scripts call a module's functions by name (`ui.call_sync`) or
//! as members of its table (`ui.module`), and a call that fails (an
//! argument of the wrong kind) is raised in the script, naming the function
//! and the module.
//!
//! Only an app loads modules, by a path it names in Rust: nothing a script
//! or a plugin can say loads one.

use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    mem::{ManuallyDrop, MaybeUninit},
    path::Path,
    rc::{Rc, Weak},
};

use mlua::{Lua, Table};
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

/// What a module registered, as its entry ran: kept aside until the module
/// is seen to register what its manifest says, then given to the host.
enum Kind {
    Function(abi::Function),
    Taking(abi::FunctionTaking),
    Action(abi::Action),
}
struct Pending {
    name: String,
    kind: Kind,
    user: User,
}
/// What a registrar's `host` points at while a module's entry runs.
#[derive(Default)]
struct Loading {
    pending: RefCell<Vec<Pending>>,
}

unsafe fn name_of(name: *const u8, len: usize) -> String {
    // SAFETY: The module's `&str`.
    unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(name, len)) }.to_owned()
}
/// # Safety
/// `host` is the `Loading` `enter` made, `name` the module's `&str`.
unsafe fn pend(host: *mut c_void, name: *const u8, len: usize, kind: Kind, user: User) {
    // SAFETY: The caller's.
    let (loading, name) = unsafe { (&*(host as *const Loading), name_of(name, len)) };
    loading
        .pending
        .borrow_mut()
        .push(Pending { name, kind, user });
}
unsafe extern "C" fn register_function(
    host: *mut c_void,
    name: *const u8,
    len: usize,
    call: abi::Function,
    user: *mut c_void,
    free: abi::Free,
) {
    // SAFETY: Called by a module's entry with the registrar's own `host`.
    unsafe {
        pend(
            host,
            name,
            len,
            Kind::Function(call),
            User { ptr: user, free },
        )
    }
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
    unsafe {
        pend(
            host,
            name,
            len,
            Kind::Taking(call),
            User { ptr: user, free },
        )
    }
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
    unsafe {
        pend(
            host,
            name,
            len,
            Kind::Action(call),
            User { ptr: user, free },
        )
    }
}

/// What a module's function or action is called with, and what an error
/// names it by.
struct Registered {
    user: User,
    name: Rc<str>,
    module: Rc<str>,
}
impl Registered {
    /// The error of a call that failed: `why` is what the module wrote
    /// where its result goes.
    #[cold]
    #[inline(never)]
    fn error(&self, why: Value) -> String {
        let why = match &why {
            Value::Str(why) => &**why,
            _ => "failed, and didn't say why",
        };
        format!("{} (module `{}`): {why}", self.name, self.module)
    }
}

thread_local! {
    /// Whether a module's call failed on this thread and no one has yet
    /// raised it: read after each call of a host's function or action, so
    /// a flag, and the error apart.
    static FAILED: Cell<bool> = const { Cell::new(false) };
    static FAILURE: RefCell<String> = const { RefCell::new(String::new()) };
}
/// Keeps a failed call's error for whoever called to raise ([`failure`]):
/// a host's functions return values, with no way to fail.
#[cold]
#[inline(never)]
fn fail(registered: &Registered, why: Value) {
    FAILURE.set(registered.error(why));
    FAILED.set(true);
}
/// The error of the module call that just failed on this thread, if one
/// did: asked after every call of a host's function or action, since one
/// may be a module's.
#[inline(always)]
pub(crate) fn failure() -> Option<mlua::Error> {
    if !FAILED.get() {
        return None;
    }
    taken()
}
#[cold]
#[inline(never)]
fn taken() -> Option<mlua::Error> {
    FAILED.set(false);
    Some(mlua::Error::runtime(FAILURE.take()))
}

/// A module's function as a host's own (`ScriptHost::function`), called
/// with one argument.
fn function_of(call: abi::Function, registered: Rc<Registered>) -> impl Fn(Value) -> Value {
    let user = registered.user.ptr;
    move |arg| {
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: The module's function and its own `user`, alive with
        // `registered`; the argument is lent for the call, where it is, and
        // what it writes is owned.
        unsafe {
            let status = call(user, std::ptr::from_ref(&arg).cast(), 1, out.as_mut_ptr());
            let made = take(out.as_mut_ptr());
            if status != abi::DONE {
                fail(&registered, made);
                return Value::Nil;
            }
            made
        }
    }
}
fn taking_of(call: abi::FunctionTaking, registered: Rc<Registered>) -> impl Fn(Value) -> Value {
    let user = registered.user.ptr;
    move |arg| {
        // (The module's from here: it frees it, or hands it back.)
        let mut arg = ManuallyDrop::new(arg);
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: As `function_of`'s; the argument is given.
        unsafe {
            let status = call(
                user,
                std::ptr::from_mut::<Value>(&mut arg).cast(),
                out.as_mut_ptr(),
            );
            let made = take(out.as_mut_ptr());
            if status != abi::DONE {
                fail(&registered, made);
                return Value::Nil;
            }
            made
        }
    }
}
fn action_of(call: abi::Action, registered: Rc<Registered>) -> impl Fn(Value) {
    let user = registered.user.ptr;
    move |arg| {
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: As `function_of`'s; `out` is written only failing.
        unsafe {
            if call(user, std::ptr::from_ref(&arg).cast(), out.as_mut_ptr()) != abi::DONE {
                fail(&registered, take(out.as_mut_ptr()));
            }
        }
    }
}

/// How a module's function is called.
#[derive(Clone, Copy)]
enum Call {
    Lent(abi::Function),
    Taking(abi::FunctionTaking),
}
/// A function of a module's table (`ui.module`).
struct Member {
    /// What a script calls it on the table.
    member: String,
    /// How many arguments its manifest gives it; one where it has none.
    arity: usize,
    call: Call,
    registered: Rc<Registered>,
}
/// A module loaded into a host.
pub(crate) struct Module {
    name: Rc<str>,
    members: Vec<Member>,
    /// Its manifest's JSON.
    manifest: Rc<str>,
}

thread_local! {
    /// The modules loaded into this thread's hosts: by the host's functions
    /// (what a script's VM has of its host), kept here so a host that loads
    /// none is no larger.
    static LOADED: RefCell<Vec<(Weak<FunctionsOf>, Rc<Module>)>> = const { RefCell::new(Vec::new()) };
    /// Those that didn't load, by name, and why: what `ui.module` says of
    /// one.
    static REFUSED: RefCell<Vec<(Weak<FunctionsOf>, String, String)>> = const { RefCell::new(Vec::new()) };
}

/// Why the module `name` didn't load into this host, if it was refused.
fn refused(functions: &crate::Functions, name: &str) -> Option<String> {
    REFUSED.with_borrow(|all| {
        all.iter()
            .find(|(host, refused, _)| {
                std::ptr::eq(host.as_ptr(), Rc::as_ptr(functions)) && refused == name
            })
            .map(|(.., why)| why.clone())
    })
}
type FunctionsOf = <crate::Functions as std::ops::Deref>::Target;

fn loaded(functions: &crate::Functions, name: &str) -> Result<Rc<Module>, Vec<String>> {
    LOADED.with_borrow(|all| {
        let of_host = || {
            all.iter()
                .filter(|(host, _)| std::ptr::eq(host.as_ptr(), Rc::as_ptr(functions)))
                .map(|(_, module)| module)
        };
        of_host()
            .find(|module| &*module.name == name)
            .cloned()
            .ok_or_else(|| of_host().map(|module| module.name.to_string()).collect())
    })
}

/// What a manifest says a module registers.
struct Registers {
    /// Each function's name, its member name and how many arguments.
    functions: Vec<(String, String, usize)>,
    actions: Vec<String>,
}
/// What a manifest says of a module.
struct Manifest {
    name: String,
    /// What it registers, if it says.
    registers: Option<Registers>,
}
fn manifest_of(json: &str) -> Result<Manifest, String> {
    let read = crate::json::parse(json).map_err(|e| format!("its manifest isn't JSON ({e})"))?;
    let number = |key: &str| read.field(key).and_then(Value::as_f64);
    if number("zgx_module") != Some(f64::from(abi::MANIFEST_FORMAT)) {
        return Err(format!(
            "its manifest is of another format than this runtime reads ({})",
            abi::MANIFEST_FORMAT
        ));
    }
    if number("abi") != Some(f64::from(abi::ABI)) {
        return Err(format!(
            "its manifest is for another module interface than this runtime's ({})",
            abi::ABI
        ));
    }
    let name = match read.field("name") {
        Some(Value::Str(name)) if abi::is_name(name) => name.to_string(),
        _ => return Err("its manifest doesn't name it (letters, digits and `_`)".into()),
    };
    let fields = |key: &str| match read.field(key) {
        Some(Value::Map(fields)) => fields
            .iter()
            .map(|Field(k, v)| (k.to_string(), v.clone()))
            .collect(),
        _ => Vec::new(),
    };
    let registers = read.field("functions").is_some().then(|| {
        let functions = fields("functions")
            .into_iter()
            .map(|(name, function): (String, Value)| {
                let member = match function.field("member") {
                    Some(Value::Str(member)) => member.to_string(),
                    _ => member_of(&name).to_owned(),
                };
                let arity = function.field("args").map_or(1, |args| args.list().len());
                (name, member, arity)
            })
            .collect();
        let actions = fields("actions")
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        Registers { functions, actions }
    });
    Ok(Manifest { name, registers })
}
/// The name a manifest's JSON gives, read whatever else it says.
fn name_in(json: &str) -> Option<&str> {
    // (Its first `"name":"…"`: the manifest's own comes before any record's
    // or function's, as the macros and `register!` write it.)
    let at = json.find(r#""name":""#)? + r#""name":""#.len();
    let name = &json[at..at + json[at..].find('"')?];
    abi::is_name(name).then_some(name)
}

/// What a function with no manifest is called on its module's table: its
/// name's last part.
fn member_of(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// Whether what a module registered is what its manifest says, or what
/// differs.
fn held_to(said: &Registers, pending: &[Pending]) -> Result<(), String> {
    let (functions, actions) = (&said.functions, &said.actions);
    let registered = |action: bool| {
        let mut names: Vec<&str> = pending
            .iter()
            .filter(|p| matches!(p.kind, Kind::Action(_)) == action)
            .map(|p| &*p.name)
            .collect();
        names.sort_unstable();
        names
    };
    let mut functions: Vec<&str> = functions.iter().map(|(name, ..)| &**name).collect();
    let mut actions: Vec<&str> = actions.iter().map(|name| &**name).collect();
    functions.sort_unstable();
    actions.sort_unstable();
    for (what, said, did) in [
        ("function", functions, registered(false)),
        ("action", actions, registered(true)),
    ] {
        if let Some(missing) = said.iter().find(|name| !did.contains(name)) {
            return Err(format!(
                "its manifest has a {what} `{missing}` it doesn't register"
            ));
        }
        if let Some(extra) = did.iter().find(|name| !said.contains(name)) {
            return Err(format!(
                "it registers a {what} `{extra}` its manifest doesn't have"
            ));
        }
        if did.len() != said.len() {
            return Err(format!("it registers a {what} twice"));
        }
    }
    Ok(())
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
    static STATES: RefCell<std::collections::HashMap<String, ModuleState>> = Default::default();
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

/// What a script passed a module's function, as the values it is lent.
/// Stated by count so no call allocates for them: a function of up to four
/// arguments reads them off the script's stack.
macro_rules! member_function {
    ($lua:expr, $member:expr, $($arg:ident),*) => {{
        let (call, registered) = ($member.call, $member.registered.clone());
        let user = registered.user.ptr;
        $lua.create_function(move |lua, ($($arg,)*): ($(member_function!(@value $arg),)*)| {
            let args = [$($arg,)*];
            called(lua, call, user, &registered, args)
        })
    }};
    (@value $arg:ident) => { Value };
}
/// A member's call: guarded as `ui.call_sync` of its name is, its
/// arguments lent (or the first given), its failure raised.
#[inline(always)]
fn called<const N: usize>(
    lua: &Lua,
    call: Call,
    user: *mut c_void,
    registered: &Registered,
    args: [Value; N],
) -> mlua::Result<Value> {
    crate::rules::guard(lua, crate::GuardKind::Function, &registered.name)?;
    // (Called with none, a module is still lent a value to read.)
    let none = Value::Nil;
    let mut out = MaybeUninit::<Raw>::uninit();
    // SAFETY: The module's function and its own `user`, alive with
    // `registered`; `args` are lent for the call (one is always there), or
    // the first given; what it writes is owned.
    let (status, made) = unsafe {
        let status = match call {
            Call::Lent(call) => {
                let lent: *const Value = if N == 0 { &none } else { args.as_ptr() };
                call(user, lent.cast(), N, out.as_mut_ptr())
            }
            Call::Taking(call) => {
                let mut args = args.into_iter();
                let mut first = ManuallyDrop::new(args.next().unwrap_or(Value::Nil));
                call(
                    user,
                    std::ptr::from_mut::<Value>(&mut first).cast(),
                    out.as_mut_ptr(),
                )
            }
        };
        (status, take(out.as_mut_ptr()))
    };
    if status != abi::DONE {
        return Err(mlua::Error::runtime(registered.error(made)));
    }
    Ok(made)
}
/// A member of more arguments than `member_function!` states.
fn member_of_many(lua: &Lua, member: &Member) -> mlua::Result<mlua::Function> {
    let (call, registered) = (member.call, member.registered.clone());
    let user = registered.user.ptr;
    lua.create_function(move |lua, args: mlua::Variadic<Value>| {
        crate::rules::guard(lua, crate::GuardKind::Function, &registered.name)?;
        let mut args = args.into_iter().collect::<Vec<Value>>();
        let len = args.len();
        if args.is_empty() {
            args.push(Value::Nil);
        }
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: As `called`'s.
        let (status, made) = unsafe {
            let status = match call {
                Call::Lent(call) => call(user, args.as_ptr().cast(), len, out.as_mut_ptr()),
                Call::Taking(call) => {
                    let mut first = ManuallyDrop::new(args.swap_remove(0));
                    call(
                        user,
                        std::ptr::from_mut::<Value>(&mut first).cast(),
                        out.as_mut_ptr(),
                    )
                }
            };
            (status, take(out.as_mut_ptr()))
        };
        if status != abi::DONE {
            return Err(mlua::Error::runtime(registered.error(made)));
        }
        Ok(made)
    })
}

/// `ui.module(name)`: a loaded module's functions as a table, each called
/// with its arguments as its manifest lists them. The app's own scripts
/// only: a plugin reaches what the app exposes to it, by name
/// (`ui.call_sync`).
pub(crate) fn install(lua: &Lua, ui: &Table) -> mlua::Result<()> {
    ui.set(
        "module",
        lua.create_function(|lua, name: String| {
            let functions = {
                let env = crate::node::env(lua)?;
                if !matches!(*env.grants, crate::Grants::All) {
                    return Err(mlua::Error::runtime(format!(
                        "ui.module `{name}`: a module is the app's own; a plugin calls what the app exposes with ui.call_sync"
                    )));
                }
                env.functions.clone()
            };
            let module = loaded(&functions, &name).map_err(|names| {
                if let Some(why) = refused(&functions, &name) {
                    return mlua::Error::runtime(format!("module `{name}` didn't load: {why}"));
                }
                mlua::Error::runtime(match names.as_slice() {
                    [] => format!("no module named `{name}`: the app loaded none"),
                    names => format!(
                        "no module named `{name}`: the app loaded {}",
                        names
                            .iter()
                            .map(|n| format!("`{n}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                })
            })?;
            let table = lua.create_table_with_capacity(0, module.members.len())?;
            for member in &module.members {
                let function = match member.arity {
                    0 => member_function!(lua, member,),
                    1 => member_function!(lua, member, a),
                    2 => member_function!(lua, member, a, b),
                    3 => member_function!(lua, member, a, b, c),
                    4 => member_function!(lua, member, a, b, c, d),
                    _ => member_of_many(lua, member),
                }?;
                table.raw_set(&*member.member, function)?;
            }
            table.set_readonly(true);
            Ok(table)
        })?,
    )
}

/// A symbol of the module `name`: `prefix` and the name, as the loader
/// takes it.
fn symbol(prefix: &str, name: &str) -> Vec<u8> {
    format!("{prefix}{name}\0").into_bytes()
}

/// The manifest the library loaded has for the module `name`, which must be
/// `in_file`, what its file has.
///
/// # Safety
/// `library` is a library loaded.
unsafe fn manifest_loaded(
    library: &libloading::Library,
    name: &str,
    in_file: &str,
) -> Result<(), String> {
    // SAFETY: A manifest's symbol is its bytes (`abi::MANIFEST`): asked
    // for as a pointer, it is where they are.
    let at: *const u8 = *unsafe { library.get::<*const u8>(&symbol(abi::MANIFEST, name)) }
        .map_err(|e| {
            format!("its file has a manifest, but the library has no symbol for it ({e})")
        })?;
    // SAFETY: At least a manifest's head is there, then as much as it says:
    // read in two steps, each no further than what is known to be the
    // symbol's.
    let in_memory = unsafe {
        let head = std::slice::from_raw_parts(at, abi::MANIFEST_HEAD);
        if head[..16] != abi::MANIFEST_MAGIC {
            return Err("its manifest's symbol doesn't begin as a manifest does".into());
        }
        let len = u32::from_le_bytes(head[20..24].try_into().unwrap()) as usize;
        abi::manifest_in(std::slice::from_raw_parts(
            at,
            abi::MANIFEST_HEAD + len + 16,
        ))
    };
    match in_memory {
        Some(in_memory) if in_memory == in_file => Ok(()),
        Some(_) => Err("the manifest in its file isn't the one it loaded with".into()),
        None => Err(
            "the manifest it loaded with isn't whole (its length, checksum or end is wrong)".into(),
        ),
    }
}

/// A host's function as its own Rust calls it (`registered_function`): a
/// module's that fails gives its error.
#[doc(hidden)]
pub type RegisteredFunction = Rc<dyn Fn(Value) -> Result<Value, String>>;
/// A host's action as its own Rust calls it (`registered_action`).
#[doc(hidden)]
pub type RegisteredAction = Rc<dyn Fn(Value) -> Result<(), String>>;

impl ScriptHost {
    /// Loads a native module (a library built with the `zgx-module` crate)
    /// and lets it register its functions and actions with this host, as
    /// `function` and `action` would: scripts call them the same way, and
    /// plugins reach them only if exposed (`expose_action`). The app's own
    /// scripts also get them as a table, by the module's name
    /// (`ui.module`).
    ///
    /// A module's manifest is read from its file, which is how its name is
    /// known, and again from the library loaded: they must be one, and the
    /// module must register what it says and nothing else, or it is refused
    /// and nothing of it is registered.
    ///
    /// The library stays loaded for as long as the process runs: what it
    /// registered may be called, and values it made freed, at any time.
    ///
    /// # Safety
    /// Loading a library runs its code: `path` must be a module the app
    /// itself built or shipped, never one a script, a plugin or a person
    /// named.
    #[doc(hidden)]
    pub unsafe fn load_module(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        // SAFETY: The caller's.
        let loaded = unsafe { self.load_module_at(path) };
        if let Err(why) = &loaded {
            // Kept, for `ui.module` to say: by the name its manifest gives,
            // else its file's.
            let name = std::fs::read(path)
                .ok()
                .and_then(|file| abi::manifest_in(&file).and_then(name_in).map(str::to_owned))
                .unwrap_or_else(|| {
                    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
                    let stem = stem.unwrap_or_default();
                    stem.strip_prefix("lib").unwrap_or(&stem).to_owned()
                });
            REFUSED.with_borrow_mut(|all| {
                all.retain(|(host, ..)| host.strong_count() > 0);
                all.push((Rc::downgrade(&self.0.functions), name, why.clone()));
            });
        }
        loaded
    }

    /// # Safety
    /// As `load_module`'s.
    unsafe fn load_module_at(&self, path: &Path) -> Result<(), String> {
        let fail = |why: &dyn std::fmt::Display| format!("{}: {why}", path.display());
        let file = std::fs::read(path).map_err(|e| fail(&e))?;
        let in_file = abi::manifest_in(&file)
            .ok_or_else(|| fail(&"not a zgx module (its file has no manifest)"))?;
        let said = manifest_of(in_file).map_err(|e| fail(&e))?;
        // SAFETY: The caller's promise.
        let library = unsafe { libloading::Library::new(path) }.map_err(|e| fail(&e))?;
        // SAFETY: The library just loaded.
        unsafe { manifest_loaded(&library, &said.name, in_file) }.map_err(|e| fail(&e))?;
        // SAFETY: A module's entry is an `abi::Entry` by that name.
        let entry = *unsafe { library.get::<abi::Entry>(&symbol(abi::ENTRY, &said.name)) }
            .map_err(|e| fail(&format_args!("it has no entry ({e})")))?;
        // (Never unloaded: closures and values of its code outlive this.)
        std::mem::forget(library);
        // SAFETY: The module's entry, by the interface it was built to.
        unsafe { self.enter(entry, in_file, said) }.map_err(|e| fail(&e))
    }

    /// Registers a module linked into the program itself, by its entry and
    /// its manifest (`zgx_module_v1` and `zgx_module_manifest`, which
    /// `zgx_module::module` and `register!` write where they are used): the
    /// same registration `load_module` does, with no library to load.
    ///
    /// # Safety
    /// `entry` is a `zgx_module` entry, and `manifest` that module's.
    #[doc(hidden)]
    pub unsafe fn enter_module(&self, entry: abi::Entry, manifest: &[u8]) -> Result<(), String> {
        let manifest = abi::manifest_in(manifest)
            .ok_or("its manifest isn't whole (its length, checksum or end is wrong)")?;
        let said = manifest_of(manifest)?;
        // SAFETY: The caller's.
        unsafe { self.enter(entry, manifest, said) }
    }

    /// # Safety
    /// `entry` is a module's, `manifest` its own, and `said` what that says.
    unsafe fn enter(
        &self,
        entry: abi::Entry,
        manifest: &str,
        said: Manifest,
    ) -> Result<(), String> {
        let loading = Loading::default();
        let registrar = Registrar {
            size: size_of::<Registrar>() as u32,
            host: std::ptr::from_ref(&loading) as *mut c_void,
            function: register_function,
            function_taking: register_function_taking,
            action: register_action,
            state,
        };
        // SAFETY: The tables an entry is called with, `registrar` alive
        // through the call.
        match unsafe { entry(&API, &registrar) } {
            0 => {}
            abi::REFUSED_ABI => {
                return Err(format!(
                    "built for another module interface than this runtime's ({})",
                    abi::ABI
                ));
            }
            code => return Err(format!("the module refused to load ({code})")),
        }
        let pending = loading.pending.into_inner();
        if let Some(registers) = &said.registers {
            // (Refused, what it registered is freed here, unused.)
            held_to(registers, &pending)?;
        }
        let module: Rc<str> = Rc::from(&*said.name);
        let mut members = Vec::new();
        for Pending { name, kind, user } in pending {
            let registered = Rc::new(Registered {
                user,
                name: Rc::from(&*name),
                module: module.clone(),
            });
            let member = |call| {
                let listed = said
                    .registers
                    .iter()
                    .flat_map(|said| &said.functions)
                    .find(|(function, ..)| *function == name);
                Member {
                    member: listed
                        .map_or_else(|| member_of(&name).to_owned(), |(_, m, _)| m.clone()),
                    arity: listed.map_or(1, |(.., arity)| *arity),
                    call,
                    registered: registered.clone(),
                }
            };
            match kind {
                Kind::Function(call) => {
                    members.push(member(Call::Lent(call)));
                    self.function(name, function_of(call, registered));
                }
                Kind::Taking(call) => {
                    members.push(member(Call::Taking(call)));
                    self.function(name, taking_of(call, registered));
                }
                Kind::Action(call) => self.action(name, action_of(call, registered)),
            }
        }
        LOADED.with_borrow_mut(|all| {
            all.retain(|(host, _)| host.strong_count() > 0);
            let host = Rc::downgrade(&self.0.functions);
            // (Loaded again into a host, a module's table is the last's.)
            all.retain(|(of, loaded)| !(of.ptr_eq(&host) && loaded.name == module));
            all.push((
                host,
                Rc::new(Module {
                    name: module,
                    members,
                    manifest: Rc::from(manifest),
                }),
            ));
        });
        Ok(())
    }

    /// The manifests of the modules loaded into this host, as JSON, in the
    /// order they were loaded.
    #[doc(hidden)]
    pub fn module_manifests(&self) -> Vec<Rc<str>> {
        LOADED.with_borrow(|all| {
            all.iter()
                .filter(|(host, _)| std::ptr::eq(host.as_ptr(), Rc::as_ptr(&self.0.functions)))
                .map(|(_, module)| module.manifest.clone())
                .collect()
        })
    }

    /// The function registered as `name` (`function`, or a module's), for
    /// the host's own Rust to call: what a module answers scripts with,
    /// asked from Rust. A module's call that fails gives its error.
    #[doc(hidden)]
    pub fn registered_function(&self, name: &str) -> Option<RegisteredFunction> {
        let function = self.0.functions.borrow().get(name).cloned()?;
        Some(Rc::new(move |arg| {
            let made = function(arg);
            match failure() {
                None => Ok(made),
                Some(error) => Err(error.to_string()),
            }
        }))
    }
    /// The action registered as `name` (`action`, or a module's), for the
    /// host's own Rust to call. A module's call that fails gives its error.
    #[doc(hidden)]
    pub fn registered_action(&self, name: &str) -> Option<RegisteredAction> {
        let action = self.0.actions.borrow().get(name).cloned()?;
        Some(Rc::new(move |arg| {
            action(arg);
            match failure() {
                None => Ok(()),
                Some(error) => Err(error.to_string()),
            }
        }))
    }
}
