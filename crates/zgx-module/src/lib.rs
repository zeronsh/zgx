//! Native modules: an app's own Rust, built apart from zgx as a library
//! (`crate-type = ["cdylib"]`) the runtime loads, its functions called from
//! Luau as a host's are (`ui.call_sync`, `ui.action`).
//!
//! ```ignore
//! use zgx_module::{Host, Value};
//!
//! zgx_module::module!(|host: &Host| {
//!     host.function("words.count", |text: Value| {
//!         let n = text.get().str().map_or(0, |s| s.split_whitespace().count());
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
        unsafe { (api().item)(&raw, index, &mut out) }.then_some(Ref {
            raw: out,
            _of: PhantomData,
        })
    }
    /// A list's items, none if it isn't one.
    pub fn items(self) -> impl ExactSizeIterator<Item = Ref<'a>> + DoubleEndedIterator {
        let raw = self.opened();
        let len = if raw.tag == abi::LIST {
            raw.b as usize
        } else {
            0
        };
        let list = Ref {
            raw,
            _of: PhantomData,
        };
        (0..len).map(move |i| {
            list.item(i).unwrap_or(Ref {
                raw: NIL,
                _of: PhantomData,
            })
        })
    }
    /// A map's field.
    #[inline]
    pub fn field(self, key: &str) -> Option<Ref<'a>> {
        let raw = self.opened();
        if raw.tag != abi::MAP {
            return None;
        }
        let mut out = NIL;
        // SAFETY: A lent map, the key's bytes, and room for one value.
        unsafe { (api().field)(&raw, key.as_ptr(), key.len(), &mut out) }.then_some(Ref {
            raw: out,
            _of: PhantomData,
        })
    }
    /// A map's fields in key order, none if it isn't one.
    pub fn fields(self) -> impl ExactSizeIterator<Item = (&'a str, Ref<'a>)> {
        let raw = self.opened();
        let len = if raw.tag == abi::MAP {
            raw.b as usize
        } else {
            0
        };
        (0..len).map(move |i| {
            let mut key = Bytes {
                ptr: std::ptr::NonNull::dangling().as_ptr(),
                len: 0,
            };
            let mut value = NIL;
            // SAFETY: A lent map, and room for a key and a value. A key is
            // UTF-8, alive as long as the map.
            unsafe {
                (api().entry)(&raw, i, &mut key, &mut value);
                let key =
                    std::str::from_utf8_unchecked(std::slice::from_raw_parts(key.ptr, key.len));
                (
                    key,
                    Ref {
                        raw: value,
                        _of: PhantomData,
                    },
                )
            }
        })
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

/// The host a module is loading into: where it registers what scripts
/// call. Lent to the module's setup, not kept.
pub struct Host<'a> {
    registrar: &'a abi::Registrar,
}

impl Host<'_> {
    /// Registers a function scripts call for its value now, with
    /// `ui.call_sync(name, arg)` (`ScriptHost::function`): on the
    /// interface's thread, so quick, with no I/O.
    pub fn function(&self, name: &str, function: impl Fn(Value) -> Value + 'static) {
        unsafe extern "C" fn call<F: Fn(Value) -> Value>(user: *mut c_void, arg: Raw) -> Raw {
            // SAFETY: `user` is the `F` boxed below, alive until `free`;
            // `arg` is owned by this call.
            unsafe { (*(user as *const F))(Value::from_raw(arg)).into_raw() }
        }
        unsafe extern "C" fn free<F>(user: *mut c_void) {
            // SAFETY: The box made below, freed once.
            drop(unsafe { Box::from_raw(user as *mut F) });
        }
        fn register<F: Fn(Value) -> Value + 'static>(r: &abi::Registrar, name: &str, f: F) {
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
    pub fn action(&self, name: &str, action: impl Fn(Value) + 'static) {
        unsafe extern "C" fn call<F: Fn(Value)>(user: *mut c_void, arg: Raw) {
            // SAFETY: As `function`'s.
            unsafe { (*(user as *const F))(Value::from_raw(arg)) }
        }
        unsafe extern "C" fn free<F>(user: *mut c_void) {
            // SAFETY: The box made below, freed once.
            drop(unsafe { Box::from_raw(user as *mut F) });
        }
        fn register<F: Fn(Value) + 'static>(r: &abi::Registrar, name: &str, f: F) {
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
