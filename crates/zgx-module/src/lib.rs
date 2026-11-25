//! **Unstable before 1.0**: what a module is written with, and the
//! interface it is built to, may change in any release until then. A module
//! built against one `zgx-module` loads only in a runtime of the same
//! interface (`abi::ABI`), which says so if it isn't.
//!
//! Native modules: an app's own Rust, built apart from zgx as a library
//! (`crate-type = ["cdylib"]`) the runtime loads, its functions called from
//! Luau as a host's are (`ui.call_sync`, `ui.action`).
//!
//! ```ignore
//! use zgx_module::{Host, Value};
//!
//! zgx_module::register!(|host: &Host| {
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
//! Written with [`module`] and [`export`], a module's functions are plain
//! Rust functions of typed arguments, and scripts and editors know their
//! types (docs/modules.md):
//!
//! ```ignore
//! #[zgx_module::module(name = "words")]
//! mod words {
//!     #[zgx_module::export]
//!     fn count(text: &str) -> f64 {
//!         text.split_whitespace().count() as f64
//!     }
//! }
//! ```
//!
//! A panic in a module's function or action is caught where the runtime
//! called it, and raised in the script that called as an error the function
//! returned would be ("panicked: …"); the module stays loaded, and its state
//! is as the panic left it. One in its setup refuses the module; one in a
//! job, timer, observer or drop is caught and printed; one in a state's
//! `Default` ends the process. Nothing unwinds across the boundary. Return an error rather than panic: a panic
//! still prints, and leaves whatever it was doing half done.

#[cfg(not(target_pointer_width = "64"))]
compile_error!("zgx-module reads values by their 64-bit layout");

pub mod abi;
mod typed;

pub use typed::{FromValue, IntoValue, List, Map};
#[cfg(feature = "macros")]
pub use zgx_module_macros::{Record, export, module};

use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    marker::PhantomData,
    mem::MaybeUninit,
    sync::atomic::{AtomicPtr, Ordering},
    time::Duration,
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

/// A `nil` to lend where there is no value.
static NIL: Raw = raw_of(abi::NIL);

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
    /// A `nil`: what a map's missing field or an argument not passed reads
    /// as.
    pub const NIL: Ref<'static> = Ref {
        at: &raw const NIL,
        _of: PhantomData,
    };
    /// What kind of value it is, as an error says it: "a string".
    pub fn kind(self) -> &'static str {
        // SAFETY: A value.
        match unsafe { tag(self.opened()) } {
            abi::NIL => "nil",
            abi::BOOL => "a boolean",
            abi::NUM => "a number",
            abi::STR | abi::TEXT => "a string",
            abi::LIST => "a list",
            abi::MAP => "a map",
            _ => "a value only the runtime reads",
        }
    }
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
    /// Whether it is a string, of any length ([`Ref::text`] reads it).
    #[inline]
    pub fn is_string(self) -> bool {
        matches!(self.tag(), abi::STR | abi::TEXT)
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
/// call. Lent to the module's setup, and to each job posted to the host
/// ([`Ui::post`]) and each timer's call ([`Host::after`]): a `&Host` is
/// how a module knows it runs on the interface's thread, where values are
/// made and read (it is not `Send`). Not kept.
pub struct Host<'a> {
    /// The registrar, while the module loads: what scripts call is
    /// registered then, never later.
    registrar: Option<&'a abi::Registrar>,
    /// A handle to the host, asked for the first time a module needs one
    /// ([`Host::ui`], [`Host::after`]): owned by a loading host, lent to a
    /// job's or a timer's.
    handle: Cell<*mut c_void>,
    owned: bool,
}

impl Drop for Host<'_> {
    fn drop(&mut self) {
        let handle = self.handle.get();
        if self.owned && !handle.is_null() {
            // SAFETY: The handle this host asked for, let go of once.
            unsafe { (api().host_release)(handle) }
        }
    }
}

impl<'a> Host<'a> {
    /// The host of a module loading.
    fn loading(registrar: &'a abi::Registrar) -> Self {
        Self {
            registrar: Some(registrar),
            handle: Cell::new(std::ptr::null_mut()),
            owned: true,
        }
    }
}

impl Host<'static> {
    /// The host of a job or a timer's call, by the handle it is lent.
    fn lent(handle: *mut c_void) -> Self {
        Self {
            registrar: None,
            handle: Cell::new(handle),
            owned: false,
        }
    }
}

impl Host<'_> {
    /// The registrar: what scripts call is registered as the module loads.
    fn registrar(&self) -> &abi::Registrar {
        self.registrar
            .expect("zgx-module: what scripts call is registered as the module loads, not in a job or a timer")
    }
    /// The host's handle, asked for once.
    fn handle(&self) -> *mut c_void {
        if self.handle.get().is_null() {
            let r = self.registrar();
            // SAFETY: The registrar's own host, inside the entry call.
            self.handle.set(unsafe { (r.host_ref)(r.host) });
        }
        self.handle.get()
    }
    /// A handle to this host for the module's own threads: what they post
    /// work to the interface's thread by ([`Ui::post`]).
    pub fn ui(&self) -> Ui {
        // SAFETY: A host handle, held; the copy the `Ui` owns.
        Ui(unsafe { (api().host_retain)(self.handle()) })
    }
    /// Calls `fire` once, `delay` from now on zgx's clock (a test's,
    /// devtools' frozen one, video's virtual one, else the wall's), on the
    /// interface's thread, unless the [`Timer`] is dropped first.
    pub fn after<F: FnOnce(&Host) + 'static>(&self, delay: Duration, fire: F) -> Timer {
        unsafe extern "C" fn call<F: FnOnce(&Host)>(user: *mut c_void, host: *mut c_void) -> u32 {
            // SAFETY: `user` is the `Option<F>` boxed below, alive until
            // its timer ends; `host` lent for the call.
            if let Some(fire) = unsafe { &mut *(user as *mut Option<F>) }.take() {
                // (A panic is caught, printed, and the timer done.)
                __private::guard_quiet(|| fire(&Host::lent(host)));
            }
            0
        }
        let user = Box::into_raw(Box::new(Some(fire)));
        // SAFETY: A host handle, held; `user` and its `free` the box's.
        Timer(unsafe { self.timer(delay, false, call::<F>, user.cast(), free_box_of(user)) })
    }
    /// Calls `fire` every `period` on zgx's clock, on the interface's
    /// thread, until the [`Timer`] is dropped.
    pub fn every<F: FnMut(&Host) + 'static>(&self, period: Duration, fire: F) -> Timer {
        unsafe extern "C" fn call<F: FnMut(&Host)>(user: *mut c_void, host: *mut c_void) -> u32 {
            // SAFETY: `user` is the `F` boxed below, alive until its timer
            // ends; `host` lent for the call.
            let fire = unsafe { &mut *(user as *mut F) };
            // (A panic is caught and printed, and stops the timer: one that
            // panics once would again each period.)
            u32::from(__private::guard_quiet(|| fire(&Host::lent(host))))
        }
        let user = Box::into_raw(Box::new(fire));
        // SAFETY: As `after`'s.
        Timer(unsafe { self.timer(period, true, call::<F>, user.cast(), free_box_of(user)) })
    }
    /// The host's shared state `key`: the one scripts read with
    /// `ui.shared(key)` and the host with `ScriptHost::shared(key, …)`,
    /// made with `initial` if there is none yet. `None` once the host is
    /// gone.
    pub fn shared(&self, key: &str, initial: Value) -> Option<Shared> {
        let mut initial = std::mem::ManuallyDrop::new(initial);
        // SAFETY: A host handle, held; `initial` owned and given.
        let at = unsafe {
            (api().shared)(
                self.handle(),
                key.as_ptr(),
                key.len(),
                std::ptr::from_mut(&mut initial.raw),
            )
        };
        (!at.is_null()).then_some(Shared(at))
    }
    /// Shows `pixels` as the image `host:<name>` (`ui.image { src =
    /// "host:words.cover" }`, `bg_image`), replacing what was there: its
    /// bytes become the image's, not copied. `name` is `<module>.<name>`.
    /// An image a module sets goes with its host, or as the module is
    /// loaded into it again. An error saying why if `name` isn't so, or
    /// the host is gone.
    pub fn image(&self, name: &str, pixels: ImageBuffer) -> Result<(), Failed> {
        let pixels = std::mem::ManuallyDrop::new(pixels);
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: A host handle, held; the buffer given (taken either way);
        // `out` written only failing.
        unsafe {
            if (api().image_set)(
                self.handle(),
                name.as_ptr(),
                name.len(),
                pixels.width,
                pixels.height,
                pixels.handle,
                out.as_mut_ptr(),
            ) == abi::DONE
            {
                return Ok(());
            }
            let why = Value::owned(out.assume_init());
            Err(Failed(
                why.get().str().unwrap_or("couldn't be set").to_owned(),
            ))
        }
    }
    /// Removes the image `host:<name>` this module set: whether there was
    /// one.
    pub fn remove_image(&self, name: &str) -> bool {
        // SAFETY: A host handle, held.
        unsafe { (api().image_remove)(self.handle(), name.as_ptr(), name.len()) != 0 }
    }
    /// # Safety
    /// `fire`, `user` and `free` are a timer's, by [`abi::Fire`].
    unsafe fn timer(
        &self,
        period: Duration,
        repeat: bool,
        fire: abi::Fire,
        user: *mut c_void,
        free: abi::Free,
    ) -> *mut c_void {
        let ms = period.as_secs_f64() * 1e3;
        // SAFETY: The caller's; a host handle, held.
        unsafe { (api().after)(self.handle(), ms, u32::from(repeat), fire, user, free) }
    }
    /// This module's state of type `T` for this thread: made (`T::default`)
    /// the first time a host on the thread asks, the same for each after,
    /// so every host a module loads into on a thread shares it, and another
    /// thread's hosts have their own.
    pub fn state<T: Default + 'static>(&self) -> State<T> {
        unsafe extern "C" fn init<T: Default>(state: *mut u8) {
            // (A `T::default` that panics leaves nothing to keep: the one
            // panic that ends the process, said so.)
            let made = std::panic::catch_unwind(RefCell::<T>::default).unwrap_or_else(|_| {
                eprintln!(
                    "zgx-module: a module's state ({}) panicked as it was made: there is no state to go on with",
                    std::any::type_name::<T>()
                );
                std::process::abort()
            });
            // SAFETY: Room for a `RefCell<T>`, unwritten.
            unsafe { state.cast::<RefCell<T>>().write(made) }
        }
        unsafe extern "C" fn free<T>(state: *mut u8) {
            // SAFETY: What `init` wrote, dropped once (a panic in its drop
            // caught and printed).
            __private::guard_quiet(|| unsafe { state.cast::<RefCell<T>>().drop_in_place() });
        }
        // (By this library's own address too: two modules may name a type
        // alike.)
        let name = format!("{:p} {}", &raw const API, std::any::type_name::<T>());
        let (size, align) = (size_of::<RefCell<T>>(), align_of::<RefCell<T>>());
        // SAFETY: The registrar's own host, inside the entry call; else a
        // host handle, lent to a job or a timer's call.
        let at = unsafe {
            match self.registrar {
                Some(r) => (r.state)(
                    r.host,
                    name.as_ptr(),
                    name.len(),
                    size,
                    align,
                    init::<T>,
                    free::<T>,
                ),
                None => (api().state)(
                    self.handle(),
                    name.as_ptr(),
                    name.len(),
                    size,
                    align,
                    init::<T>,
                    free::<T>,
                ),
            }
        };
        State { at: at.cast() }
    }
    /// Registers a function scripts call for its value now, with
    /// `ui.call_sync(name, arg)` (`ScriptHost::function`): on the
    /// interface's thread, so quick, with no I/O. Its argument is lent for
    /// the call (`to_owned` keeps it).
    pub fn function<F: Fn(Ref<'_>) -> Value + 'static>(&self, name: &str, function: F) {
        unsafe extern "C" fn call<G: Fn(Ref<'_>) -> Value>(
            user: *mut c_void,
            args: *const Raw,
            _len: usize,
            out: *mut Raw,
        ) -> u32 {
            // SAFETY: `user` is the `F` boxed below, alive until `free`;
            // the first of `args` is lent for this call (there is always
            // one); `out` takes what it made, or why it failed.
            unsafe {
                __private::guard(out, || {
                    let made = (*(user as *const G))(Ref {
                        at: args,
                        _of: PhantomData,
                    });
                    __private::give(out, made)
                })
            }
        }
        let user = Box::into_raw(Box::new(function));
        // SAFETY: `call` reads `user` as the box it is, and `free` frees it.
        unsafe { self.raw_function(name, call::<F>, user.cast(), free_box_of(user)) }
    }
    /// [`Host::function`], of a function that may fail: what it returns as
    /// `Err` is raised in the script that called, as an error naming this
    /// function and its module.
    pub fn try_function<F: Fn(Ref<'_>) -> Result<Value, Failed> + 'static>(
        &self,
        name: &str,
        function: F,
    ) {
        unsafe extern "C" fn call<G: Fn(Ref<'_>) -> Result<Value, Failed>>(
            user: *mut c_void,
            args: *const Raw,
            _len: usize,
            out: *mut Raw,
        ) -> u32 {
            // SAFETY: As `function`'s.
            unsafe {
                __private::guard(out, || {
                    match (*(user as *const G))(Ref {
                        at: args,
                        _of: PhantomData,
                    }) {
                        Ok(made) => __private::give(out, made),
                        Err(failed) => __private::fail(out, failed),
                    }
                })
            }
        }
        let user = Box::into_raw(Box::new(function));
        // SAFETY: As `function`'s.
        unsafe { self.raw_function(name, call::<F>, user.cast(), free_box_of(user)) }
    }
    /// Registers a function handed its argument (`ui.call_sync(name,
    /// arg)`), for one that keeps or returns what it is given: no count of
    /// it is made. Any other is cheaper as a [`Host::function`], whose
    /// argument the runtime frees where it is.
    pub fn function_taking<F: Fn(Value) -> Value + 'static>(&self, name: &str, function: F) {
        unsafe extern "C" fn call<G: Fn(Value) -> Value>(
            user: *mut c_void,
            arg: *mut Raw,
            out: *mut Raw,
        ) -> u32 {
            // SAFETY: `user` is the `F` boxed below, alive until `free`;
            // `arg` is owned by this call (dropped as a panic unwinds);
            // `out` takes what it made, or why it failed.
            unsafe {
                let arg = Value::owned(arg.read());
                __private::guard(out, || {
                    let made = (*(user as *const G))(arg);
                    __private::give(out, made)
                })
            }
        }
        let user = Box::into_raw(Box::new(function));
        let r = self.registrar();
        // SAFETY: The registrar's own host, inside the entry call; `call`
        // reads `user` as the box it is, and `free` frees it.
        unsafe {
            (r.function_taking)(
                r.host,
                name.as_ptr(),
                name.len(),
                call::<F>,
                user.cast(),
                free_box_of(user),
            )
        }
    }
    /// Registers an action scripts bind with `ui.action(name, arg)` or call
    /// with `ui.invoke` (`ScriptHost::action`).
    pub fn action<F: Fn(Ref<'_>) + 'static>(&self, name: &str, action: F) {
        unsafe extern "C" fn call<G: Fn(Ref<'_>)>(
            user: *mut c_void,
            arg: *const Raw,
            out: *mut Raw,
        ) -> u32 {
            // SAFETY: As `function`'s; `out` is written only failing.
            unsafe {
                __private::guard(out, || {
                    (*(user as *const G))(Ref {
                        at: arg,
                        _of: PhantomData,
                    });
                    abi::DONE
                })
            }
        }
        let user = Box::into_raw(Box::new(action));
        // SAFETY: As `function`'s.
        unsafe { self.raw_action(name, call::<F>, user.cast(), free_box_of(user)) }
    }
    /// [`Host::action`], of an action that may fail: what it returns as
    /// `Err` is reported as a script's error is, naming this action and its
    /// module.
    pub fn try_action<F: Fn(Ref<'_>) -> Result<(), Failed> + 'static>(
        &self,
        name: &str,
        action: F,
    ) {
        unsafe extern "C" fn call<G: Fn(Ref<'_>) -> Result<(), Failed>>(
            user: *mut c_void,
            arg: *const Raw,
            out: *mut Raw,
        ) -> u32 {
            // SAFETY: As `function`'s; `out` is written only failing.
            unsafe {
                __private::guard(out, || {
                    match (*(user as *const G))(Ref {
                        at: arg,
                        _of: PhantomData,
                    }) {
                        Ok(()) => abi::DONE,
                        Err(failed) => __private::fail(out, failed),
                    }
                })
            }
        }
        let user = Box::into_raw(Box::new(action));
        // SAFETY: As `function`'s.
        unsafe { self.raw_action(name, call::<F>, user.cast(), free_box_of(user)) }
    }
    /// Registers `call` itself, with what it is handed as `user` and what
    /// frees that: what [`Host::function`] does with a closure's, and what
    /// [`export`] does with a function it wrote.
    ///
    /// # Safety
    /// `call` is sound called as [`abi::Function`] says with this `user`,
    /// on this thread, until `free` is called with it, once.
    #[doc(hidden)]
    pub unsafe fn raw_function(
        &self,
        name: &str,
        call: abi::Function,
        user: *mut c_void,
        free: abi::Free,
    ) {
        let r = self.registrar();
        // SAFETY: The registrar's own host, inside the entry call.
        unsafe { (r.function)(r.host, name.as_ptr(), name.len(), call, user, free) }
    }
    /// [`Host::raw_function`], of an action.
    ///
    /// # Safety
    /// As `raw_function`'s, by [`abi::Action`].
    #[doc(hidden)]
    pub unsafe fn raw_action(
        &self,
        name: &str,
        call: abi::Action,
        user: *mut c_void,
        free: abi::Free,
    ) {
        let r = self.registrar();
        // SAFETY: The registrar's own host, inside the entry call.
        unsafe { (r.action)(r.host, name.as_ptr(), name.len(), call, user, free) }
    }
    /// [`Host::raw_function`], of an async action.
    ///
    /// # Safety
    /// As `raw_function`'s, by [`abi::AsyncAction`].
    #[doc(hidden)]
    pub unsafe fn raw_async_action(
        &self,
        name: &str,
        call: abi::AsyncAction,
        user: *mut c_void,
        free: abi::Free,
    ) {
        let r = self.registrar();
        // SAFETY: The registrar's own host, inside the entry call.
        unsafe { (r.async_action)(r.host, name.as_ptr(), name.len(), call, user, free) }
    }
    /// Registers an async action scripts call with `ui.call(name, arg)`
    /// (`ScriptHost::async_action`), getting a promise of its result: it
    /// returns at once, and settles `promise` later ([`Promise::resolve`],
    /// [`Promise::reject`]), on the interface's thread, from a job a
    /// thread of its own posts ([`Ui::post`]). A promise dropped unsettled
    /// rejects, naming the action and the module.
    pub fn async_action<F: Fn(Ref<'_>, Promise<Value>) + 'static>(&self, name: &str, action: F) {
        unsafe extern "C" fn call<G: Fn(Ref<'_>, Promise<Value>)>(
            user: *mut c_void,
            arg: *const Raw,
            promise: *mut c_void,
        ) {
            // SAFETY: `user` is the `F` boxed below, alive until `free`;
            // `arg` lent for the call; the promise the module's (a panic
            // drops it, which rejects it).
            __private::guard_async(|| unsafe {
                (*(user as *const G))(
                    Ref {
                        at: arg,
                        _of: PhantomData,
                    },
                    Promise::from_raw(promise),
                )
            })
        }
        let user = Box::into_raw(Box::new(action));
        // SAFETY: `call` reads `user` as the `F` it is, and the box's free.
        unsafe { self.raw_async_action(name, call::<F>, user.cast(), free_box_of(user)) }
    }
    /// Registers a line highlighter a textarea calls itself, with no script
    /// between (`ui.highlighter(name, styles)`, which maps `kinds` to
    /// colours): given a line and what the line before left (0: nothing
    /// open), it hands each span to `push` (bytes from 1, both ends in, and
    /// its kind's number in `kinds`) and returns what this line leaves.
    /// Its states are below 2^31 - 1 (`abi::HIGHLIGHT_STATES`). A panic in
    /// it is caught (and printed, as any): the runtime turns it off for that
    /// textarea and shows why, as it does a module's error. Called on the
    /// interface's thread alone, as long as the host or a textarea or script
    /// of it holds it, then dropped once. False where the
    /// runtime is older than highlighters: the module loads without one.
    /// Unstable: its shape may change in any release.
    #[doc(hidden)]
    pub fn highlighter<F>(&self, name: &str, kinds: &[&str], highlight: F) -> bool
    where
        F: Fn(&[u8], u32, &mut dyn FnMut(u32, u32, u32)) -> u32 + 'static,
    {
        let r = self.registrar();
        if (r.size as usize)
            < std::mem::offset_of!(abi::Registrar, highlighter) + size_of::<usize>()
        {
            return false;
        }
        unsafe extern "C" fn call<G: Fn(&[u8], u32, &mut dyn FnMut(u32, u32, u32)) -> u32>(
            user: *mut c_void,
            line: *const u8,
            len: usize,
            state: u32,
            sink: *mut c_void,
            push: abi::Push,
        ) -> u32 {
            let line = if len == 0 {
                &[][..]
            } else {
                // SAFETY: Lent by the runtime for the call.
                unsafe { std::slice::from_raw_parts(line, len) }
            };
            // A panic can't cross into the runtime: it is the highlighter
            // failing, which turns it off (`abi::HIGHLIGHT_FAILED`).
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // SAFETY: `user` is the `G` boxed below; `sink` the
                // runtime's, for `push`, both for the call.
                unsafe {
                    (*(user as *const G))(line, state, &mut |from, to, kind| {
                        push(sink, from, to, kind)
                    })
                }
            }))
            .unwrap_or(abi::HIGHLIGHT_FAILED)
        }
        let names: Vec<abi::Bytes> = kinds
            .iter()
            .map(|k| abi::Bytes {
                ptr: k.as_ptr(),
                len: k.len(),
            })
            .collect();
        let user = Box::into_raw(Box::new(highlight));
        // SAFETY: The registrar's own host, inside the entry call; `names`
        // lent for it.
        unsafe {
            (r.highlighter)(
                r.host,
                name.as_ptr(),
                name.len(),
                names.as_ptr(),
                names.len(),
                call::<F>,
                user.cast(),
                free_box_of(user),
            )
        }
        true
    }
}

/// A handle to a host for a module's own threads ([`Host::ui`]): `Send`,
/// and cloned as often as threads need. Values aren't (their counts aren't
/// atomic): a thread [`post`](Self::post)s a job, and the job makes and
/// sets them on the interface's thread.
pub struct Ui(*mut c_void);
// SAFETY: A host handle may be held, cloned, posted to and let go of on any
// thread (`Api::host_retain`, `post`, `host_release`).
unsafe impl Send for Ui {}
// SAFETY: As `Send`'s: nothing of it is read but through those.
unsafe impl Sync for Ui {}
impl Clone for Ui {
    fn clone(&self) -> Self {
        // SAFETY: A host handle, held.
        Ui(unsafe { (api().host_retain)(self.0) })
    }
}
impl Drop for Ui {
    fn drop(&mut self) {
        // SAFETY: The handle this one holds, let go of once.
        unsafe { (api().host_release)(self.0) }
    }
}
impl Ui {
    /// Runs `job` on the host's thread, the interface's, at its next turn,
    /// in one reactive batch (what it sets updates once): from any thread,
    /// waking the host only as it posts. False once the host is gone (the
    /// job is dropped, unrun, here).
    pub fn post<J: FnOnce(&Host) + Send + 'static>(&self, job: J) -> bool {
        unsafe extern "C" fn run<J: FnOnce(&Host)>(user: *mut c_void, host: *mut c_void) {
            // SAFETY: `user` is the `J` boxed below, run once and freed as
            // it runs; `host` lent for the call.
            let job = *unsafe { Box::from_raw(user as *mut J) };
            // (A panic is caught and printed: the jobs after it run.)
            __private::guard_quiet(|| job(&Host::lent(host)));
        }
        // (A job that captures nothing allocates nothing.)
        let user = Box::into_raw(Box::new(job));
        // SAFETY: A host handle, held; `user` and its `free` the box's,
        // which may be sent (`J: Send`).
        unsafe { (api().post)(self.0, run::<J>, user.cast(), free_box_of(user)) == abi::POSTED }
    }
}

/// A host's shared state ([`Host::shared`]): what a module publishes its
/// model in, for scripts to read and follow, and what it hears scripts
/// write. On the interface's thread (it is not `Send`): a module's thread
/// sets it from a job it posts ([`Ui::post`]).
pub struct Shared(*mut c_void);
impl Drop for Shared {
    fn drop(&mut self) {
        // SAFETY: The handle this one holds, let go of once.
        unsafe { (api().shared_release)(self.0) }
    }
}
impl Shared {
    /// What it holds, counted, not copied (`nil` once its host is gone).
    /// Read while a binding asks a module, it is followed as a script's
    /// `get` is.
    pub fn get(&self) -> Value {
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: A handle, held; the runtime writes `out`, owned.
        unsafe {
            (api().shared_get)(self.0, out.as_mut_ptr());
            Value::owned(out.assume_init())
        }
    }
    /// Sets it, keeping the parts of what it held that are equal (publish a
    /// whole model each time: only what changed costs anything); whether
    /// anything changed. An error once its host is gone (nothing set).
    pub fn set(&self, value: Value) -> Result<bool, Failed> {
        let mut value = std::mem::ManuallyDrop::new(value);
        // SAFETY: A handle, held; `value` owned and given.
        match unsafe { (api().shared_set)(self.0, std::ptr::from_mut(&mut value.raw)) } {
            abi::HOST_GONE => Err(host_gone()),
            changed => Ok(changed != 0),
        }
    }
    /// `text` added to the end of the string it holds, as a script's
    /// `state:append(text)`: what follows a long one reads the edit alone.
    /// An error if it holds something else, or its host is gone.
    pub fn append(&self, text: &str) -> Result<(), Failed> {
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: A handle, held; the runtime writes `out` only failing.
        unsafe {
            if (api().shared_append)(self.0, text.as_ptr(), text.len(), out.as_mut_ptr())
                == abi::DONE
            {
                return Ok(());
            }
            let why = Value::owned(out.assume_init());
            Err(Failed(
                why.get().str().unwrap_or("can't be added to").to_owned(),
            ))
        }
    }
    /// `rows` written into the list it holds, each by its field `key` (a
    /// string): a row whose key the list has replaces the first row of
    /// that key where it is, the others are added at the end, and of two
    /// given with one key the last is written. One pass over the list, and
    /// only `rows` are made: diff a model on the module's own thread and
    /// post the rows that changed. Holding nothing (`nil`) yet, it becomes
    /// a list of them; an error, writing nothing, if it holds something
    /// else, a row has no string at `key`, or its host is gone.
    pub fn upsert(&self, key: &str, rows: impl IntoIterator<Item = Value>) -> Result<(), Failed> {
        let mut rows = std::mem::ManuallyDrop::new(Value::list(rows));
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: A handle, held; `rows` given; the runtime writes `out`
        // only failing.
        unsafe {
            if (api().shared_upsert)(
                self.0,
                key.as_ptr(),
                key.len(),
                std::ptr::from_mut(&mut rows.raw),
                out.as_mut_ptr(),
            ) == abi::DONE
            {
                return Ok(());
            }
            let why = Value::owned(out.assume_init());
            Err(Failed(
                why.get().str().unwrap_or("couldn't be written").to_owned(),
            ))
        }
    }
    /// [`Shared::upsert`] with a place for each row ([`Place`]): just before
    /// a key's row, at the end, or where its key's row is. A row the list
    /// has moves to its place, so a module that diffs a reordered list on
    /// its thread posts only the rows that moved, each placed before the row
    /// that follows it. Rows placed before one key go in the order given;
    /// one walk of the list. An error, nothing written, if a row is placed
    /// before itself, before a key neither the list nor the rows have, rows
    /// are placed before one another, or what `upsert` refuses.
    pub fn upsert_at<'p>(
        &self,
        key: &str,
        rows: impl IntoIterator<Item = (Value, Place<'p>)>,
    ) -> Result<(), Failed> {
        let (rows, places): (Vec<Value>, Vec<Value>) = rows
            .into_iter()
            .map(|(row, place)| {
                let place = match place {
                    Place::Stay => Value::NIL,
                    Place::Before(key) => Value::from(key),
                    Place::End => Value::from(true),
                };
                (row, place)
            })
            .unzip();
        let mut rows = std::mem::ManuallyDrop::new(Value::list(rows));
        let mut places = std::mem::ManuallyDrop::new(Value::list(places));
        let mut out = MaybeUninit::<Raw>::uninit();
        // SAFETY: A handle, held; both lists given; `out` written only
        // failing.
        unsafe {
            if (api().shared_upsert_at)(
                self.0,
                key.as_ptr(),
                key.len(),
                std::ptr::from_mut(&mut rows.raw),
                std::ptr::from_mut(&mut places.raw),
                out.as_mut_ptr(),
            ) == abi::DONE
            {
                return Ok(());
            }
            let why = Value::owned(out.assume_init());
            Err(Failed(
                why.get().str().unwrap_or("couldn't be written").to_owned(),
            ))
        }
    }
    /// The rows of the list it holds whose field `key` is one of `keys`
    /// removed (every such row); whether any was. An error once its host is
    /// gone.
    pub fn remove<'k>(
        &self,
        key: &str,
        keys: impl IntoIterator<Item = &'k str>,
    ) -> Result<bool, Failed> {
        let keys: Vec<Bytes> = keys
            .into_iter()
            .map(|k| Bytes {
                ptr: k.as_ptr(),
                len: k.len(),
            })
            .collect();
        // SAFETY: A handle, held; the keys' bytes alive for the call.
        match unsafe {
            (api().shared_remove)(self.0, key.as_ptr(), key.len(), keys.as_ptr(), keys.len())
        } {
            abi::HOST_GONE => Err(host_gone()),
            removed => Ok(removed != 0),
        }
    }
    /// Calls `observe` each time it changes, from now, on the interface's
    /// thread after the batch that changed it, with what it holds (lent),
    /// until the [`Observer`] is dropped.
    pub fn observe<F: FnMut(&Host, Ref<'_>) + 'static>(&self, observe: F) -> Observer {
        unsafe extern "C" fn call<F: FnMut(&Host, Ref<'_>)>(
            user: *mut c_void,
            host: *mut c_void,
            value: *const Raw,
        ) {
            // SAFETY: `user` is the `F` boxed below, alive until its
            // observer goes; `host` and `value` lent for the call.
            let observe = unsafe { &mut *(user as *mut F) };
            // (A panic is caught and printed: it observes the next change.)
            __private::guard_quiet(|| {
                observe(
                    &Host::lent(host),
                    Ref {
                        at: value,
                        _of: PhantomData,
                    },
                )
            });
        }
        let user = Box::into_raw(Box::new(observe));
        // SAFETY: A handle, held; `user` and its `free` the box's.
        Observer(unsafe {
            (api().shared_observe)(self.0, call::<F>, user.cast(), free_box_of(user))
        })
    }
}

/// A shared state's observer ([`Shared::observe`]): dropping it stops it.
#[must_use = "an observer stops when dropped"]
pub struct Observer(*mut c_void);
impl Drop for Observer {
    fn drop(&mut self) {
        // (Null: its host was gone as it was made.)
        if !self.0.is_null() {
            // SAFETY: The subscription this one holds, let go of once.
            unsafe { (api().shared_unobserve)(self.0) }
        }
    }
}

/// An async action's promise ([`Host::async_action`], `#[export(async)]`):
/// what the script that called waits on. `Send`, so a thread of the
/// module's may hold it while it works; settled on the interface's thread
/// (a `&Host` says so), once. Dropped unsettled, it rejects, saying the
/// module dropped it.
#[must_use = "a promise dropped unsettled rejects"]
pub struct Promise<T = Value> {
    at: *mut c_void,
    _of: PhantomData<fn(T)>,
}
// SAFETY: A promise may be held and let go of on any thread
// (`Api::promise_release`); settled only with a `&Host`, on the interface's.
unsafe impl<T> Send for Promise<T> {}
impl<T> Promise<T> {
    /// # Safety
    /// `at` is a promise the runtime handed the module, unsettled.
    #[doc(hidden)]
    pub unsafe fn from_raw(at: *mut c_void) -> Self {
        Promise {
            at,
            _of: PhantomData,
        }
    }
    /// A handle to the host of the script waiting on it: how a thread
    /// holding it posts back ([`Ui::post`]).
    pub fn ui(&self) -> Ui {
        // SAFETY: An unsettled promise, held; the handle the `Ui` owns.
        Ui(unsafe { (api().promise_host)(self.at) })
    }
    /// Settles it on the interface's thread, from any: posts `make`, which
    /// makes its result there (a value is that thread's) and settles it
    /// with that. Dropped unrun if the host is gone (no one waits then).
    pub fn resolve_with<F>(self, make: F)
    where
        T: IntoValue + 'static,
        F: FnOnce(&Host) -> Result<T, Failed> + Send + 'static,
    {
        let ui = self.ui();
        ui.post(move |host| match make(host) {
            Ok(value) => self.resolve(host, value),
            Err(why) => self.reject(host, why),
        });
    }
    /// Rejects it: `why` is raised in the script that waits, naming the
    /// action and the module, as a failing function's is.
    pub fn reject(self, _host: &Host, why: impl Into<Failed>) {
        self.settle(false, Value::from(why.into().0));
    }
    /// Rejects it with `why` as it is.
    fn reject_now(self, why: Value) {
        self.settle(false, why);
    }
    fn settle(self, ok: bool, value: Value) {
        let promise = std::mem::ManuallyDrop::new(self);
        let mut value = std::mem::ManuallyDrop::new(value);
        // SAFETY: An unsettled promise, settled once, on the interface's
        // thread (a `&Host` was lent to the caller); `value` given.
        unsafe {
            (api().settle)(
                promise.at,
                u32::from(ok),
                std::ptr::from_mut(&mut value.raw),
            )
        }
    }
}
impl<T: IntoValue> Promise<T> {
    /// Settles it with `value`.
    pub fn resolve(self, _host: &Host, value: T) {
        self.settle(true, value.into_value());
    }
}
impl<T> Drop for Promise<T> {
    fn drop(&mut self) {
        // Dropped as an async action's start panics: rejected with the
        // panic's message once it is caught (`__private::guard_async`).
        if std::thread::panicking() && __private::defer(self.at) {
            return;
        }
        // SAFETY: An unsettled promise, let go of once (any thread).
        unsafe { (api().promise_release)(self.at) }
    }
}

/// A timer ([`Host::after`], [`Host::every`]): dropping it stops it.
#[must_use = "a timer stops when dropped: keep it, or `forget` it to let it run"]
pub struct Timer(*mut c_void);
impl Timer {
    /// Lets the timer run on without its handle, until it ends (`after`'s
    /// once) or its host goes.
    pub fn forget(self) {
        let timer = std::mem::ManuallyDrop::new(self);
        if !timer.0.is_null() {
            // SAFETY: The timer's handle, let go of once, unstopped.
            unsafe { (api().timer_release)(timer.0, 0) }
        }
    }
}
impl Drop for Timer {
    fn drop(&mut self) {
        // (Null: its host was gone as it was made.)
        if !self.0.is_null() {
            // SAFETY: The timer's handle, let go of once, stopping it.
            unsafe { (api().timer_release)(self.0, 1) }
        }
    }
}

/// What frees a registration's `user` that is a `Box<F>`.
fn free_box_of<F>(_: *mut F) -> abi::Free {
    unsafe extern "C" fn free<F>(user: *mut c_void) {
        // SAFETY: The box a registration made, freed once (a panic in a
        // closure's drop caught and printed).
        __private::guard_quiet(|| drop(unsafe { Box::from_raw(user as *mut F) }));
    }
    free::<F>
}

/// Where a row given to [`Shared::upsert_at`] goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place<'a> {
    /// Where the list's row of its key is, or at the end if it has none:
    /// as [`Shared::upsert`] writes it.
    Stay,
    /// Just before the row with this key: the list's first of it, or one
    /// given with it (wherever that one goes).
    Before(&'a str),
    /// At the end of the list.
    End,
}

/// An image's pixels, written by the module on any thread and given to
/// [`Host::image`]: `width` by `height` RGBA8, straight alpha, rows top to
/// bottom with no gap between them (a row is `width * 4` bytes). Its bytes
/// are allocated as what the image keeps, so showing it copies nothing.
/// It starts all 0 (transparent).
pub struct ImageBuffer {
    handle: *mut c_void,
    bytes: *mut u8,
    width: u32,
    height: u32,
}
// SAFETY: Its bytes are its own until it is given up: written from one
// thread at a time (`&mut self`), from any.
unsafe impl Send for ImageBuffer {}
impl ImageBuffer {
    /// A buffer for a `width` by `height` picture; `None` if either is 0 or
    /// it would be larger than an image can be.
    pub fn new(width: u32, height: u32) -> Option<ImageBuffer> {
        let len = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        let mut bytes = std::ptr::null_mut();
        // SAFETY: `bytes` takes where its bytes are.
        let handle = unsafe { (api().image_buffer)(len, &mut bytes) };
        (!handle.is_null()).then_some(ImageBuffer {
            handle,
            bytes,
            width,
            height,
        })
    }
    /// Its width, in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }
    /// Its height, in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }
    /// Its bytes, to write: `width * height * 4`, a pixel's red, green, blue
    /// and alpha in turn.
    pub fn pixels(&mut self) -> &mut [u8] {
        // SAFETY: Its own `width * height * 4` bytes, all initialized,
        // borrowed as it is.
        unsafe {
            std::slice::from_raw_parts_mut(
                self.bytes,
                self.width as usize * self.height as usize * 4,
            )
        }
    }
}
impl Drop for ImageBuffer {
    fn drop(&mut self) {
        // SAFETY: Its handle, not given to `image_set`: let go of once.
        unsafe { (api().image_drop)(self.handle) };
    }
}

/// Why a call failed: what a function or an action returns as `Err`, raised
/// in the script that called with the function's and the module's names
/// before it. Say what was wrong with what the script passed, as the end of
/// a sentence: `"has no row 12"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failed(pub String);
/// Why a write to a shared state did nothing: its host is gone (the
/// runtime's words for it, as `append` and `upsert` say).
fn host_gone() -> Failed {
    Failed("its host is gone".to_owned())
}
impl From<String> for Failed {
    fn from(why: String) -> Self {
        Failed(why)
    }
}
impl From<&str> for Failed {
    fn from(why: &str) -> Self {
        Failed(why.to_owned())
    }
}
impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What [`export`] and [`module`] write their code with. Not for a module's
/// own use: it changes with them.
#[doc(hidden)]
pub mod __private {
    use super::{Failed, FromValue, Raw, Ref, State, Value, abi};
    pub use std::ffi::c_void;

    /// The argument at `index` of the `len` a function was lent: a `nil` if
    /// it wasn't passed.
    ///
    /// # Safety
    /// `args` are a call's (`abi::Function`): `len` values, and at least
    /// one to read.
    #[inline(always)]
    pub unsafe fn arg<'a>(args: *const Raw, len: usize, index: usize) -> Ref<'a> {
        if index == 0 || index < len {
            Ref {
                // SAFETY: Among the values lent.
                at: unsafe { args.add(index) },
                _of: std::marker::PhantomData,
            }
        } else {
            Ref::NIL
        }
    }
    /// A call's result, written where the runtime takes it.
    ///
    /// # Safety
    /// `out` has room for a value and holds none.
    #[inline(always)]
    pub unsafe fn give(out: *mut Raw, made: Value) -> u32 {
        // SAFETY: The caller's; the value is the runtime's from here.
        unsafe { out.write(made.raw) };
        std::mem::forget(made);
        abi::DONE
    }
    /// Why a call failed, written where its result would have gone.
    ///
    /// # Safety
    /// As `give`'s.
    #[cold]
    #[inline(never)]
    pub unsafe fn fail(out: *mut Raw, failed: Failed) -> u32 {
        // SAFETY: The caller's.
        unsafe { give(out, Value::from(failed.0)) };
        abi::FAILED
    }
    /// `call`, a call of a module's function or action, whose panic is
    /// caught at the boundary and fails the call (written to `out`), as an
    /// error it returned would: raised in the script that called, the
    /// module still loaded.
    ///
    /// # Safety
    /// As `give`'s, while `call` hasn't written it.
    #[inline(always)]
    pub unsafe fn guard(out: *mut Raw, call: impl FnOnce() -> u32) -> u32 {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)) {
            Ok(status) => status,
            // SAFETY: The caller's: a panicking call wrote nothing.
            Err(panic) => unsafe { fail(out, panicked(&*panic)) },
        }
    }
    thread_local! {
        /// Inside an async action's start (`guard_async`), how deep: and
        /// the promises dropped as it unwound, to reject once it is caught.
        static STARTING: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        static DROPPED: std::cell::RefCell<Vec<*mut c_void>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    /// `call`, an async action's start, whose panic is caught at the
    /// boundary: a promise it dropped as it unwound (its own) is rejected
    /// with the panic's message, as a call's failure is.
    #[inline(always)]
    pub fn guard_async(call: impl FnOnce()) {
        STARTING.with(|s| s.set(s.get() + 1));
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(call));
        STARTING.with(|s| s.set(s.get() - 1));
        if let Err(panic) = ran {
            let why = panicked(&*panic);
            for promise in DROPPED.with_borrow_mut(std::mem::take) {
                // SAFETY: A promise the runtime handed this start, dropped
                // unsettled as it unwound, settled here once, on the
                // interface's thread.
                unsafe { super::Promise::<Value>::from_raw(promise) }
                    .reject_now(Value::from(why.0.as_str()));
            }
        }
    }
    /// Whether a promise dropped as a panic unwinds is kept to reject
    /// (inside an async action's start, on its thread), not let go.
    pub(crate) fn defer(promise: *mut c_void) -> bool {
        STARTING.with(|s| s.get() > 0) && {
            DROPPED.with_borrow_mut(|d| d.push(promise));
            true
        }
    }
    /// `call`, a module's job, timer, observer or drop, whose panic is
    /// caught at the boundary (printed, as any panic is) with no one to
    /// tell: whether it didn't panic.
    #[inline(always)]
    pub fn guard_quiet(call: impl FnOnce()) -> bool {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)).is_ok()
    }
    /// What a call that panicked fails with: the panic's message.
    #[cold]
    #[inline(never)]
    pub fn panicked(panic: &(dyn std::any::Any + Send)) -> Failed {
        let message = panic
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("(a panic with no message)");
        Failed(format!("panicked: {message}"))
    }
    /// A call failing over the argument at `index` (from 0), named `name`,
    /// which isn't a `T`.
    ///
    /// # Safety
    /// As `give`'s.
    #[cold]
    #[inline(never)]
    pub unsafe fn wrong<'a, T: FromValue<'a>>(
        out: *mut Raw,
        index: usize,
        name: &str,
        value: Ref<'a>,
    ) -> u32 {
        let why = format!("argument {} ({name}) {}", index + 1, T::wrong(value));
        // SAFETY: The caller's.
        unsafe { fail(out, Failed(why)) }
    }
    /// A registration's `user` that is a module's state, as the function
    /// registered with it reads it.
    ///
    /// # Safety
    /// `user` is `state_user::<T>`'s, on the thread it was made on.
    #[inline(always)]
    pub unsafe fn state<'a, T: 'static>(user: *mut c_void) -> &'a std::cell::RefCell<T> {
        // SAFETY: The caller's: the runtime keeps a module's state for as
        // long as the thread runs.
        unsafe { &*(user as *const std::cell::RefCell<T>) }
    }
    /// A module's state as a registration's `user`.
    pub fn state_user<T: 'static>(state: State<T>) -> *mut c_void {
        state.at as *mut c_void
    }
    /// What frees a `user` that needs no freeing.
    pub unsafe extern "C" fn keep(_user: *mut c_void) {}
    /// An async action's promise, from the runtime's pointer.
    ///
    /// # Safety
    /// As [`super::Promise::from_raw`]'s.
    #[inline(always)]
    pub unsafe fn promise<T>(at: *mut c_void) -> super::Promise<T> {
        // SAFETY: The caller's.
        unsafe { super::Promise::from_raw(at) }
    }
    /// An async action rejected at once over the argument at `index`,
    /// named `name`, which isn't a `T`.
    ///
    /// # Safety
    /// `promise` is an unsettled promise, on the interface's thread.
    #[cold]
    #[inline(never)]
    pub unsafe fn wrong_async<'a, T: FromValue<'a>>(
        promise: *mut c_void,
        index: usize,
        name: &str,
        value: Ref<'a>,
    ) {
        let why = format!("argument {} ({name}) {}", index + 1, T::wrong(value));
        let mut why = std::mem::ManuallyDrop::new(Value::from(why));
        // SAFETY: The caller's; `why` given.
        unsafe { (super::api().settle)(promise, 0, std::ptr::from_mut(&mut why.raw)) }
    }
}

/// What a module's entry does ([`register!`], [`module`]): checks the runtime speaks this module's
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
        // (A registrar may end before its highlighter: `Host::highlighter`
        // says so, and the module loads without one.)
        || (registrar.size as usize) < std::mem::offset_of!(abi::Registrar, highlighter)
    {
        return abi::REFUSED_ABI;
    }
    API.store(api.cast_mut(), Ordering::Relaxed);
    // A panic in setup can't cross into the runtime: the module doesn't
    // load, and what it registered before it is let go.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        setup(&Host::loading(registrar))
    })) {
        Ok(()) => 0,
        Err(_) => abi::SETUP_PANICKED,
    }
}

/// A module's entry, written by hand: `setup` is called with the [`Host`]
/// each time the module is loaded into one, to register its functions
/// (`Host::function`, `Host::action`), and `name` is what the module is
/// called. What [`module`] writes for a `mod` of exported functions; a
/// module made this way says nothing of its functions in its manifest, so
/// nothing knows their types.
///
/// ```ignore
/// zgx_module::register!("words", |host: &zgx_module::Host| {
///     host.function("count", |text| {
///         let words = text.str().map_or(0, |t| t.split_whitespace().count());
///         zgx_module::Value::from(words as f64)
///     });
/// });
/// ```
#[macro_export]
macro_rules! register {
    ($name:literal, $setup:expr) => {
        const _: () = assert!(
            $crate::abi::is_name($name),
            "a module's name is letters, digits and `_`"
        );
        /// # Safety
        /// Called by the zgx runtime alone (`zgx_module::abi::Entry`).
        #[unsafe(export_name = concat!("zgx_module_v1_", $name))]
        pub unsafe extern "C" fn zgx_module_v1(
            api: *const $crate::abi::Api,
            registrar: *const $crate::abi::Registrar,
        ) -> u32 {
            // SAFETY: As the runtime calls it.
            unsafe { $crate::enter(api, registrar, $setup) }
        }
        /// The module's manifest (`zgx_module::abi::MANIFEST`): its name.
        #[used]
        #[unsafe(export_name = concat!("zgx_module_manifest_", $name))]
        pub static zgx_module_manifest: [u8; ZGX_MODULE_JSON.len() + 48] =
            $crate::abi::manifest(ZGX_MODULE_JSON);
        #[doc(hidden)]
        const ZGX_MODULE_JSON: &str = concat!(
            "{\"zgx_module\":1,\"abi\":1,\"name\":\"",
            $name,
            "\",\"version\":\"",
            env!("CARGO_PKG_VERSION"),
            "\"}"
        );
    };
}
