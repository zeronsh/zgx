//! What crosses between a module and the runtime: C functions, and the
//! layout of a value, which a module reads where the runtime holds it.
//!
//! A value is three words ([`Raw`]): a byte saying which kind, then what it
//! holds. A string, a list and a map hold a counted pointer and a length;
//! a module reads their bytes, items and fields in place, and counts them
//! in place. It asks the runtime only to make a value, to free one it held
//! last, and to open a value held some other way.
//!
//! The layout is the runtime's `Value` (`#[repr(u8)]`, its variants
//! numbered), Rust's `Rc` of a slice (a pointer to its two counts and its
//! contents, then a length) and a map's field (a key and a value). The
//! runtime measures its own as it loads a module and refuses one built to
//! another ([`Layout`], [`REFUSED_LAYOUT`]). 64-bit targets only.

use std::{ffi::c_void, mem::MaybeUninit};

/// This interface's number: a module and a runtime agree on it or the
/// module doesn't load. It changes only when something here is taken away
/// or changes meaning; what is added goes at the end of [`Api`] and
/// [`Registrar`], told by their `size`.
pub const ABI: u32 = 1;

/// The symbol a module exports ([`Entry`]).
pub const ENTRY: &[u8] = b"zgx_module_v1\0";

/// `nil`.
pub const NIL: u8 = 0;
/// A boolean: the byte at [`BOOL_AT`] is 0 or 1.
pub const BOOL: u8 = 1;
/// A number: an `f64` at [`HELD_AT`].
pub const NUM: u8 = 2;
/// A string: a counted pointer at [`HELD_AT`], its length in bytes after it.
pub const STR: u8 = 3;
/// A long string, held in pieces: [`Api::flat`] gives it as one.
pub const TEXT: u8 = 4;
/// A list: a counted pointer to its items (each a [`Raw`]) and how many.
pub const LIST: u8 = 5;
/// A map with string keys, sorted by key: a counted pointer to its fields
/// (each [`FIELD_SIZE`] bytes) and how many.
pub const MAP: u8 = 6;
/// A row kept folded until it is read: [`Api::open`] gives what it opens
/// to.
pub const FOLDED: u8 = 9;
// (Any other kind, 7, 8 or past 9: an element, an image, or one newer than
// this module. A module passes it on, clones and frees it
// through the runtime, and reads nothing of it.)

/// A value's size in bytes, and a list's stride.
pub const VALUE_SIZE: usize = 24;
/// Where a boolean's byte is in a value.
pub const BOOL_AT: usize = 1;
/// Where a value holds its number, or its counted pointer; a length is a
/// word after.
pub const HELD_AT: usize = 8;
/// From a counted pointer, where its count is: how many hold it.
pub const COUNT_AT: usize = 0;
/// From a counted pointer, where its contents begin: a string's UTF-8, a
/// list's items, a map's fields.
pub const DATA_AT: usize = 16;
/// A map's field's size in bytes, and a map's stride.
pub const FIELD_SIZE: usize = 40;
/// Where a field's key is: a counted pointer to its UTF-8, then its length.
pub const FIELD_KEY_AT: usize = 0;
/// Where a field's value is, a [`Raw`].
pub const FIELD_VALUE_AT: usize = 16;

/// The layout a runtime has, measured: each the constant of the same name
/// here, or the module doesn't load.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub value_size: u16,
    pub bool_at: u16,
    pub held_at: u16,
    pub count_at: u16,
    pub data_at: u16,
    pub field_size: u16,
    pub field_key_at: u16,
    pub field_value_at: u16,
}

/// The layout this crate reads values by.
pub const LAYOUT: Layout = Layout {
    value_size: VALUE_SIZE as u16,
    bool_at: BOOL_AT as u16,
    held_at: HELD_AT as u16,
    count_at: COUNT_AT as u16,
    data_at: DATA_AT as u16,
    field_size: FIELD_SIZE as u16,
    field_key_at: FIELD_KEY_AT as u16,
    field_value_at: FIELD_VALUE_AT as u16,
};

/// A value's three words. Owned (given back once, or handed to the
/// runtime) or lent (read while what lent it lives): which one is by where
/// it came from, said at each function.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct Raw(pub [MaybeUninit<u8>; VALUE_SIZE]);

/// A string's bytes, lent.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Bytes {
    pub ptr: *const u8,
    pub len: usize,
}

/// The runtime's functions for values, the same for every module and for
/// as long as the process runs. All are called on the thread the module
/// was loaded on (the interface's). Each writes what it makes to `out`,
/// owned.
#[repr(C)]
pub struct Api {
    /// [`ABI`].
    pub abi: u32,
    /// This struct's size in bytes, as the runtime built it: a module
    /// built against a longer one reads no further than this.
    pub size: u32,
    /// How this runtime lays values out.
    pub layout: Layout,
    /// A string of these bytes (UTF-8).
    pub string: unsafe extern "C" fn(ptr: *const u8, len: usize, out: *mut Raw),
    /// A list of these `len` owned values, which it takes.
    pub list: unsafe extern "C" fn(items: *mut Raw, len: usize, out: *mut Raw),
    /// A map of these `len` keys and owned values, which it takes, in any
    /// order (the last of a key given twice kept).
    pub map: unsafe extern "C" fn(keys: *const Bytes, values: *mut Raw, len: usize, out: *mut Raw),
    /// A list of a list's items `from` to `to` (from 0, `to` excluded):
    /// the items counted, not copied. `nil` if it isn't a list or they
    /// aren't in it.
    pub slice: unsafe extern "C" fn(list: *const Raw, from: usize, to: usize, out: *mut Raw),
    /// The same value once more: counted, not copied. (A string, a list
    /// and a map a module counts itself, at [`COUNT_AT`]; this is for the
    /// kinds it can't read.)
    pub clone: unsafe extern "C" fn(value: *const Raw, out: *mut Raw),
    /// Gives an owned value back: the last hold on a string, a list or a
    /// map (a module lets go of any other hold itself, at [`COUNT_AT`]),
    /// or a kind it can't read.
    pub release: unsafe extern "C" fn(value: *mut Raw),
    /// What a [`FOLDED`] value opens to, lent for as long as it is, kept
    /// open from then; null for any other.
    pub open: unsafe extern "C" fn(value: *const Raw) -> *const Raw,
    /// A [`TEXT`] value as a string of all its text (a copy); `nil` for any
    /// other.
    pub flat: unsafe extern "C" fn(value: *const Raw, out: *mut Raw),
}

/// A module's function, as `ui.call_sync(name, arg)` calls it: `arg` is
/// lent for the call (a function that keeps it clones it), and what it
/// writes to `out` is owned, the runtime's from then.
pub type Function = unsafe extern "C" fn(user: *mut c_void, arg: *const Raw, out: *mut Raw);
/// A module's action, as `ui.action(name, arg)` and `ui.invoke` call it:
/// `arg` lent, as a function's.
pub type Action = unsafe extern "C" fn(user: *mut c_void, arg: *const Raw);
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
/// loaded into. Returns 0, or a reason it can't load.
pub type Entry = unsafe extern "C" fn(api: *const Api, registrar: *const Registrar) -> u32;

/// The runtime speaks another [`ABI`], or a shorter [`Api`] than this
/// module needs.
pub const REFUSED_ABI: u32 = 1;
/// The runtime lays values out another way than this module reads them
/// ([`Layout`]).
pub const REFUSED_LAYOUT: u32 = 2;
