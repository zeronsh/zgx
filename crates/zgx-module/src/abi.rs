//! What crosses between a module and the runtime: C functions, and the
//! layout of a value, which a module reads where the runtime holds it.
//!
//! A value is three words ([`Raw`]): a byte saying which kind, then what it
//! holds. A string holds a pointer to its text and its length; a list and
//! a map hold a pointer to their items, their count before them, and how
//! many, as does a string its text. A module reads text, items and fields
//! in place, and counts a string, a list or a map in place. It asks the
//! runtime to make a value, to free one it held last, and to open a value
//! held some other way.
//!
//! Every part of that layout is stated in zgx-script's own types (`Value`
//! is `#[repr(u8)]`; its string, its counted slice and a map's field are
//! `#[repr(C)]`), none left to the compiler, and zgx-script fails to build
//! if they stop matching the numbers here. 64-bit targets only.

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
/// A string: a pointer to its UTF-8 at [`HELD_AT`], its length in bytes
/// after it; its count before its text, as a list's before its items.
pub const STR: u8 = 3;
/// A long string, held in pieces: [`Api::flat`] gives it as one.
pub const TEXT: u8 = 4;
/// A list: a pointer to its items (each a [`Raw`]) and how many.
pub const LIST: u8 = 5;
/// A map with string keys, sorted by key: a pointer to its fields (each
/// [`FIELD_SIZE`] bytes) and how many.
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
/// Where a value holds its number, or its pointer; a length is a word
/// after.
pub const HELD_AT: usize = 8;
/// How far before a string's text, a list's items or a map's fields its
/// count is: how many hold it.
pub const COUNT_BEFORE: usize = 16;
/// A map's field's size in bytes, and a map's stride.
pub const FIELD_SIZE: usize = 40;
/// Where a field's key is: a pointer to its UTF-8, then its length.
pub const FIELD_KEY_AT: usize = 0;
/// Where a field's value is, a [`Raw`].
pub const FIELD_VALUE_AT: usize = 16;

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
    /// The same value once more: counted, not copied. (A string, a list and
    /// a map a module counts itself, at [`COUNT_BEFORE`]; this is for the
    /// kinds it can't read.)
    pub clone: unsafe extern "C" fn(value: *const Raw, out: *mut Raw),
    /// Gives an owned value back: the last hold on a string, a list or a
    /// map (a module lets go of any other hold itself, at [`COUNT_BEFORE`]),
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
/// A function handed its argument: `arg` is owned by the function, which
/// frees it or hands it on (what it writes to `out` may be `arg` itself).
/// For one that keeps or returns what it is given; any other is cheaper
/// lent its argument ([`Function`]), which the runtime then frees in place.
pub type FunctionTaking = unsafe extern "C" fn(user: *mut c_void, arg: *mut Raw, out: *mut Raw);
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
    /// Registers a function scripts call with `ui.call_sync(name, arg)`,
    /// handed its argument.
    pub function_taking: unsafe extern "C" fn(
        host: *mut c_void,
        name: *const u8,
        len: usize,
        call: FunctionTaking,
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
    /// A module's state for this thread, by a name of the module's own:
    /// `size` bytes at `align`, made the first time the thread asks (`init`
    /// writes it) and the same pointer each time after, for every host on
    /// the thread; freed (`free`) as the thread ends. A module asks once as
    /// it loads and keeps the pointer, so reading its state is a load.
    pub state: unsafe extern "C" fn(
        host: *mut c_void,
        name: *const u8,
        len: usize,
        size: usize,
        align: usize,
        init: unsafe extern "C" fn(state: *mut u8),
        free: unsafe extern "C" fn(state: *mut u8),
    ) -> *mut u8,
}

/// A module's entry, exported as [`ENTRY`]: called once per host it is
/// loaded into. Returns 0, or a reason it can't load.
pub type Entry = unsafe extern "C" fn(api: *const Api, registrar: *const Registrar) -> u32;

/// The runtime speaks another [`ABI`], or a shorter [`Api`] than this
/// module needs.
pub const REFUSED_ABI: u32 = 1;
