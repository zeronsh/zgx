//! Native modules: an app's own Rust, built apart from zgx as a library
//! (`crate-type = ["cdylib"]`) the runtime loads, its functions called from
//! Luau as a host's are (`ui.call_sync`, `ui.action`).
//!
//! ```ignore
//! use zgx_module::{Host, Value};
//!
//! zgx_module::module!(|host: &Host| {
//!     host.function("words.count", |text| {
//!         let n = text.str().map_or(0, |s| s.split_whitespace().count());
//!         Value::from(n as f64)
//!     });
//! });
//! ```
//!
//! A module links this crate and nothing of zgx: what crosses is [`abi`], C
//! functions and a value's layout, so a module built once runs in any
//! runtime that speaks its [`abi::ABI`] (docs/design/runtime.md). Values
//! are the runtime's own, read where it holds them: a string's text, a
//! list's items and a map's fields cost a module what they cost code linked
//! into the app, and nothing is copied.
//!
//! A panic in a module ends the process, as one in a host's own function
//! does today: nothing unwinds across the boundary.

#[cfg(not(target_pointer_width = "64"))]
compile_error!("zgx-module reads values by their 64-bit layout");

pub mod abi;

use std::{
    cell::RefCell,
    ffi::c_void,
    marker::PhantomData,
    mem::MaybeUninit,
    sync::atomic::{AtomicPtr, Ordering},
};

use abi::{Api, Bytes, Raw};

/// The runtime's value functions, set as the module loads.
static API: AtomicPtr<Api> = AtomicPtr::new(std::ptr::null_mut());

#[inline]
fn api() -> &'static Api {
    let api = API.load(Ordering::Relaxed);
    assert!(
        !api.is_null(),
        "zgx-module: a value made before the module loaded"
    );
    // SAFETY: Set by `enter` to the runtime's table, which lives as long
    // as the process.
    unsafe { &*api }
}

const fn raw_of(tag: u8) -> Raw {
    let mut bytes = [MaybeUninit::new(0u8); abi::VALUE_SIZE];
    bytes[0] = MaybeUninit::new(tag);
    Raw(bytes)
}

/// The kind of the value at `at`.
///
/// # Safety
/// `at` points at a value.
#[inline(always)]
unsafe fn tag(at: *const Raw) -> u8 {
    // SAFETY: A value's first byte says which kind it is.
    unsafe { at.cast::<u8>().read() }
}
/// The word `offset` bytes into what `at` points at.
///
/// # Safety
/// A word of the runtime's is there.
#[inline(always)]
unsafe fn word<T>(at: *const T, offset: usize) -> usize {
    // SAFETY: The caller's.
    unsafe { at.cast::<u8>().add(offset).cast::<usize>().read() }
}
/// Where a list's items or a map's fields begin.
///
/// # Safety
/// `at` points at a list or a map.
#[inline(always)]
unsafe fn data(at: *const Raw) -> *const u8 {
    // SAFETY: Such a value holds a pointer to its contents.
    unsafe { word(at, abi::HELD_AT) as *const u8 }
}
/// # Safety
/// `ptr` and `len` are a string of the runtime's, alive for `'a`.
#[inline(always)]
unsafe fn text<'a>(ptr: *const u8, len: usize) -> &'a str {
    // SAFETY: The runtime's strings are UTF-8.
    unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) }
}

/// Whether `tag` is a kind a module counts itself: a string, a list or a
/// map, whose count is at a stated place.
#[inline(always)]
fn counted(tag: u8) -> bool {
    matches!(tag, abi::STR | abi::LIST | abi::MAP)
}

/// A value a module owns: what a function returns, and what it keeps. The
/// runtime's own, so cloning one counts it and copies nothing.
#[repr(transparent)]
pub struct Value {
    raw: Raw,
    /// The runtime's values belong to the interface's thread.
    _thread: PhantomData<*const ()>,
}

impl Value {
    /// `nil`.
    pub const NIL: Value = Value {
        raw: raw_of(abi::NIL),
        _thread: PhantomData,
    };

    /// A list of `items`.
    pub fn list(items: impl IntoIterator<Item = Value>) -> Value {
        // (`Value` is its three words and a marker: a `Vec` of them is the
        // items the runtime takes.)
        let mut items: Vec<Value> = items.into_iter().collect();
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: `items` are owned values, given to the runtime; it writes
        // `out`.
        unsafe {
            (api().list)(items.as_mut_ptr().cast(), items.len(), out.as_mut_ptr());
            items.set_len(0);
            Value::owned(out.assume_init())
        }
    }
    /// A map of `fields`, in any order (the last of a key given twice
    /// kept).
    pub fn record<K: AsRef<str>>(fields: impl IntoIterator<Item = (K, Value)>) -> Value {
        let (held, mut values): (Vec<K>, Vec<Value>) = fields.into_iter().unzip();
        let keys: Vec<Bytes> = held
            .iter()
            .map(|k| Bytes {
                ptr: k.as_ref().as_ptr(),
                len: k.as_ref().len(),
            })
            .collect();
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: As many keys as owned values, the keys' bytes alive in
        // `held`; the runtime takes the values and writes `out`.
        unsafe {
            (api().map)(
                keys.as_ptr(),
                values.as_mut_ptr().cast(),
                values.len(),
                out.as_mut_ptr(),
            );
            values.set_len(0);
            Value::owned(out.assume_init())
        }
    }
    /// It, to read.
    #[inline(always)]
    pub fn get(&self) -> Ref<'_> {
        Ref {
            at: &self.raw,
            _of: PhantomData,
        }
    }
    #[inline(always)]
    fn owned(raw: Raw) -> Value {
        Value {
            raw,
            _thread: PhantomData,
        }
    }
}
// A list the runtime takes is a `Vec<Value>`'s buffer.
const _: () = assert!(size_of::<Value>() == abi::VALUE_SIZE && align_of::<Value>() == 8);

impl Default for Value {
    fn default() -> Self {
        Value::NIL
    }
}
impl Drop for Value {
    #[inline]
    fn drop(&mut self) {
        let at: *mut Raw = &mut self.raw;
        // SAFETY: An owned value. A string, a list or a map held elsewhere
        // too is one hold fewer, as its own drop is; the last hold, and any
        // kind held another way, the runtime frees.
        unsafe {
            let tag = tag(at);
            if tag <= abi::NUM {
                return;
            }
            if counted(tag) {
                let count = (word(at, abi::HELD_AT) as *mut u8)
                    .sub(abi::COUNT_BEFORE)
                    .cast::<usize>();
                if *count > 1 {
                    *count -= 1;
                    return;
                }
            }
            (api().release)(at);
        }
    }
}
impl Clone for Value {
    #[inline]
    fn clone(&self) -> Self {
        self.get().to_owned()
    }
}
impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}
impl From<bool> for Value {
    #[inline]
    fn from(b: bool) -> Self {
        let mut raw = raw_of(abi::BOOL);
        raw.0[abi::BOOL_AT] = MaybeUninit::new(u8::from(b));
        Value::owned(raw)
    }
}
impl From<f64> for Value {
    #[inline]
    fn from(n: f64) -> Self {
        let mut raw = raw_of(abi::NUM);
        // SAFETY: A number's `f64` is at `HELD_AT`, inside the value.
        unsafe {
            std::ptr::from_mut(&mut raw)
                .cast::<u8>()
                .add(abi::HELD_AT)
                .cast::<f64>()
                .write(n)
        };
        Value::owned(raw)
    }
}
impl From<&str> for Value {
    #[inline]
    fn from(s: &str) -> Self {
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: A string's bytes, read before this returns; the runtime
        // writes `out`.
        unsafe {
            (api().string)(s.as_ptr(), s.len(), out.as_mut_ptr());
            Value::owned(out.assume_init())
        }
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::from(s.as_str())
    }
}
impl<'a> From<Ref<'a>> for Value {
    fn from(value: Ref<'a>) -> Self {
        value.to_owned()
    }
}

/// A value lent: a function's argument, an owned one being read, a list's
/// item or a map's field, where the runtime holds it. Reading through it
/// counts and copies nothing, and calls nothing.
#[derive(Clone, Copy)]
pub struct Ref<'a> {
    at: *const Raw,
    _of: PhantomData<&'a Value>,
}

impl<'a> Ref<'a> {
    /// What a script would read of a row kept folded (the map it opens
    /// to); else itself.
    #[inline(always)]
    fn opened(self) -> *const Raw {
        // SAFETY: `at` points at a value.
        if unsafe { tag(self.at) } != abi::FOLDED {
            return self.at;
        }
        self.unfolded()
    }
    #[cold]
    #[inline(never)]
    fn unfolded(self) -> *const Raw {
        // SAFETY: A lent value.
        let opened = unsafe { (api().open)(self.at) };
        if opened.is_null() { self.at } else { opened }
    }
    #[inline(always)]
    fn tag(self) -> u8 {
        // SAFETY: `at` points at a value.
        unsafe { tag(self.at) }
    }
    pub fn is_nil(self) -> bool {
        self.tag() == abi::NIL
    }
    /// Whether Luau counts it as true: anything but `nil` and `false`.
    pub fn truthy(self) -> bool {
        self.tag() != abi::NIL && self.bool() != Some(false)
    }
    #[inline]
    pub fn bool(self) -> Option<bool> {
        // SAFETY: A boolean's byte is at `BOOL_AT`.
        (self.tag() == abi::BOOL)
            .then(|| unsafe { self.at.cast::<u8>().add(abi::BOOL_AT).read() != 0 })
    }
    #[inline]
    pub fn num(self) -> Option<f64> {
        // SAFETY: A number's `f64` is at `HELD_AT`.
        (self.tag() == abi::NUM)
            .then(|| unsafe { self.at.cast::<u8>().add(abi::HELD_AT).cast::<f64>().read() })
    }
    /// Its text if it is a string, read where the runtime holds it.
    #[inline]
    pub fn str(self) -> Option<&'a str> {
        if self.tag() != abi::STR {
            return None;
        }
        // SAFETY: A string holds a pointer to its UTF-8 and its length,
        // alive as long as the value this was lent from.
        Some(unsafe {
            text(
                word(self.at, abi::HELD_AT) as *const u8,
                word(self.at, abi::HELD_AT + 8),
            )
        })
    }
    /// Its text if it is a string of any length. The runtime holds one past
    /// 64 KiB in pieces, which [`Ref::str`] doesn't read: this reads it
    /// too, as one string made for the reading.
    #[inline]
    pub fn text(self) -> Option<Text<'a>> {
        match self.tag() {
            abi::STR => self.str().map(Text::Lent),
            abi::TEXT => {
                let mut out = MaybeUninit::<Raw>::uninit();
                // SAFETY: A lent value; the runtime writes `out`.
                let flat = unsafe {
                    (api().flat)(self.at, out.as_mut_ptr());
                    Value::owned(out.assume_init())
                };
                flat.get().str().is_some().then_some(Text::Held(flat))
            }
            _ => None,
        }
    }
    /// Whether it is a list.
    #[inline]
    pub fn is_list(self) -> bool {
        // SAFETY: A value.
        unsafe { tag(self.opened()) == abi::LIST }
    }
    /// Whether it is a map.
    #[inline]
    pub fn is_map(self) -> bool {
        // SAFETY: A value.
        unsafe { tag(self.opened()) == abi::MAP }
    }
    /// A list's items or a map's fields, where they are, and how many;
    /// nowhere and none if it isn't a `kind`.
    #[inline(always)]
    fn contents(self, kind: u8) -> (*const u8, usize) {
        let at = self.opened();
        // SAFETY: A value; a list or a map holds its contents and a count.
        unsafe {
            if tag(at) == kind {
                (data(at), word(at, abi::HELD_AT + 8))
            } else {
                (std::ptr::null(), 0)
            }
        }
    }
    /// How many items a list has, or fields a map; 0 for anything else.
    #[inline]
    pub fn len(self) -> usize {
        let at = self.opened();
        // SAFETY: A value; a list or a map holds a count.
        unsafe {
            if matches!(tag(at), abi::LIST | abi::MAP) {
                word(at, abi::HELD_AT + 8)
            } else {
                0
            }
        }
    }
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
    /// A list's item, from 0.
    #[inline]
    pub fn item(self, index: usize) -> Option<Ref<'a>> {
        let (items, len) = self.contents(abi::LIST);
        // SAFETY: Within the list, whose items are values.
        (index < len).then(|| Ref {
            at: unsafe { items.cast::<Raw>().add(index) },
            _of: PhantomData,
        })
    }
    /// A list's items, none if it isn't one.
    #[inline]
    pub fn items(self) -> impl ExactSizeIterator<Item = Ref<'a>> + DoubleEndedIterator + Clone {
        let (items, len) = self.contents(abi::LIST);
        // SAFETY: Within the list, whose items are values.
        (0..len).map(move |i| Ref {
            at: unsafe { items.cast::<Raw>().add(i) },
            _of: PhantomData,
        })
    }
    /// A list of this list's items `from` to `to` (from 0, `to` excluded):
    /// the items counted, not copied. `nil` if they aren't in it.
    pub fn slice(self, from: usize, to: usize) -> Value {
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: A lent value; the runtime writes `out`.
        unsafe {
            (api().slice)(self.opened(), from, to, out.as_mut_ptr());
            Value::owned(out.assume_init())
        }
    }
    /// The map's field at `index` (in key order): its key and its value.
    ///
    /// # Safety
    /// `fields` is a map's, of more than `index` fields.
    #[inline(always)]
    unsafe fn entry(fields: *const u8, index: usize) -> (&'a str, Ref<'a>) {
        // SAFETY: A field is a key (a pointer to its UTF-8, and its length)
        // and a value, `FIELD_SIZE` apart.
        unsafe {
            let field = fields.add(index * abi::FIELD_SIZE);
            let key = field.add(abi::FIELD_KEY_AT);
            let key = text(word(key, 0) as *const u8, word(key, 8));
            (
                key,
                Ref {
                    at: field.add(abi::FIELD_VALUE_AT).cast(),
                    _of: PhantomData,
                },
            )
        }
    }
    /// A map's field: found as the runtime finds it, with no call.
    #[inline]
    pub fn field(self, key: &str) -> Option<Ref<'a>> {
        let (fields, len) = self.contents(abi::MAP);
        // SAFETY: A map's fields, each index under `len`.
        unsafe {
            // (As the runtime's own lookup: a short map in order, a long one
            // by halves; its fields are sorted by key.)
            if len <= 8 {
                for i in 0..len {
                    let (k, value) = Self::entry(fields, i);
                    if key_eq(k, key) {
                        return Some(value);
                    }
                }
                return None;
            }
            let (mut low, mut high) = (0, len);
            while low < high {
                let mid = low + (high - low) / 2;
                let (k, value) = Self::entry(fields, mid);
                match k.cmp(key) {
                    std::cmp::Ordering::Equal => return Some(value),
                    std::cmp::Ordering::Less => low = mid + 1,
                    std::cmp::Ordering::Greater => high = mid,
                }
            }
            None
        }
    }
    /// The fields of a map named by `keys`, each at its key's place: `let
    /// [id, text] = row.get(["id", "text"])`. None for a field it doesn't
    /// have, all none if it isn't a map.
    #[inline]
    pub fn get<const N: usize>(self, keys: [&str; N]) -> [Option<Ref<'a>>; N] {
        keys.map(|key| self.field(key))
    }
    /// A map's fields in key order, none if it isn't one.
    #[inline]
    pub fn fields(self) -> impl ExactSizeIterator<Item = (&'a str, Ref<'a>)> + Clone {
        let (fields, len) = self.contents(abi::MAP);
        // SAFETY: A map's fields, each index under `len`.
        (0..len).map(move |i| unsafe { Self::entry(fields, i) })
    }
    /// It, owned: counted, not copied.
    #[inline]
    pub fn to_owned(self) -> Value {
        // SAFETY: A lent value. A string, a list or a map is held once more,
        // as its own clone holds it; a number, a boolean and `nil` are their
        // three words; any other kind the runtime clones.
        unsafe {
            let tag = tag(self.at);
            if counted(tag) {
                let count = (word(self.at, abi::HELD_AT) as *mut u8)
                    .sub(abi::COUNT_BEFORE)
                    .cast::<usize>();
                if *count < usize::MAX / 2 {
                    *count += 1;
                    return Value::owned(self.at.read());
                }
            } else if tag <= abi::NUM {
                return Value::owned(self.at.read());
            }
            let mut out = MaybeUninit::<Raw>::uninit();
            (api().clone)(self.at, out.as_mut_ptr());
            Value::owned(out.assume_init())
        }
    }
}
impl std::fmt::Debug for Ref<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_nil() {
            f.write_str("nil")
        } else if let Some(b) = self.bool() {
            write!(f, "{b}")
        } else if let Some(n) = self.num() {
            write!(f, "{n}")
        } else if let Some(s) = self.str() {
            write!(f, "{s:?}")
        } else if self.is_list() {
            f.debug_list().entries(self.items()).finish()
        } else if self.is_map() {
            f.debug_map().entries(self.fields()).finish()
        } else {
            f.write_str("<value>")
        }
    }
}

/// A string's text ([`Ref::text`]): where the runtime holds it, or, of one
/// it holds in pieces, a string made of them.
pub enum Text<'a> {
    Lent(&'a str),
    Held(Value),
}
impl std::ops::Deref for Text<'_> {
    type Target = str;
    fn deref(&self) -> &str {
        match self {
            Text::Lent(text) => text,
            Text::Held(value) => value.get().str().unwrap_or(""),
        }
    }
}

/// Whether two keys are the same text: the runtime's own comparison
/// (`zgx-script`'s `key_eq`), a few loads for keys up to 24 bytes and no
/// call, so a field found here costs what one found there does.
#[inline(always)]
fn key_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let n = a.len();
    if n != b.len() {
        return false;
    }
    let word = |s: &[u8], at: usize| u64::from_ne_bytes(s[at..at + 8].try_into().unwrap());
    let half = |s: &[u8], at: usize| u32::from_ne_bytes(s[at..at + 4].try_into().unwrap());
    match n {
        0 => true,
        1..=3 => a[0] == b[0] && a[n / 2] == b[n / 2] && a[n - 1] == b[n - 1],
        4..=7 => half(a, 0) == half(b, 0) && half(a, n - 4) == half(b, n - 4),
        8..=16 => word(a, 0) == word(b, 0) && word(a, n - 8) == word(b, n - 8),
        17..=24 => {
            word(a, 0) == word(b, 0) && word(a, 8) == word(b, 8) && word(a, n - 8) == word(b, n - 8)
        }
        _ => a == b,
    }
}

/// A module's own state ([`Host::state`]): one per thread, the same for
/// every host on it, kept by the runtime and handed to the module as it
/// loads. A function keeps the handle and reads the state through it with
/// no lookup, where a `thread_local!` in a library is a call each time.
pub struct State<T: 'static> {
    at: *const RefCell<T>,
}
impl<T> Clone for State<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for State<T> {}
impl<T> State<T> {
    /// Runs `read` with the state. As a `RefCell`: `read` using this state
    /// again panics.
    #[inline]
    pub fn with<R>(self, read: impl FnOnce(&mut T) -> R) -> R {
        // SAFETY: The runtime keeps it for as long as the thread runs, and
        // a handle never leaves its thread (it holds a raw pointer).
        read(&mut unsafe { &*self.at }.borrow_mut())
    }
}

/// The host a module is loading into: where it registers what scripts
/// call. Lent to the module's setup, not kept.
pub struct Host<'a> {
    registrar: &'a abi::Registrar,
}

impl Host<'_> {
    /// This module's state of type `T` for this thread: made (`T::default`)
    /// the first time a host on the thread asks, the same for each after,
    /// so every host a module loads into on a thread shares it, and another
    /// thread's hosts have their own.
    pub fn state<T: Default + 'static>(&self) -> State<T> {
        unsafe extern "C" fn init<T: Default>(state: *mut u8) {
            // SAFETY: Room for a `RefCell<T>`, unwritten.
            unsafe { state.cast::<RefCell<T>>().write(RefCell::default()) }
        }
        unsafe extern "C" fn free<T>(state: *mut u8) {
            // SAFETY: What `init` wrote, dropped once.
            unsafe { state.cast::<RefCell<T>>().drop_in_place() }
        }
        // (By this library's own address too: two modules may name a type
        // alike.)
        let name = format!("{:p} {}", &raw const API, std::any::type_name::<T>());
        let r = self.registrar;
        // SAFETY: The registrar's own host, inside the entry call.
        let at = unsafe {
            (r.state)(
                r.host,
                name.as_ptr(),
                name.len(),
                size_of::<RefCell<T>>(),
                align_of::<RefCell<T>>(),
                init::<T>,
                free::<T>,
            )
        };
        State { at: at.cast() }
    }
    /// Registers a function scripts call for its value now, with
    /// `ui.call_sync(name, arg)` (`ScriptHost::function`): on the
    /// interface's thread, so quick, with no I/O. Its argument is lent for
    /// the call (`to_owned` keeps it).
    pub fn function(&self, name: &str, function: impl Fn(Ref<'_>) -> Value + 'static) {
        unsafe extern "C" fn call<F: Fn(Ref<'_>) -> Value>(
            user: *mut c_void,
            arg: *const Raw,
            out: *mut Raw,
        ) {
            // SAFETY: `user` is the `F` boxed below, alive until `free`;
            // `arg` is lent for this call; `out` takes what it made.
            unsafe {
                let made = (*(user as *const F))(Ref {
                    at: arg,
                    _of: PhantomData,
                });
                out.write(made.raw);
                std::mem::forget(made);
            }
        }
        unsafe extern "C" fn free<F>(user: *mut c_void) {
            // SAFETY: The box made below, freed once.
            drop(unsafe { Box::from_raw(user as *mut F) });
        }
        fn register<F: Fn(Ref<'_>) -> Value + 'static>(r: &abi::Registrar, name: &str, f: F) {
            let user = Box::into_raw(Box::new(f)) as *mut c_void;
            // SAFETY: The registrar's own host, inside the entry call.
            unsafe {
                (r.function)(
                    r.host,
                    name.as_ptr(),
                    name.len(),
                    call::<F>,
                    user,
                    free::<F>,
                )
            }
        }
        register(self.registrar, name, function);
    }
    /// Registers a function handed its argument (`ui.call_sync(name,
    /// arg)`), for one that keeps or returns what it is given: no count of
    /// it is made. Any other is cheaper as a [`Host::function`], whose
    /// argument the runtime frees where it is.
    pub fn function_taking(&self, name: &str, function: impl Fn(Value) -> Value + 'static) {
        unsafe extern "C" fn call<F: Fn(Value) -> Value>(
            user: *mut c_void,
            arg: *mut Raw,
            out: *mut Raw,
        ) {
            // SAFETY: `user` is the `F` boxed below, alive until `free`;
            // `arg` is owned by this call; `out` takes what it made.
            unsafe {
                let made = (*(user as *const F))(Value::owned(arg.read()));
                out.write(made.raw);
                std::mem::forget(made);
            }
        }
        unsafe extern "C" fn free<F>(user: *mut c_void) {
            // SAFETY: The box made below, freed once.
            drop(unsafe { Box::from_raw(user as *mut F) });
        }
        fn register<F: Fn(Value) -> Value + 'static>(r: &abi::Registrar, name: &str, f: F) {
            let user = Box::into_raw(Box::new(f)) as *mut c_void;
            // SAFETY: The registrar's own host, inside the entry call.
            unsafe {
                (r.function_taking)(
                    r.host,
                    name.as_ptr(),
                    name.len(),
                    call::<F>,
                    user,
                    free::<F>,
                )
            }
        }
        register(self.registrar, name, function);
    }
    /// Registers an action scripts bind with `ui.action(name, arg)` or call
    /// with `ui.invoke` (`ScriptHost::action`).
    pub fn action(&self, name: &str, action: impl Fn(Ref<'_>) + 'static) {
        unsafe extern "C" fn call<F: Fn(Ref<'_>)>(user: *mut c_void, arg: *const Raw) {
            // SAFETY: As `function`'s.
            unsafe {
                (*(user as *const F))(Ref {
                    at: arg,
                    _of: PhantomData,
                })
            }
        }
        unsafe extern "C" fn free<F>(user: *mut c_void) {
            // SAFETY: The box made below, freed once.
            drop(unsafe { Box::from_raw(user as *mut F) });
        }
        fn register<F: Fn(Ref<'_>) + 'static>(r: &abi::Registrar, name: &str, f: F) {
            let user = Box::into_raw(Box::new(f)) as *mut c_void;
            // SAFETY: The registrar's own host, inside the entry call.
            unsafe {
                (r.action)(
                    r.host,
                    name.as_ptr(),
                    name.len(),
                    call::<F>,
                    user,
                    free::<F>,
                )
            }
        }
        register(self.registrar, name, action);
    }
}

/// What [`module!`]'s entry does: checks the runtime speaks this module's
/// interface, keeps its value functions, and runs `setup`.
///
/// # Safety
/// `api` and `registrar` are the runtime's, as [`abi::Entry`] is called
/// with: `api` alive for the process, `registrar` for this call.
#[doc(hidden)]
pub unsafe fn enter(
    api: *const Api,
    registrar: *const abi::Registrar,
    setup: impl FnOnce(&Host),
) -> u32 {
    // SAFETY: The runtime's tables; each begins with what says how much of
    // it there is.
    let (api_ref, registrar) = unsafe { (&*api, &*registrar) };
    if api_ref.abi != abi::ABI
        || (api_ref.size as usize) < size_of::<Api>()
        || (registrar.size as usize) < size_of::<abi::Registrar>()
    {
        return abi::REFUSED_ABI;
    }
    API.store(api.cast_mut(), Ordering::Relaxed);
    setup(&Host { registrar });
    0
}

/// A module's entry: `setup` is called with the [`Host`] each time the
/// module is loaded into one, to register its functions.
#[macro_export]
macro_rules! module {
    ($setup:expr) => {
        /// # Safety
        /// Called by the zgx runtime alone (`zgx_module::abi::Entry`).
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn zgx_module_v1(
            api: *const $crate::abi::Api,
            registrar: *const $crate::abi::Registrar,
        ) -> u32 {
            // SAFETY: As the runtime calls it.
            unsafe { $crate::enter(api, registrar, $setup) }
        }
    };
}
