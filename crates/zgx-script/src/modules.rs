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
    sync::{Arc, Mutex},
    task::{Poll, Waker},
    time::Duration,
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
    host_retain,
    host_release,
    post,
    after,
    timer_release,
    state,
    shared,
    shared_get,
    shared_set,
    shared_append,
    shared_observe,
    shared_unobserve,
    shared_release,
    settle,
    promise_release,
    promise_host,
    shared_upsert,
    shared_remove,
    image_buffer,
    image_set,
    image_drop,
    image_remove,
    shared_upsert_at,
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
    Async(abi::AsyncAction),
}
struct Pending {
    name: String,
    kind: Kind,
    user: User,
}
/// What a registrar's `host` points at while a module's entry runs.
struct Loading {
    pending: RefCell<Vec<Pending>>,
    /// Its line highlighters, registered with the host once it has loaded.
    highlighters: RefCell<Vec<(String, crate::native_highlight::Native)>>,
    /// The host it loads into, for a handle to it (`host_ref`).
    host: ScriptHost,
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

unsafe extern "C" fn register_async_action(
    host: *mut c_void,
    name: *const u8,
    len: usize,
    call: abi::AsyncAction,
    user: *mut c_void,
    free: abi::Free,
) {
    // SAFETY: As `register_function`'s.
    unsafe { pend(host, name, len, Kind::Async(call), User { ptr: user, free }) }
}

/// `Host::highlighter`: kept with the module's other registrations until
/// it has loaded, then the host's (`ui.highlighter`).
unsafe extern "C" fn register_highlighter(
    host: *mut c_void,
    name: *const u8,
    len: usize,
    kinds: *const abi::Bytes,
    count: usize,
    call: abi::Highlight,
    user: *mut c_void,
    free: abi::Free,
) {
    // SAFETY: The registrar's arguments, lent for the call; `host` the
    // `Loading` the registrar was made with.
    let (loading, name, kinds) = unsafe {
        let kinds = (0..count)
            .map(|k| {
                let kind = &*kinds.add(k);
                name_of(kind.ptr, kind.len)
            })
            .collect::<Vec<_>>();
        (&*(host as *const Loading), name_of(name, len), kinds)
    };
    let user = User { ptr: user, free };
    let line = move |line: &[u8], state: u32, push: &mut dyn FnMut(u32, u32, u32)| -> u32 {
        // (The whole `User`, not its pointer alone: it frees the module's
        // highlighter as this closure goes.)
        let user = &user;
        unsafe extern "C" fn sink(sink: *mut c_void, from: u32, to: u32, kind: u32) {
            // SAFETY: The `&mut dyn FnMut` lent below, for the call alone.
            unsafe { (*(sink as *mut &mut dyn FnMut(u32, u32, u32)))(from, to, kind) }
        }
        let mut push = push;
        // SAFETY: The module's highlighter, its `user` alive while this is,
        // `line` and the sink lent for the call.
        unsafe {
            call(
                user.ptr,
                line.as_ptr(),
                line.len(),
                state,
                (&raw mut push).cast(),
                sink,
            )
        }
    };
    loading.highlighters.borrow_mut().push((
        name.clone(),
        crate::native_highlight::Native {
            name,
            kinds,
            line: Box::new(line),
        },
    ));
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
    /// An async action: called as `ui.call(name, arg)` is, for a promise.
    Async,
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

/// Lets go of what this thread keeps of the host of `functions`, as it goes:
/// its modules' tables, and so (once its scripts' VMs are closed too) what
/// they registered, whose drops run then rather than at the thread's next
/// load. (Matched by address alone: the host's functions may outlive it in
/// a VM closing after it; the table holds a `Weak` of them, which keeps
/// their allocation, so no other host's can be at that address meanwhile.)
pub(crate) fn forget_host(functions: &crate::Functions) {
    let of = |host: &Weak<FunctionsOf>| std::ptr::eq(host.as_ptr(), Rc::as_ptr(functions));
    // (Taken out first: a module's drop may load another.)
    let gone: Vec<_> = LOADED.with_borrow_mut(|all| {
        let (gone, kept) = std::mem::take(all)
            .into_iter()
            .partition(|(host, _)| of(host));
        *all = kept;
        gone
    });
    REFUSED.with_borrow_mut(|all| all.retain(|(host, ..)| !of(host)));
    drop(gone);
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
    /// Each async action's name and member name.
    asyncs: Vec<(String, String)>,
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
        let asyncs = fields("async")
            .into_iter()
            .map(|(name, of): (String, Value)| {
                let member = match of.field("member") {
                    Some(Value::Str(member)) => member.to_string(),
                    _ => member_of(&name).to_owned(),
                };
                (name, member)
            })
            .collect();
        Registers {
            functions,
            actions,
            asyncs,
        }
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
    let registered = |of: fn(&Kind) -> bool| {
        let mut names: Vec<&str> = pending
            .iter()
            .filter(|p| of(&p.kind))
            .map(|p| &*p.name)
            .collect();
        names.sort_unstable();
        names
    };
    fn sorted(mut names: Vec<&str>) -> Vec<&str> {
        names.sort_unstable();
        names
    }
    let functions = sorted(said.functions.iter().map(|(name, ..)| &**name).collect());
    let actions = sorted(said.actions.iter().map(|name| &**name).collect());
    let asyncs = sorted(said.asyncs.iter().map(|(name, _)| &**name).collect());
    for (what, said, did) in [
        (
            "function",
            functions,
            registered(|k| matches!(k, Kind::Function(_) | Kind::Taking(_))),
        ),
        (
            "action",
            actions,
            registered(|k| matches!(k, Kind::Action(_))),
        ),
        (
            "async action",
            asyncs,
            registered(|k| matches!(k, Kind::Async(_))),
        ),
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

// A module's reach into its host: a handle to it any thread can post work
// to, run on the interface's thread, and timers. A host keeps nothing for
// this until a module asks for a handle to it (`host_ref`): a host's
// mailbox is made then, in a registry of this thread's, so a host and a VM
// are no larger for it, and nothing runs or waits for a host that has none.

/// Where a module's threads post work for a host (`Api::post`): a host
/// handle is a counted pointer to one. Drained on the host's thread by one
/// task, which waits on a waker: posting wakes it, and nothing else does.
struct Mailbox {
    thread: std::thread::ThreadId,
    queue: Mutex<Queue>,
}
struct Queue {
    jobs: Vec<Posted>,
    waker: Option<Waker>,
    /// Whether its host is there to run what is posted.
    open: bool,
}
impl Mailbox {
    fn queue(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }
}
/// A posted job and its `user`: the job frees it as it runs; a job that
/// doesn't run (its host gone) is freed by its `free`. A module's `user`
/// for a job may cross threads (`Api::post`).
struct Posted {
    job: abi::Job,
    user: User,
}
// SAFETY: What `Api::post` asks of a module: a job's `user` may be sent.
unsafe impl Send for Posted {}

thread_local! {
    /// The hosts on this thread a module holds a handle to, by their
    /// mailboxes.
    static REACHED: RefCell<Vec<(Weak<crate::Inner>, Arc<Mailbox>)>> = const { RefCell::new(Vec::new()) };
}
/// `host`'s mailbox: made the first time a module asks for a handle to it,
/// the task that drains it with it.
fn mailbox_of(host: &ScriptHost) -> Arc<Mailbox> {
    REACHED.with_borrow_mut(|all| {
        all.retain(|(host, _)| host.strong_count() > 0);
        if let Some((_, mailbox)) = all
            .iter()
            .find(|(of, _)| std::ptr::eq(of.as_ptr(), Rc::as_ptr(&host.0)))
        {
            return mailbox.clone();
        }
        let mailbox = Arc::new(Mailbox {
            thread: std::thread::current().id(),
            queue: Mutex::new(Queue {
                jobs: Vec::new(),
                waker: None,
                open: true,
            }),
        });
        host.0
            .timers
            .spawn(drained(mailbox.clone(), Rc::downgrade(&host.0)));
        all.push((Rc::downgrade(&host.0), mailbox.clone()));
        mailbox
    })
}
/// The host a handle is to, asked on its thread: none on another thread,
/// or once the host is gone.
///
/// # Safety
/// `handle` is a host handle (`host_ref`'s), held.
unsafe fn host_of(handle: *mut c_void) -> Option<ScriptHost> {
    // SAFETY: The caller's.
    let mailbox = unsafe { &*(handle as *const Mailbox) };
    if mailbox.thread != std::thread::current().id() {
        return None;
    }
    REACHED.with_borrow(|all| {
        all.iter()
            .find(|(_, of)| std::ptr::eq(Arc::as_ptr(of), mailbox))
            .and_then(|(host, _)| host.upgrade())
            .map(ScriptHost)
    })
}
/// Runs what is posted to `mailbox`, on its host's thread, as it comes;
/// closes it as the host goes (what waits is freed, unrun).
async fn drained(mailbox: Arc<Mailbox>, host: Weak<crate::Inner>) {
    struct Closing(Arc<Mailbox>);
    impl Drop for Closing {
        fn drop(&mut self) {
            let unrun = {
                let mut queue = self.0.queue();
                queue.open = false;
                queue.waker = None;
                std::mem::take(&mut queue.jobs)
            };
            drop(unrun);
        }
    }
    let _closing = Closing(mailbox.clone());
    let handle = Arc::as_ptr(&mailbox) as *mut c_void;
    // (Two lists traded turn by turn: posting allocates nothing once each
    // holds what a turn brings.)
    let mut jobs = Vec::new();
    loop {
        std::future::poll_fn(|cx| {
            let mut queue = mailbox.queue();
            if queue.jobs.is_empty() {
                queue.waker = Some(cx.waker().clone());
                return Poll::Pending;
            }
            std::mem::swap(&mut queue.jobs, &mut jobs);
            Poll::Ready(())
        })
        .await;
        let Some(host) = host.upgrade() else {
            return;
        };
        for Posted { job, user } in jobs.drain(..) {
            // (The job frees its `user` as it runs.)
            let user = ManuallyDrop::new(user);
            // SAFETY: The module's job and its own `user`, on the host's
            // thread, the handle lent (the mailbox is held here).
            host.runtime.batch(|| unsafe { job(user.ptr, handle) });
        }
    }
}

/// A handle to the host a module loads into (`Registrar::host_ref`).
unsafe extern "C" fn host_ref(loading: *mut c_void) -> *mut c_void {
    // SAFETY: The registrar's own `host`, inside the entry call.
    let loading = unsafe { &*(loading as *const Loading) };
    Arc::into_raw(mailbox_of(&loading.host)) as *mut c_void
}
unsafe extern "C" fn host_retain(host: *mut c_void) -> *mut c_void {
    // SAFETY: A host handle, held (any thread: the count is atomic).
    unsafe { Arc::increment_strong_count(host as *const Mailbox) };
    host
}
unsafe extern "C" fn host_release(host: *mut c_void) {
    // SAFETY: A host handle the module let go of, once.
    unsafe { Arc::decrement_strong_count(host as *const Mailbox) };
}
unsafe extern "C" fn post(
    host: *mut c_void,
    job: abi::Job,
    user: *mut c_void,
    free: abi::Free,
) -> u32 {
    // SAFETY: A host handle, held; any thread.
    let mailbox = unsafe { &*(host as *const Mailbox) };
    let posted = Posted {
        job,
        user: User { ptr: user, free },
    };
    let (waker, refused) = {
        let mut queue = mailbox.queue();
        if queue.open {
            queue.jobs.push(posted);
            (queue.waker.take(), None)
        } else {
            (None, Some(posted))
        }
    };
    // (Freed or woken outside the lock: either may post again.)
    if let Some(refused) = refused {
        drop(refused);
        return abi::GONE;
    }
    if let Some(waker) = waker {
        waker.wake();
    }
    abi::POSTED
}
unsafe extern "C" fn after(
    host: *mut c_void,
    ms: f64,
    repeat: u32,
    fire: abi::Fire,
    user: *mut c_void,
    free: abi::Free,
) -> *mut c_void {
    let user = User { ptr: user, free };
    // SAFETY: A host handle, held.
    let Some(script_host) = (unsafe { host_of(host) }) else {
        return std::ptr::null_mut();
    };
    // SAFETY: The same handle, held once more for the timer's calls.
    let mailbox = unsafe {
        Arc::increment_strong_count(host as *const Mailbox);
        Arc::from_raw(host as *const Mailbox)
    };
    let runtime = script_host.0.runtime.clone();
    let seconds = if ms.is_finite() && ms > 0. {
        ms / 1e3
    } else {
        0.
    };
    let period = Duration::from_secs_f64(seconds.min(zgx::timer::MAX_SLEEP.as_secs_f64()));
    let stop = script_host.0.timers.start(period, repeat != 0, move || {
        // (The whole `user`, not its pointer alone: the closure frees it as
        // the timer ends.)
        let user = &user;
        let handle = Arc::as_ptr(&mailbox) as *mut c_void;
        // SAFETY: The module's timer and its own `user`, alive with this
        // closure; the handle lent.
        runtime.batch(|| unsafe { fire(user.ptr, handle) }) != 0
    });
    Box::into_raw(Box::new(stop)) as *mut c_void
}
unsafe extern "C" fn timer_release(timer: *mut c_void, stop: u32) {
    // SAFETY: A timer `after` made, let go of once.
    let timer = *unsafe { Box::from_raw(timer as *mut crate::timers::Stop) };
    if stop == 0 {
        timer.detach();
    }
}

/// A module's handle to a host's shared state (`Api::shared`): its signal
/// while the host is there, and the host's handle, lent to observers. On
/// the interface's thread only.
struct SharedHandle {
    signal: zgx::reactive::Signal<Value>,
    host: Weak<crate::Inner>,
    mailbox: Arc<Mailbox>,
}
impl SharedHandle {
    /// Its signal, while its host is there: one load to know.
    #[inline]
    fn signal(&self) -> Option<&zgx::reactive::Signal<Value>> {
        (self.host.strong_count() != 0).then_some(&self.signal)
    }
}
unsafe extern "C" fn shared(
    host: *mut c_void,
    key: *const u8,
    len: usize,
    initial: *mut Raw,
) -> *mut c_void {
    // SAFETY: The module's owned value, taken.
    let initial = unsafe { take(initial) };
    // SAFETY: A host handle, held; the module's `&str`.
    let (script_host, key) = (unsafe { host_of(host) }, unsafe { name_of(key, len) });
    let Some(script_host) = script_host else {
        return std::ptr::null_mut();
    };
    // SAFETY: The same handle, held once more for the state's observers.
    let mailbox = unsafe {
        Arc::increment_strong_count(host as *const Mailbox);
        Arc::from_raw(host as *const Mailbox)
    };
    let signal = script_host.shared(&key, initial).signal();
    Box::into_raw(Box::new(SharedHandle {
        signal,
        host: Rc::downgrade(&script_host.0),
        mailbox,
    })) as *mut c_void
}
/// # Safety
/// `shared` is a handle `shared` made, held.
unsafe fn handle_of<'a>(shared: *const c_void) -> &'a SharedHandle {
    // SAFETY: The caller's.
    unsafe { &*(shared as *const SharedHandle) }
}
unsafe extern "C" fn shared_get(shared: *const c_void, out: *mut Raw) {
    // SAFETY: A handle, held.
    let value = unsafe { handle_of(shared) }
        .signal()
        .map_or(Value::Nil, |signal| signal.get());
    // SAFETY: `out` takes what is made.
    unsafe { out.cast::<Value>().write(value) }
}
/// Why a write to a shared state whose host is gone did nothing.
const HOST_GONE: &str = "its host is gone";
/// `out` given why a write failed: its host gone.
///
/// # Safety
/// `out` takes a value.
unsafe fn host_gone(out: *mut Raw) -> u32 {
    // SAFETY: The caller's.
    unsafe { out.cast::<Value>().write(Value::from(HOST_GONE)) };
    abi::FAILED
}
unsafe extern "C" fn shared_set(shared: *const c_void, value: *mut Raw) -> u32 {
    // SAFETY: The module's owned value, taken; a handle, held.
    let (value, handle) = unsafe { (take(value), handle_of(shared)) };
    match handle.signal() {
        Some(signal) => crate::value::set_reconciled(signal, value) as u32,
        None => abi::HOST_GONE,
    }
}
unsafe extern "C" fn shared_append(
    shared: *const c_void,
    text: *const u8,
    len: usize,
    out: *mut Raw,
) -> u32 {
    // SAFETY: A handle, held; the module's UTF-8.
    let (handle, text) = unsafe { (handle_of(shared), name_of(text, len)) };
    let Some(signal) = handle.signal() else {
        // SAFETY: `out` takes why.
        return unsafe { host_gone(out) };
    };
    match crate::Shared(signal.clone()).append(&text) {
        Ok(()) => abi::DONE,
        Err(why) => {
            // SAFETY: `out` takes what is made.
            unsafe { out.cast::<Value>().write(Value::from(why.as_str())) };
            abi::FAILED
        }
    }
}
unsafe extern "C" fn shared_upsert(
    shared: *const c_void,
    key: *const u8,
    key_len: usize,
    rows: *mut Raw,
    out: *mut Raw,
) -> u32 {
    // SAFETY: A handle, held; the module's `&str`; its owned list, taken.
    let (handle, key, rows) = unsafe { (handle_of(shared), name_of(key, key_len), take(rows)) };
    let Some(signal) = handle.signal() else {
        // SAFETY: `out` takes why.
        return unsafe { host_gone(out) };
    };
    let rows = match rows {
        Value::List(rows) => rows.to_vec(),
        Value::Nil => Vec::new(),
        _ => {
            // SAFETY: `out` takes why.
            unsafe { out.cast::<Value>().write(Value::from("rows are a list")) };
            return abi::FAILED;
        }
    };
    match crate::value::upsert_rows(signal, &key, rows) {
        Ok(_) => abi::DONE,
        Err(why) => {
            // SAFETY: `out` takes why.
            unsafe { out.cast::<Value>().write(Value::from(why.as_str())) };
            abi::FAILED
        }
    }
}
unsafe extern "C" fn shared_upsert_at(
    shared: *const c_void,
    key: *const u8,
    key_len: usize,
    rows: *mut Raw,
    places: *mut Raw,
    out: *mut Raw,
) -> u32 {
    use crate::value::Place;
    // SAFETY: A handle, held; the module's `&str`; its owned lists, taken.
    let (handle, key, rows, places) = unsafe {
        (
            handle_of(shared),
            name_of(key, key_len),
            take(rows),
            take(places),
        )
    };
    let failed = |why: String| {
        // SAFETY: `out` takes why.
        unsafe { out.cast::<Value>().write(Value::from(why.as_str())) };
        abi::FAILED
    };
    let Some(signal) = handle.signal() else {
        // SAFETY: `out` takes why.
        return unsafe { host_gone(out) };
    };
    let (rows, places) = match (rows, places) {
        (Value::List(rows), Value::List(places)) => (rows, places),
        (Value::Nil, Value::Nil) => return abi::DONE,
        _ => return failed("rows and their places are lists".into()),
    };
    if rows.len() != places.len() {
        return failed(format!("{} places for {} rows", places.len(), rows.len()));
    }
    let mut given = Vec::with_capacity(rows.len());
    for (i, (row, place)) in rows.iter().zip(places.iter()).enumerate() {
        let place = match place {
            Value::Nil => Place::Stay,
            Value::Bool(true) => Place::End,
            Value::Str(key) => Place::Before(key.to_string()),
            _ => {
                return failed(format!(
                    "row {}'s place is nil (where it is), a key (before it) or true (the end)",
                    i + 1
                ));
            }
        };
        given.push((row.clone(), place));
    }
    match crate::value::place_rows(signal, &key, given) {
        Ok(_) => abi::DONE,
        Err(why) => failed(why),
    }
}
unsafe extern "C" fn shared_remove(
    shared: *const c_void,
    key: *const u8,
    key_len: usize,
    keys: *const Bytes,
    len: usize,
) -> u32 {
    // SAFETY: A handle, held; the module's `&str`s, `len` of them.
    let (handle, key) = unsafe { (handle_of(shared), name_of(key, key_len)) };
    let Some(signal) = handle.signal() else {
        return abi::HOST_GONE;
    };
    let keys: Vec<&str> = (0..len)
        .map(|i| {
            // SAFETY: Among the `len` the module lent, each UTF-8.
            unsafe {
                let key = &*keys.add(i);
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(key.ptr, key.len))
            }
        })
        .collect();
    crate::value::remove_rows(signal, &key, &keys) as u32
}
/// A shared state's observer: its effect, and the module's `user` (freed as
/// it goes).
struct Observer {
    _effect: zgx::reactive::Effect,
}
unsafe extern "C" fn shared_observe(
    shared: *const c_void,
    call: abi::Observe,
    user: *mut c_void,
    free: abi::Free,
) -> *mut c_void {
    let user = User { ptr: user, free };
    // SAFETY: A handle, held.
    let handle = unsafe { handle_of(shared) };
    let (Some(signal), Some(host)) = (handle.signal().cloned(), handle.host.upgrade()) else {
        return std::ptr::null_mut();
    };
    let runtime = host.runtime.clone();
    let mailbox = handle.mailbox.clone();
    let mut first = true;
    let effect = host.runtime.effect(move || {
        // (The whole `user`, freed with the effect.)
        let user = &user;
        let value = signal.get();
        // (Not at once: from the next change.)
        if std::mem::take(&mut first) {
            return;
        }
        let handle = Arc::as_ptr(&mailbox) as *mut c_void;
        // SAFETY: The module's observer and its `user`, alive with the
        // effect; the value and the handle lent for the call, which reads
        // nothing it follows.
        runtime.untracked(|| unsafe { call(user.ptr, handle, std::ptr::from_ref(&value).cast()) });
    });
    Box::into_raw(Box::new(Observer { _effect: effect })) as *mut c_void
}
unsafe extern "C" fn shared_unobserve(subscription: *mut c_void) {
    // SAFETY: A subscription `shared_observe` made, let go of once.
    drop(unsafe { Box::from_raw(subscription as *mut Observer) });
}
unsafe extern "C" fn shared_release(shared: *mut c_void) {
    // SAFETY: A handle `shared` made, let go of once.
    drop(unsafe { Box::from_raw(shared as *mut SharedHandle) });
}

/// An image's pixels as a module writes them (`Api::image_buffer`): made
/// as the `Arc` the image shares, so setting it copies nothing.
type Buffer = Arc<[u8]>;
thread_local! {
    /// The images modules set on this thread's hosts, by name: what goes
    /// as a module is loaded into its host again (the host's own go with
    /// it). Made the first time one is set.
    static IMAGES: RefCell<Vec<(Weak<crate::Inner>, std::collections::HashSet<String>)>> =
        const { RefCell::new(Vec::new()) };
}
/// The images a module named `module` set on `host`, removed: as it is
/// loaded into it again.
fn remove_images_of(host: &ScriptHost, module: &str) {
    let gone: Vec<String> = IMAGES.with_borrow_mut(|all| {
        all.retain(|(host, _)| host.strong_count() > 0);
        let Some((_, names)) = all
            .iter_mut()
            .find(|(of, _)| std::ptr::eq(of.as_ptr(), Rc::as_ptr(&host.0)))
        else {
            return Vec::new();
        };
        let of = |name: &String| name.split_once('.').is_some_and(|(m, _)| m == module);
        let gone = names.iter().filter(|n| of(n)).cloned().collect();
        names.retain(|n| !of(n));
        gone
    });
    for name in gone {
        host.image(&name, None);
    }
}
unsafe extern "C" fn image_buffer(len: usize, data: *mut *mut u8) -> *mut c_void {
    if len == 0 || len > isize::MAX as usize / 2 {
        return std::ptr::null_mut();
    }
    // Zeroed as it is allocated (a large one's pages come zeroed), so a
    // module that writes some of it leaves nothing undefined.
    // SAFETY: All 0, so every byte a `u8`.
    let mut pixels: Box<Buffer> =
        Box::new(unsafe { Arc::<[u8]>::new_zeroed_slice(len).assume_init() });
    let bytes = Arc::get_mut(&mut pixels)
        .expect("made just now")
        .as_mut_ptr();
    // SAFETY: `data` takes where its bytes are.
    unsafe { data.write(bytes) };
    Box::into_raw(pixels) as *mut c_void
}
unsafe extern "C" fn image_drop(buffer: *mut c_void) {
    // SAFETY: A buffer `image_buffer` made, let go of once.
    drop(unsafe { Box::from_raw(buffer as *mut Buffer) });
}
unsafe extern "C" fn image_set(
    host: *mut c_void,
    name: *const u8,
    len: usize,
    width: u32,
    height: u32,
    buffer: *mut c_void,
    out: *mut Raw,
) -> u32 {
    // SAFETY: A buffer `image_buffer` made, taken; the module's `&str`;
    // a host handle, held.
    let (pixels, name, mailbox) = unsafe {
        (
            *Box::from_raw(buffer as *mut Buffer),
            name_of(name, len),
            &*(host as *const Mailbox),
        )
    };
    let failed = |why: String| {
        // SAFETY: `out` takes why.
        unsafe { out.cast::<Value>().write(Value::from(why.as_str())) };
        abi::FAILED
    };
    if mailbox.thread != std::thread::current().id() {
        // (`nil`: no value is made off the interface's thread.)
        // SAFETY: `out` takes it.
        unsafe { out.cast::<Value>().write(Value::Nil) };
        return abi::FAILED;
    }
    if !name
        .split_once('.')
        .is_some_and(|(module, rest)| !module.is_empty() && !rest.is_empty())
    {
        return failed(format!(
            "an image a module sets is named `<module>.<name>`, not `{name}`"
        ));
    }
    let wanted = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4));
    if width == 0 || height == 0 {
        return failed(format!(
            "`{name}` is {width}×{height}: an image is at least 1×1"
        ));
    }
    if wanted != Some(pixels.len()) {
        return failed(format!(
            "`{name}` is {width}×{height}, so {} bytes of RGBA8 (a row {} bytes), not {}",
            wanted.map_or("too many".into(), |n| n.to_string()),
            width as usize * 4,
            pixels.len()
        ));
    }
    // SAFETY: On the handle's thread, as checked.
    let Some(host) = (unsafe { host_of(host) }) else {
        return failed(format!("`{name}` wasn't set: {HOST_GONE}"));
    };
    let image = match zgx::image::ImageData::new(width, height, pixels) {
        Ok(image) => image,
        Err(why) => return failed(format!("`{name}`: {why}")),
    };
    IMAGES.with_borrow_mut(|all| {
        all.retain(|(host, _)| host.strong_count() > 0);
        match all
            .iter_mut()
            .find(|(of, _)| std::ptr::eq(of.as_ptr(), Rc::as_ptr(&host.0)))
        {
            Some((_, names)) => drop(names.insert(name.clone())),
            None => all.push((Rc::downgrade(&host.0), [name.clone()].into())),
        }
    });
    host.image(&name, Some(Arc::new(image)));
    abi::DONE
}
unsafe extern "C" fn image_remove(host: *mut c_void, name: *const u8, len: usize) -> u32 {
    // SAFETY: A host handle, held; the module's `&str`.
    let (Some(host), name) = (unsafe { (host_of(host), name_of(name, len)) }) else {
        return 0;
    };
    let was = IMAGES.with_borrow_mut(|all| {
        all.iter_mut()
            .find(|(of, _)| std::ptr::eq(of.as_ptr(), Rc::as_ptr(&host.0)))
            .is_some_and(|(_, names)| names.remove(&name))
    });
    if was {
        host.image(&name, None);
    }
    u32::from(was)
}

/// A module's promise, as a script holds it (`ui.call`): settled by the
/// module (`Api::settle`) or rejected as it is let go of unsettled.
#[derive(Default)]
struct Settled {
    result: RefCell<Option<Result<Value, String>>>,
    waker: RefCell<Option<Waker>>,
}
/// The promises a thread's modules hold, by number: each one's wait, and
/// what made it.
type Promises = std::collections::HashMap<u64, (Weak<Settled>, Rc<Registered>)>;
thread_local! {
    /// The promises this thread's modules hold: made as an async action is
    /// first called.
    static PROMISES: RefCell<Promises> = RefCell::default();
    static NEXT_PROMISE: Cell<u64> = const { Cell::new(0) };
}
/// What a module holds for a promise: its number, and its host's mailbox
/// (how it is rejected from any thread, let go of).
struct PromiseHandle {
    id: u64,
    mailbox: Arc<Mailbox>,
}
/// The call of a module's async action: its promise made, the module
/// called, and the result awaited as the module settles it.
fn async_call(
    call: abi::AsyncAction,
    registered: &Rc<Registered>,
    host: &Weak<crate::Inner>,
    arg: Value,
) -> impl std::future::Future<Output = Result<Value, String>> + 'static + use<> {
    let settled = Rc::new(Settled::default());
    let started = host.upgrade().map(|host| {
        let id = NEXT_PROMISE.replace(NEXT_PROMISE.get() + 1);
        PROMISES.with_borrow_mut(|all| {
            all.insert(id, (Rc::downgrade(&settled), registered.clone()));
        });
        let promise = Box::into_raw(Box::new(PromiseHandle {
            id,
            mailbox: mailbox_of(&ScriptHost(host)),
        }));
        // SAFETY: The module's action and its own `user`, alive with
        // `registered`; `arg` lent for the call; the promise the module's.
        unsafe {
            call(
                registered.user.ptr,
                std::ptr::from_ref(&arg).cast(),
                promise.cast(),
            )
        };
        id
    });
    /// Lets go of its entry as the caller's wait goes.
    struct Forget(Option<u64>);
    impl Drop for Forget {
        fn drop(&mut self) {
            if let Some(id) = self.0 {
                let _ = PROMISES.try_with(|all| all.borrow_mut().remove(&id));
            }
        }
    }
    let forget = Forget(started);
    async move {
        let _forget = forget;
        std::future::poll_fn(|cx| match settled.result.borrow_mut().take() {
            Some(result) => Poll::Ready(result),
            None => {
                *settled.waker.borrow_mut() = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await
    }
}
/// Settles promise `id` on this thread, if its caller still waits.
fn settled(id: u64, result: impl FnOnce(&Registered) -> Result<Value, String>) {
    let Some((settled, registered)) = PROMISES.with_borrow_mut(|all| all.remove(&id)) else {
        return;
    };
    let Some(settled) = settled.upgrade() else {
        return;
    };
    *settled.result.borrow_mut() = Some(result(&registered));
    if let Some(waker) = settled.waker.borrow_mut().take() {
        waker.wake();
    }
}
unsafe extern "C" fn settle(promise: *mut c_void, ok: u32, value: *mut Raw) {
    // SAFETY: A promise the module settles once; its owned value, taken.
    let (promise, value) = unsafe { (Box::from_raw(promise as *mut PromiseHandle), take(value)) };
    if promise.mailbox.thread != std::thread::current().id() {
        // (Values are the interface thread's: one here would be another's.)
        eprintln!("zgx: a module settled a promise off the interface's thread");
        std::process::abort();
    }
    settled(promise.id, |registered| match ok {
        0 => Err(registered.error(value)),
        _ => Ok(value),
    });
}
unsafe extern "C" fn promise_host(promise: *const c_void) -> *mut c_void {
    // SAFETY: A promise, held (any thread: the count is atomic).
    let promise = unsafe { &*(promise as *const PromiseHandle) };
    Arc::into_raw(promise.mailbox.clone()) as *mut c_void
}
unsafe extern "C" fn promise_release(promise: *mut c_void) {
    unsafe extern "C" fn rejected(user: *mut c_void, _host: *mut c_void) {
        // SAFETY: The number posted below, freed as the job runs.
        let id = *unsafe { Box::from_raw(user as *mut u64) };
        settled(id, |registered| {
            Err(format!(
                "{} (module `{}`): the module dropped its promise unsettled",
                registered.name, registered.module
            ))
        });
    }
    unsafe extern "C" fn free(user: *mut c_void) {
        // SAFETY: The box posted below, freed once.
        drop(unsafe { Box::from_raw(user as *mut u64) });
    }
    // SAFETY: A promise let go of once, from any thread.
    let promise = unsafe { Box::from_raw(promise as *mut PromiseHandle) };
    let id = Box::into_raw(Box::new(promise.id));
    // (Rejected on the host's thread, where promises are: from here a post.
    // A host gone has no caller left to tell.)
    // SAFETY: A host handle, held by the promise; a job of this file's.
    unsafe {
        post(
            Arc::as_ptr(&promise.mailbox) as *mut c_void,
            rejected,
            id.cast(),
            free,
        )
    };
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
            Call::Async => unreachable!("an async action's member calls `ui.call`"),
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
                Call::Async => unreachable!("an async action's member calls `ui.call`"),
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
    // (`ui.call` read as a table with an async action is made, not here:
    // reading it makes the names made with it.)
    let calls = ui.clone();
    ui.set(
        "module",
        lua.create_function(move |lua, name: String| {
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
                if let Call::Async = member.call {
                    // (As `ui.call(name, arg)`: a promise, the app's grants.)
                    let call: mlua::Function = calls.get("call")?;
                    let name = member.registered.name.clone();
                    let function = lua.create_function(move |_, arg: Option<Value>| {
                        call.call::<mlua::Value>((&*name, arg))
                    })?;
                    table.raw_set(&*member.member, function)?;
                    continue;
                }
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
        // Loaded into this host again: what it set before goes first.
        remove_images_of(self, &said.name);
        let loading = Loading {
            pending: RefCell::default(),
            highlighters: RefCell::default(),
            host: self.clone(),
        };
        let registrar = Registrar {
            size: size_of::<Registrar>() as u32,
            host: std::ptr::from_ref(&loading) as *mut c_void,
            function: register_function,
            function_taking: register_function_taking,
            action: register_action,
            state,
            host_ref,
            async_action: register_async_action,
            highlighter: register_highlighter,
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
            abi::SETUP_PANICKED => {
                return Err(
                    "its setup panicked as it loaded (the panic's message was printed)".into(),
                );
            }
            code => return Err(format!("the module refused to load ({code})")),
        }
        let pending = loading.pending.into_inner();
        if let Some(registers) = &said.registers {
            // (Refused, what it registered is freed here, unused.)
            held_to(registers, &pending)?;
        }
        for (name, native) in loading.highlighters.into_inner() {
            crate::native_highlight::register(
                &self.0.functions,
                &self.0.highlighters,
                name,
                native,
            );
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
                let listed_async = said
                    .registers
                    .iter()
                    .flat_map(|said| &said.asyncs)
                    .find(|(function, _)| *function == name);
                Member {
                    member: listed
                        .map(|(_, m, _)| m.clone())
                        .or_else(|| listed_async.map(|(_, m)| m.clone()))
                        .unwrap_or_else(|| member_of(&name).to_owned()),
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
                Kind::Async(call) => {
                    members.push(member(Call::Async));
                    let host = Rc::downgrade(&self.0);
                    self.async_action(name, move |arg| async_call(call, &registered, &host, arg));
                }
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

#[cfg(test)]
mod registrar_tests {
    //! The registrar's last entry, its highlighter, between modules and
    //! hosts built at either side of it.
    use super::*;
    use std::cell::Cell;
    use std::mem::offset_of;

    thread_local! {
        static HIGHLIGHTER_TAKEN: Cell<Option<bool>> = const { Cell::new(None) };
    }
    mod either {
        use super::HIGHLIGHTER_TAKEN;
        use zgx_module::{Host, Value};
        zgx_module::register!("either", |host: &Host| {
            host.function("either.one", |_| Value::from(1.0));
            let taken = host.highlighter("either.lines", &["word"], |_, state, _| state);
            HIGHLIGHTER_TAKEN.with(|t| t.set(Some(taken)));
        });
    }

    /// A host's registrar as `enter` makes it, `size` bytes of it said.
    fn registrar(loading: &Loading, size: usize) -> Registrar {
        Registrar {
            size: size as u32,
            host: std::ptr::from_ref(loading) as *mut c_void,
            function: register_function,
            function_taking: register_function_taking,
            action: register_action,
            state,
            host_ref,
            async_action: register_async_action,
            highlighter: register_highlighter,
        }
    }
    fn host_loading() -> (zgx::reactive::Runtime, Loading) {
        let runtime = zgx::reactive::Runtime::new();
        let host = ScriptHost::inline(&runtime, "app", "return ui.text { \"x\" }");
        (
            runtime,
            Loading {
                pending: RefCell::default(),
                highlighters: RefCell::default(),
                host,
            },
        )
    }

    /// A module built with highlighters, on a host from before them: it
    /// loads, its functions with it, and is told it has no highlighter.
    #[test]
    fn a_new_module_on_an_old_host_loads_without_its_highlighter() {
        let (_runtime, loading) = host_loading();
        let old = registrar(&loading, offset_of!(Registrar, highlighter));
        HIGHLIGHTER_TAKEN.with(|t| t.set(None));
        // SAFETY: The module's entry, with tables alive through the call.
        assert_eq!(unsafe { either::zgx_module_v1(&API, &old) }, 0);
        assert_eq!(HIGHLIGHTER_TAKEN.with(Cell::get), Some(false));
        assert_eq!(loading.pending.borrow().len(), 1, "its function");
        assert!(loading.highlighters.borrow().is_empty());
        // On a host that has them, the same module takes one.
        let (_runtime, loading) = host_loading();
        let new = registrar(&loading, size_of::<Registrar>());
        // SAFETY: As above.
        assert_eq!(unsafe { either::zgx_module_v1(&API, &new) }, 0);
        assert_eq!(HIGHLIGHTER_TAKEN.with(Cell::get), Some(true));
        assert_eq!(loading.highlighters.borrow().len(), 1);
        // A registrar shorter than the interface before highlighters is
        // another interface's.
        let older = registrar(&loading, offset_of!(Registrar, async_action));
        // SAFETY: As above.
        assert_eq!(
            unsafe { either::zgx_module_v1(&API, &older) },
            abi::REFUSED_ABI
        );
    }

    /// The registrar as a module built before highlighters reads it.
    #[repr(C)]
    struct OldRegistrar {
        size: u32,
        host: *mut c_void,
        function: unsafe extern "C" fn(
            *mut c_void,
            *const u8,
            usize,
            abi::Function,
            *mut c_void,
            abi::Free,
        ),
        function_taking: *const c_void,
        action: *const c_void,
        state: *const c_void,
        host_ref: *const c_void,
        async_action: *const c_void,
    }
    unsafe extern "C" fn old_entry(api: *const Api, registrar: *const Registrar) -> u32 {
        // (Registered, never called here.)
        unsafe extern "C" fn one(_: *mut c_void, _: *const Raw, _: usize, _: *mut Raw) -> u32 {
            abi::FAILED
        }
        unsafe extern "C" fn free(_: *mut c_void) {}
        // SAFETY: The runtime's tables, read as the module was built to.
        unsafe {
            let old = &*(registrar as *const OldRegistrar);
            if (*api).abi != abi::ABI || (old.size as usize) < size_of::<OldRegistrar>() {
                return abi::REFUSED_ABI;
            }
            (old.function)(
                old.host,
                b"old.one".as_ptr(),
                7,
                one,
                std::ptr::null_mut(),
                free,
            );
        }
        0
    }

    /// A module built before highlighters, on a host that has them: the
    /// registrar is longer than it reads, and it loads as it did.
    #[test]
    fn an_old_module_on_a_new_host_loads_as_it_did() {
        assert_eq!(
            size_of::<OldRegistrar>(),
            offset_of!(Registrar, highlighter),
            "the old layout"
        );
        let (_runtime, loading) = host_loading();
        let new = registrar(&loading, size_of::<Registrar>());
        // SAFETY: The test's entry, with tables alive through the call.
        assert_eq!(unsafe { old_entry(&API, &new) }, 0);
        assert_eq!(loading.pending.borrow().len(), 1);
        assert!(loading.highlighters.borrow().is_empty());
    }
}
