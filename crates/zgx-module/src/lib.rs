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
//! types and functions, so a module built once runs in any runtime that
//! speaks its [`abi::ABI`], whatever compiler built either
//! (docs/design/runtime.md). Values are the runtime's own, counted not
//! copied: a list a module returns is the list the script reads.
//!
//! A panic in a module ends the process, as one in a host's own function
//! does today: nothing unwinds across the boundary.

pub mod abi;

use std::{
    ffi::c_void,
    marker::PhantomData,
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

const NIL: Raw = Raw {
    tag: abi::NIL,
    a: 0,
    b: 0,
};

/// Whether a value of this tag is counted by the runtime (so cloned and
/// released through it), not held in the three words.
#[inline]
fn counted(tag: u32) -> bool {
    tag >= abi::STR
}

/// A value a module owns: what a function is called with and returns. The
/// runtime's own, so cloning one counts it and copies nothing.
pub struct Value {
    raw: Raw,
    /// The runtime's values belong to the interface's thread.
    _thread: PhantomData<*const ()>,
}

impl Value {
    /// `nil`.
    pub const NIL: Value = Value {
        raw: NIL,
        _thread: PhantomData,
    };

    /// A list of `items`.
    pub fn list(items: impl IntoIterator<Item = Value>) -> Value {
        let mut raws: Vec<Raw> = items.into_iter().map(Value::into_raw).collect();
        // SAFETY: `raws` are owned values, given to the runtime.
        Value::owned(unsafe { (api().list)(raws.as_mut_ptr(), raws.len()) })
    }
    /// A map of `fields`, in any order (the last of a key given twice
    /// kept).
    pub fn record<K: AsRef<str>>(fields: impl IntoIterator<Item = (K, Value)>) -> Value {
        let fields: Vec<(K, Value)> = fields.into_iter().collect();
        let keys: Vec<Bytes> = fields
            .iter()
            .map(|(k, _)| Bytes {
                ptr: k.as_ref().as_ptr(),
                len: k.as_ref().len(),
            })
            .collect();
        // (The keys' text is still held by `held` while the runtime reads it.)
        let (held, values): (Vec<K>, Vec<Value>) = fields.into_iter().unzip();
        let mut raws: Vec<Raw> = values.into_iter().map(Value::into_raw).collect();
        // SAFETY: As many keys as owned values, the keys' bytes alive in `held`.
        let made = unsafe { (api().map)(keys.as_ptr(), raws.as_mut_ptr(), raws.len()) };
        drop(held);
        Value::owned(made)
    }
    /// It, to read.
    #[inline]
    pub fn get(&self) -> Ref<'_> {
        Ref {
            raw: self.raw,
            _of: PhantomData,
        }
    }
    #[inline]
    fn owned(raw: Raw) -> Value {
        Value {
            raw,
            _thread: PhantomData,
        }
    }
    /// Its three words, the caller now owning what they count.
    #[inline]
    pub fn into_raw(self) -> Raw {
        let raw = self.raw;
        std::mem::forget(self);
        raw
    }
    /// # Safety
    /// `raw` is an owned value of this runtime, owned by nothing else.
    #[inline]
    pub unsafe fn from_raw(raw: Raw) -> Value {
        Value::owned(raw)
    }
}
impl Default for Value {
    fn default() -> Self {
        Value::NIL
    }
}
impl Drop for Value {
    #[inline]
    fn drop(&mut self) {
        if counted(self.raw.tag) {
            // SAFETY: Owned, and released once.
            unsafe { (api().release)(self.raw) }
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
        Value::owned(Raw {
            tag: abi::BOOL,
            a: u64::from(b),
            b: 0,
        })
    }
}
impl From<f64> for Value {
    #[inline]
    fn from(n: f64) -> Self {
        Value::owned(Raw {
            tag: abi::NUM,
            a: n.to_bits(),
            b: 0,
        })
    }
}
impl From<&str> for Value {
    #[inline]
    fn from(s: &str) -> Self {
        // SAFETY: A string's bytes, read before this returns.
        Value::owned(unsafe { (api().string)(s.as_ptr(), s.len()) })
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

/// A value lent: an owned one being read, a list's item or a map's field.
/// Reading through it counts and copies nothing.
#[derive(Clone, Copy)]
pub struct Ref<'a> {
    raw: Raw,
    _of: PhantomData<&'a Value>,
}

impl<'a> Ref<'a> {
    /// What a script would read of a value the runtime holds its own way
    /// (a row kept folded as its map); else itself.
    #[inline]
    fn opened(self) -> Raw {
        if self.raw.tag != abi::OTHER {
            return self.raw;
        }
        let mut out = NIL;
        // SAFETY: A lent value, and room for one.
        if unsafe { (api().open)(&self.raw, &mut out) } {
            out
        } else {
            self.raw
        }
    }
    pub fn is_nil(self) -> bool {
        self.raw.tag == abi::NIL
    }
    /// Whether Luau counts it as true: anything but `nil` and `false`.
    pub fn truthy(self) -> bool {
        !matches!((self.raw.tag, self.raw.a), (abi::NIL, _) | (abi::BOOL, 0))
    }
    pub fn bool(self) -> Option<bool> {
        (self.raw.tag == abi::BOOL).then_some(self.raw.a != 0)
    }
    pub fn num(self) -> Option<f64> {
        (self.raw.tag == abi::NUM).then(|| f64::from_bits(self.raw.a))
    }
    /// Its text if it is a string, read where the runtime holds it.
    #[inline]
    pub fn str(self) -> Option<&'a str> {
        let raw = self.opened();
        if raw.tag != abi::STR {
            return None;
        }
        // SAFETY: A string's `a` and `b` are its UTF-8 and its length, alive
        // as long as the value this was lent from.
        Some(unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                raw.a as *const u8,
                raw.b as usize,
            ))
        })
    }
    /// Whether it is a list.
    pub fn is_list(self) -> bool {
        self.opened().tag == abi::LIST
    }
    /// Whether it is a map.
    pub fn is_map(self) -> bool {
        self.opened().tag == abi::MAP
    }
    /// How many items a list has, or fields a map; 0 for anything else.
    #[inline]
    pub fn len(self) -> usize {
        let raw = self.opened();
        if matches!(raw.tag, abi::LIST | abi::MAP) {
            raw.b as usize
        } else {
            0
        }
    }
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
    /// A list's item, from 0.
    #[inline]
    pub fn item(self, index: usize) -> Option<Ref<'a>> {
        let raw = self.opened();
        if raw.tag != abi::LIST || index >= raw.b as usize {
            return None;
        }
        let mut out = NIL;
        // SAFETY: A lent list, and room for one value.
        (unsafe { (api().items)(&raw, index, &mut out, 1) } == 1).then_some(Ref {
            raw: out,
            _of: PhantomData,
        })
    }
    /// A list's items, none if it isn't one: read from the runtime many at
    /// a time, so a pass over a long list crosses to it a few times, not
    /// once an item.
    pub fn items(self) -> Items<'a> {
        let raw = self.opened();
        let len = if raw.tag == abi::LIST {
            raw.b as usize
        } else {
            0
        };
        Items {
            list: raw,
            len,
            next: 0,
            held: [std::mem::MaybeUninit::uninit(); CHUNK],
            from: 0,
            count: 0,
            _of: PhantomData,
        }
    }
    /// A list of this list's items `from` to `to` (from 0, `to` excluded):
    /// the items counted, not copied. `nil` if they aren't in it.
    pub fn slice(self, from: usize, to: usize) -> Value {
        let raw = self.opened();
        // SAFETY: A lent value.
        Value::owned(unsafe { (api().slice)(&raw, from, to) })
    }
    /// A map's field. Reading several of one map's, [`Ref::get`] reads them
    /// in one call.
    #[inline]
    pub fn field(self, key: &str) -> Option<Ref<'a>> {
        let [field] = self.get([key]);
        field
    }
    /// The fields of a map named by `keys`, each at its key's place, read
    /// from the runtime in one call: `let [id, text] = row.get(["id",
    /// "text"])`. None for a field it doesn't have, all none if it isn't a
    /// map.
    #[inline]
    pub fn get<const N: usize>(self, keys: [&str; N]) -> [Option<Ref<'a>>; N] {
        let raw = self.opened();
        if raw.tag != abi::MAP {
            return [None; N];
        }
        let keys = keys.map(|key| Bytes {
            ptr: key.as_ptr(),
            len: key.len(),
        });
        let mut out = [std::mem::MaybeUninit::<Raw>::uninit(); N];
        // SAFETY: A lent map, `N` keys and room for `N` values, each of
        // which the runtime writes.
        unsafe { (api().fields)(&raw, keys.as_ptr(), N, out.as_mut_ptr().cast()) };
        out.map(|raw| {
            // SAFETY: Written by the call above.
            let raw = unsafe { raw.assume_init() };
            (raw.tag != abi::ABSENT).then_some(Ref {
                raw,
                _of: PhantomData,
            })
        })
    }
    /// A map's fields, all read from the runtime at once: for code that
    /// reads several of them. Empty if it isn't a map.
    pub fn record(self) -> Record<'a> {
        let raw = self.opened();
        let len = if raw.tag == abi::MAP {
            raw.b as usize
        } else {
            0
        };
        let mut record = Record {
            len: 0,
            keys: [std::mem::MaybeUninit::uninit(); INLINE],
            values: [std::mem::MaybeUninit::uninit(); INLINE],
            more: Vec::new(),
            _of: PhantomData,
        };
        if len <= INLINE {
            // SAFETY: A lent map, and room for `INLINE` keys and values.
            record.len = unsafe {
                (api().entries)(
                    &raw,
                    0,
                    record.keys.as_mut_ptr().cast(),
                    record.values.as_mut_ptr().cast(),
                    INLINE,
                )
            };
        } else {
            let mut keys = vec![
                Bytes {
                    ptr: std::ptr::null(),
                    len: 0
                };
                len
            ];
            let mut values = vec![NIL; len];
            // SAFETY: A lent map, and room for all its keys and values.
            let n =
                unsafe { (api().entries)(&raw, 0, keys.as_mut_ptr(), values.as_mut_ptr(), len) };
            record.more = keys.into_iter().zip(values).take(n).collect();
        }
        record
    }
    /// A map's fields in key order, none if it isn't one.
    pub fn fields(self) -> impl Iterator<Item = (&'a str, Ref<'a>)> {
        let record = self.record();
        (0..record.len()).map(move |i| record.at(i))
    }
    /// It, owned: counted, not copied.
    #[inline]
    pub fn to_owned(self) -> Value {
        if counted(self.raw.tag) {
            // SAFETY: A lent value; the runtime counts it once more.
            Value::owned(unsafe { (api().clone)(&self.raw) })
        } else {
            Value::owned(self.raw)
        }
    }
}
impl std::fmt::Debug for Ref<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let raw = self.opened();
        match raw.tag {
            abi::NIL => f.write_str("nil"),
            abi::BOOL => write!(f, "{}", raw.a != 0),
            abi::NUM => write!(f, "{}", f64::from_bits(raw.a)),
            abi::STR => write!(f, "{:?}", self.str().unwrap_or("")),
            abi::LIST => f.debug_list().entries(self.items()).finish(),
            abi::MAP => f.debug_map().entries(self.fields()).finish(),
            _ => f.write_str("<value>"),
        }
    }
}

/// How many of a list's items [`Items`] reads from the runtime at a time.
const CHUNK: usize = 32;
/// How many of a map's fields a [`Record`] holds without allocating.
const INLINE: usize = 16;

/// A list's items ([`Ref::items`]).
pub struct Items<'a> {
    list: Raw,
    len: usize,
    next: usize,
    /// The items from `from`, the first `count` written.
    held: [std::mem::MaybeUninit<Raw>; CHUNK],
    from: usize,
    count: usize,
    _of: PhantomData<&'a Value>,
}
impl<'a> Iterator for Items<'a> {
    type Item = Ref<'a>;
    #[inline]
    fn next(&mut self) -> Option<Ref<'a>> {
        if self.next >= self.len {
            return None;
        }
        if self.next >= self.from + self.count {
            // SAFETY: A lent list, and room for `CHUNK` values.
            self.count = unsafe {
                (api().items)(&self.list, self.next, self.held.as_mut_ptr().cast(), CHUNK)
            };
            self.from = self.next;
            if self.count == 0 {
                self.len = self.next;
                return None;
            }
        }
        // SAFETY: Among the `count` the runtime wrote.
        let raw = unsafe { self.held[self.next - self.from].assume_init() };
        self.next += 1;
        Some(Ref {
            raw,
            _of: PhantomData,
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.len - self.next;
        (left, Some(left))
    }
}
impl ExactSizeIterator for Items<'_> {}

/// A map's fields, read from the runtime at once ([`Ref::record`]).
pub struct Record<'a> {
    /// How many of `keys` and `values` the runtime wrote.
    len: usize,
    keys: [std::mem::MaybeUninit<Bytes>; INLINE],
    values: [std::mem::MaybeUninit<Raw>; INLINE],
    /// A map of more than `INLINE` fields: all of them.
    more: Vec<(Bytes, Raw)>,
    _of: PhantomData<&'a Value>,
}
impl<'a> Record<'a> {
    /// How many fields.
    pub fn len(&self) -> usize {
        if self.more.is_empty() {
            self.len
        } else {
            self.more.len()
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[inline]
    fn at(&self, index: usize) -> (&'a str, Ref<'a>) {
        let (key, raw) = if self.more.is_empty() {
            // SAFETY: `index` is under `len`, which the runtime wrote.
            unsafe {
                (
                    self.keys[index].assume_init(),
                    self.values[index].assume_init(),
                )
            }
        } else {
            self.more[index]
        };
        // SAFETY: A key is UTF-8, alive as long as the map it was lent from.
        let key =
            unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(key.ptr, key.len)) };
        (
            key,
            Ref {
                raw,
                _of: PhantomData,
            },
        )
    }
    /// The field `key`.
    #[inline]
    pub fn get(&self, key: &str) -> Option<Ref<'a>> {
        (0..self.len())
            .map(|i| self.at(i))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
    }
    /// Its fields, in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, Ref<'a>)> + '_ {
        (0..self.len()).map(|i| self.at(i))
    }
}

/// The host a module is loading into: where it registers what scripts
/// call. Lent to the module's setup, not kept.
pub struct Host<'a> {
    registrar: &'a abi::Registrar,
}

impl Host<'_> {
    /// Registers a function scripts call for its value now, with
    /// `ui.call_sync(name, arg)` (`ScriptHost::function`): on the
    /// interface's thread, so quick, with no I/O. Its argument is lent for
    /// the call (`to_owned` keeps it).
    pub fn function(&self, name: &str, function: impl Fn(Ref<'_>) -> Value + 'static) {
        unsafe extern "C" fn call<F: Fn(Ref<'_>) -> Value>(
            user: *mut c_void,
            arg: *const Raw,
        ) -> Raw {
            // SAFETY: `user` is the `F` boxed below, alive until `free`;
            // `arg` is lent for this call.
            unsafe {
                (*(user as *const F))(Ref {
                    raw: *arg,
                    _of: PhantomData,
                })
                .into_raw()
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
    /// Registers an action scripts bind with `ui.action(name, arg)` or call
    /// with `ui.invoke` (`ScriptHost::action`).
    pub fn action(&self, name: &str, action: impl Fn(Ref<'_>) + 'static) {
        unsafe extern "C" fn call<F: Fn(Ref<'_>)>(user: *mut c_void, arg: *const Raw) {
            // SAFETY: As `function`'s.
            unsafe {
                (*(user as *const F))(Ref {
                    raw: *arg,
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
