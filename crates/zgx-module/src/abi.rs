//! What crosses between a module and the runtime: C types and C functions
//! only, so neither side needs the other's compiler or build.
//!
//! A value is three words ([`Raw`]). Booleans and numbers are in it; a
//! string, a list and a map are a pointer and a length into memory the
//! runtime owns and counts. A module reads a string's bytes where they are,
//! asks the runtime for a list's item or a map's field (lent, nothing
//! counted or copied), and makes values through [`Api`].

use std::ffi::c_void;

/// This interface's number: a module and a runtime agree on it or the
/// module doesn't load. It changes only when something here is taken away
/// or changes meaning; what is added goes at the end of [`Api`] and
/// [`Registrar`], told by their `size`.
pub const ABI: u32 = 1;

/// The symbol a module exports ([`Entry`]).
pub const ENTRY: &[u8] = b"zgx_module_v1\0";

/// `nil`.
pub const NIL: u32 = 0;
/// A boolean: `a` is 0 or 1.
pub const BOOL: u32 = 1;
/// A number: `a` is its `f64`'s bits.
pub const NUM: u32 = 2;
/// A string: `a` points at its UTF-8, `b` is its length in bytes.
pub const STR: u32 = 3;
/// A list: `b` is how many items; `a` is the runtime's.
pub const LIST: u32 = 4;
/// A map with string keys, sorted by key: `b` is how many fields; `a` is
/// the runtime's.
pub const MAP: u32 = 5;
/// Any other value the runtime holds (a long text, an image, an element, a
/// row kept folded): a module passes it on, and reads it through [`Api`].
pub const OTHER: u32 = 6;

/// A value. Owned (released once, with [`Api::release`], or handed back to
/// the runtime) or lent (valid while what lent it is, never released):
/// which one is by where it came from, said at each function.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Raw {
    pub tag: u32,
    pub a: u64,
    pub b: u64,
}

/// A string's bytes, lent.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Bytes {
    pub ptr: *const u8,
    pub len: usize,
}

/// The runtime's functions for values, the same for every module and for
/// as long as the process runs. All are called on the thread the module
/// was loaded on (the interface's).
#[repr(C)]
pub struct Api {
    /// [`ABI`].
    pub abi: u32,
    /// This struct's size in bytes, as the runtime built it: a module
    /// built against a longer one reads no further than this.
    pub size: u32,
    /// A string of these bytes (UTF-8), owned.
    pub string: unsafe extern "C" fn(ptr: *const u8, len: usize) -> Raw,
    /// A list of these `len` owned values, which it takes; owned.
    pub list: unsafe extern "C" fn(items: *mut Raw, len: usize) -> Raw,
    /// A map of these `len` keys and owned values, which it takes, in any
    /// order (the last of a key given twice kept); owned.
    pub map: unsafe extern "C" fn(keys: *const Bytes, values: *mut Raw, len: usize) -> Raw,
    /// The same value once more, owned: counted, not copied.
    pub clone: unsafe extern "C" fn(value: *const Raw) -> Raw,
    /// Gives an owned value back.
    pub release: unsafe extern "C" fn(value: Raw),
    /// A list's item (from 0), lent for as long as the list is; false past
    /// its end or if it isn't a list.
    pub item: unsafe extern "C" fn(list: *const Raw, index: usize, out: *mut Raw) -> bool,
    /// A map's field, lent for as long as the map is; false if it has none
    /// of that name or isn't a map.
    pub field:
        unsafe extern "C" fn(map: *const Raw, key: *const u8, len: usize, out: *mut Raw) -> bool,
    /// A map's field by its place (from 0, in key order), its key and value
    /// lent for as long as the map is.
    pub entry: unsafe extern "C" fn(
        map: *const Raw,
        index: usize,
        key: *mut Bytes,
        value: *mut Raw,
    ) -> bool,
    /// An [`OTHER`] value as what a script would read of it, lent for as
    /// long as it is: a row kept folded as its map. False for what has no
    /// such reading here (an image, an element, a long text held in
    /// chunks), which stays [`OTHER`].
    pub open: unsafe extern "C" fn(value: *const Raw, out: *mut Raw) -> bool,
}

/// A module's function, as `ui.call_sync(name, arg)` calls it: `arg` is
/// owned by the function, and so is what it returns by the runtime.
pub type Function = unsafe extern "C" fn(user: *mut c_void, arg: Raw) -> Raw;
/// A module's action, as `ui.action(name, arg)` and `ui.invoke` call it.
pub type Action = unsafe extern "C" fn(user: *mut c_void, arg: Raw);
/// Frees a registration's `user` when the host that held it goes.
pub type Free = unsafe extern "C" fn(user: *mut c_void);

/// What a module registers with as it loads: valid only inside its
/// [`Entry`] call.
#[repr(C)]
pub struct Registrar {
    /// This struct's size in bytes (as [`Api::size`]).
    pub size: u32,
    /// The host being registered with, handed back in each call below.
    pub host: *mut c_void,
    /// Registers a function scripts call with `ui.call_sync(name, arg)`.
    pub function: unsafe extern "C" fn(
        host: *mut c_void,
        name: *const u8,
        len: usize,
        call: Function,
        user: *mut c_void,
        free: Free,
    ),
    /// Registers an action scripts bind with `ui.action(name, arg)`.
    pub action: unsafe extern "C" fn(
        host: *mut c_void,
        name: *const u8,
        len: usize,
        call: Action,
        user: *mut c_void,
        free: Free,
    ),
}

/// A module's entry, exported as [`ENTRY`]: called once per host it is
/// loaded into. Returns 0, or a reason it can't load ([`REFUSED_ABI`]).
pub type Entry = unsafe extern "C" fn(api: *const Api, registrar: *const Registrar) -> u32;

/// The runtime speaks another [`ABI`], or a shorter [`Api`] than this
/// module needs.
pub const REFUSED_ABI: u32 = 1;
