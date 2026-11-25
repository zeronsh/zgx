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

/// What the symbol of a module's entry ([`Entry`]) begins with: the
/// module's name follows (`zgx_module_v1_words`), so two modules linked
/// into one program are two symbols. A module's name is letters, digits
/// and `_`, and its manifest says it.
pub const ENTRY: &str = "zgx_module_v1_";

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
    // What follows is the interface's second part, the host's own reach:
    // a module's threads hand work to the interface's thread, and timers.
    // Each function is called on the interface's thread unless it says
    // otherwise.
    /// The host handle `host` once more (a [`Registrar::host_ref`]'s), to
    /// keep: any thread.
    pub host_retain: unsafe extern "C" fn(host: *mut c_void) -> *mut c_void,
    /// Lets go of a host handle: any thread.
    pub host_release: unsafe extern "C" fn(host: *mut c_void),
    /// Runs `job` on the host's thread, the interface's, at its next turn:
    /// any thread. It wakes the host as a value sent to it does, and nothing
    /// waits or polls while nothing is posted. The jobs of one turn run in
    /// the order posted, each in one reactive batch (what it sets updates
    /// once); each frees its `user` as it runs, and `free` frees one that
    /// doesn't run. [`POSTED`], or [`GONE`] once the host is: `free` is
    /// then called at once, on the caller's thread.
    pub post:
        unsafe extern "C" fn(host: *mut c_void, job: Job, user: *mut c_void, free: Free) -> u32,
    /// Calls `fire` after `ms` milliseconds on zgx's clock (a test's,
    /// devtools' frozen one, video's virtual one, else the wall's), then
    /// every `ms` while `repeat` is not 0 and `fire` returns not 0: a
    /// timer, to let go with [`timer_release`](Self::timer_release).
    /// `user` is freed as it ends. Null once the host is gone (`free` called
    /// at once).
    pub after: unsafe extern "C" fn(
        host: *mut c_void,
        ms: f64,
        repeat: u32,
        fire: Fire,
        user: *mut c_void,
        free: Free,
    ) -> *mut c_void,
    /// Lets go of a timer [`after`](Self::after) made, stopping it if
    /// `stop` is not 0 (else it runs on until it ends or its host goes).
    pub timer_release: unsafe extern "C" fn(timer: *mut c_void, stop: u32),
    /// A module's state for this thread, as [`Registrar::state`] gives it,
    /// asked by a host handle: from a job or a timer's call, after the
    /// module loaded.
    pub state: unsafe extern "C" fn(
        host: *mut c_void,
        name: *const u8,
        len: usize,
        size: usize,
        align: usize,
        init: unsafe extern "C" fn(state: *mut u8),
        free: unsafe extern "C" fn(state: *mut u8),
    ) -> *mut u8,
    /// The host's shared state `key` (`ScriptHost::shared`, `ui.shared`:
    /// the same one scripts and the host see), made with `initial` (taken)
    /// if there is none yet: a handle, to let go with
    /// [`shared_release`](Self::shared_release). Null once the host is gone.
    pub shared: unsafe extern "C" fn(
        host: *mut c_void,
        key: *const u8,
        len: usize,
        initial: *mut Raw,
    ) -> *mut c_void,
    /// What it holds, owned (counted, not copied); `nil` once its host is
    /// gone. Read inside a binding's call, it is followed as a script's
    /// `get` is.
    pub shared_get: unsafe extern "C" fn(shared: *const c_void, out: *mut Raw),
    /// Sets it to `value` (taken), keeping the parts of what it held that
    /// are equal (`Shared::set`): 1 if anything changed, 0 if not;
    /// [`HOST_GONE`] once its host is gone (nothing set).
    pub shared_set: unsafe extern "C" fn(shared: *const c_void, value: *mut Raw) -> u32,
    /// Adds `len` bytes of text (UTF-8) to the end of the string it holds,
    /// as a script's `state:append`: [`DONE`], or [`FAILED`] with why
    /// written to `out` (it holds something else, or its host is gone).
    pub shared_append: unsafe extern "C" fn(
        shared: *const c_void,
        text: *const u8,
        len: usize,
        out: *mut Raw,
    ) -> u32,
    /// Calls `call` each time what it holds changes, from now (not at
    /// once), on the interface's thread after the batch that changed it:
    /// a subscription, to let go with
    /// [`shared_unobserve`](Self::shared_unobserve), `user` freed then.
    /// Null once its host is gone (`free` called at once).
    pub shared_observe: unsafe extern "C" fn(
        shared: *const c_void,
        call: Observe,
        user: *mut c_void,
        free: Free,
    ) -> *mut c_void,
    /// Lets go of a subscription, which is never called again.
    pub shared_unobserve: unsafe extern "C" fn(subscription: *mut c_void),
    /// Lets go of a shared state's handle (the state stays: it is the
    /// host's).
    pub shared_release: unsafe extern "C" fn(shared: *mut c_void),
    /// Settles an async action's promise ([`AsyncAction`]): with `value`
    /// (taken) if `ok` is not 0, else rejected with `value`, a string of
    /// why (raised as the action's error, naming it and the module). Once,
    /// on the interface's thread; the promise is let go of with it. Settled
    /// after its caller is gone, it does nothing.
    pub settle: unsafe extern "C" fn(promise: *mut c_void, ok: u32, value: *mut Raw),
    /// Lets go of a promise unsettled: it rejects, saying the module
    /// dropped it. Any thread.
    pub promise_release: unsafe extern "C" fn(promise: *mut c_void),
    /// A handle to the host of the script waiting on `promise`, to keep
    /// (let go with [`host_release`](Self::host_release)): how a thread
    /// holding the promise posts its settling back. Any thread.
    pub promise_host: unsafe extern "C" fn(promise: *const c_void) -> *mut c_void,
    /// Writes `rows` (a list, taken) into the list the shared state holds,
    /// each row by its field `key` (a string, `key_len` bytes): a row whose
    /// key the list has replaces the first row of that key where it is,
    /// the others are added at the end in the order given, and of two rows
    /// given with one key the last is written. One pass over the list;
    /// only the rows given are new (`Shared::upsert`). [`DONE`], or
    /// [`FAILED`] with why in `out`, nothing written (a row with no string
    /// at `key`, or its host is gone).
    pub shared_upsert: unsafe extern "C" fn(
        shared: *const c_void,
        key: *const u8,
        key_len: usize,
        rows: *mut Raw,
        out: *mut Raw,
    ) -> u32,
    /// Removes the rows of the list it holds whose field `key` is one of
    /// the `len` strings `keys` (lent), every such row: 1 if any was, 0 if
    /// none; [`HOST_GONE`] once its host is gone.
    pub shared_remove: unsafe extern "C" fn(
        shared: *const c_void,
        key: *const u8,
        key_len: usize,
        keys: *const Bytes,
        len: usize,
    ) -> u32,
    /// A buffer for an image's pixels, `len` bytes, all 0: a handle, its
    /// bytes at `*data`, made as what the image shares (set, they aren't
    /// copied). Any thread; the module's to write, from one thread at a
    /// time, until it gives it to `image_set` or lets go of it with
    /// `image_drop`. NULL if `len` is 0 or more than `isize::MAX / 2`.
    pub image_buffer: unsafe extern "C" fn(len: usize, data: *mut *mut u8) -> *mut c_void,
    /// Sets the image `host:<name>` (scripts' `ui.image { src = … }`) to a
    /// `width` by `height` picture whose pixels are `buffer`'s (taken, on
    /// success or not): RGBA8, straight alpha, rows top to bottom, tightly
    /// packed (a row's stride is `width * 4`), so `width * height * 4`
    /// bytes. On the interface's thread; `name` is `<module>.<name>`, and
    /// what a module sets goes with its host, or as the module is loaded
    /// into it again. `DONE`, or `FAILED` with why in `out`, nothing set:
    /// another length, a size of 0, a name not so, or its host gone (as
    /// `post` says `GONE`; the buffer freed). On another thread, `FAILED`
    /// and `out` `nil` (no value is made off the interface's thread).
    pub image_set: unsafe extern "C" fn(
        host: *mut c_void,
        name: *const u8,
        len: usize,
        width: u32,
        height: u32,
        buffer: *mut c_void,
        out: *mut Raw,
    ) -> u32,
    /// Lets go of a buffer not set: any thread.
    pub image_drop: unsafe extern "C" fn(buffer: *mut c_void),
    /// Removes the image `host:<name>` a module set: not 0 if there was one.
    /// On the interface's thread (on another, nothing).
    pub image_remove: unsafe extern "C" fn(host: *mut c_void, name: *const u8, len: usize) -> u32,
    /// [`Api::shared_upsert`] with a place for each row: `places` (a list,
    /// taken) as long as `rows`, each `nil` (where its key's row is, or at
    /// the end if the list has none), a string (just before the row with
    /// that key: the list's first of it, or one given, wherever that goes)
    /// or `true` (at the end). A row the list has moves to its place; rows
    /// placed before one key go in the order given; one walk of the list.
    /// [`DONE`], or [`FAILED`] with why in `out`, nothing written: a row
    /// placed before itself, before a key neither the list nor the rows
    /// have, rows placed before one another (`a` before `b` before `a`),
    /// what `shared_upsert` refuses, or its host gone.
    pub shared_upsert_at: unsafe extern "C" fn(
        shared: *const c_void,
        key: *const u8,
        key_len: usize,
        rows: *mut Raw,
        places: *mut Raw,
        out: *mut Raw,
    ) -> u32,
}

/// A shared state's observer ([`Api::shared_observe`]): `value` is what it
/// holds now, lent for the call; `host` the state's host handle, lent.
pub type Observe = unsafe extern "C" fn(user: *mut c_void, host: *mut c_void, value: *const Raw);

/// What [`Api::post`] returns when the job will run.
pub const POSTED: u32 = 1;
/// What [`Api::post`] returns when the host is gone: the job won't run.
pub const GONE: u32 = 0;
/// What [`Api::shared_set`] and [`Api::shared_remove`] return when the
/// state's host is gone, nothing written: not a count, so told from one.
/// (The writes that return `DONE` or `FAILED` fail then, saying so.)
pub const HOST_GONE: u32 = u32::MAX;

/// A job [`Api::post`] runs on the interface's thread: `host` is the host
/// handle it was posted to, lent for the call. It runs once, and frees its
/// own `user` as it does (the post's `free` is for a job that never runs:
/// its host went first).
pub type Job = unsafe extern "C" fn(user: *mut c_void, host: *mut c_void);
/// A timer's call ([`Api::after`]), on the interface's thread, `host` lent:
/// a repeating timer goes on while it returns not 0.
pub type Fire = unsafe extern "C" fn(user: *mut c_void, host: *mut c_void) -> u32;

/// What a call returns when it did what it was asked.
pub const DONE: u32 = 0;
/// What a call returns when it couldn't (an argument of the wrong kind): it
/// wrote why to `out`, a string, where its result would have gone, and the
/// runtime raises that in the script that called, naming the module and the
/// function. Any value but [`DONE`] is read as this.
pub const FAILED: u32 = 1;

/// A module's function, as `ui.call_sync(name, arg)` calls it, and as a
/// script calls it by name on the module's table (`ui.module`), with any
/// number of arguments: `args` are `len` values, lent for the call (a
/// function that keeps one clones it). There is always one to read: called
/// with none, it is lent a `nil` (and `len` is 0). What it writes to `out`
/// is owned, the runtime's from then, and it always writes it: its result
/// and [`DONE`], or why it failed and [`FAILED`].
pub type Function =
    unsafe extern "C" fn(user: *mut c_void, args: *const Raw, len: usize, out: *mut Raw) -> u32;
/// A function handed its argument, the first (any after it the runtime
/// frees): `arg` is owned by the function, which frees it or hands it on
/// (what it writes to `out` may be `arg` itself). For one that keeps or
/// returns what it is given; any other is cheaper lent its arguments
/// ([`Function`]), which the runtime then frees in place.
pub type FunctionTaking =
    unsafe extern "C" fn(user: *mut c_void, arg: *mut Raw, out: *mut Raw) -> u32;
/// A module's action, as `ui.action(name, arg)` and `ui.invoke` call it:
/// `arg` lent, as a function's. It writes `out` only when it fails (why,
/// and [`FAILED`]).
pub type Action = unsafe extern "C" fn(user: *mut c_void, arg: *const Raw, out: *mut Raw) -> u32;
/// Frees a registration's `user` when the host that held it goes.
pub type Free = unsafe extern "C" fn(user: *mut c_void);
/// A module's async action, as `ui.call(name, arg)` calls it (and a script
/// by name on the module's table): `arg` lent for the call, as a
/// function's. It returns at once; the script holds a promise, which the
/// module settles later ([`Api::settle`], on the interface's thread) or
/// lets go of ([`Api::promise_release`], any thread), exactly one of the
/// two. `promise` may be sent to another thread to wait there.
pub type AsyncAction =
    unsafe extern "C" fn(user: *mut c_void, arg: *const Raw, promise: *mut c_void);

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
    // The interface's second part ([`Api::host_retain`] on).
    /// A handle to the host being loaded into, kept past the entry call:
    /// what [`Api::post`] and [`Api::after`] take, and what a module's
    /// threads hand work to the host by. Let go with
    /// [`Api::host_release`]. A host whose module asks for none keeps
    /// nothing for it.
    pub host_ref: unsafe extern "C" fn(host: *mut c_void) -> *mut c_void,
    /// Registers an async action scripts call with `ui.call(name, arg)`,
    /// getting a promise of its result.
    pub async_action: unsafe extern "C" fn(
        host: *mut c_void,
        name: *const u8,
        len: usize,
        call: AsyncAction,
        user: *mut c_void,
        free: Free,
    ),
    // The interface's third part (a registrar this long has it).
    /// Registers a line highlighter a textarea calls itself
    /// (`ui.highlighter(name, styles)`), with no script between: `kinds`
    /// names what its spans' kinds are, by their number.
    pub highlighter: unsafe extern "C" fn(
        host: *mut c_void,
        name: *const u8,
        len: usize,
        kinds: *const Bytes,
        count: usize,
        call: Highlight,
        user: *mut c_void,
        free: Free,
    ),
}

/// A line highlighter's call: `line`'s spans, each handed to `push` as
/// bytes from 1 (both ends in) and a kind, from the state the line before
/// left (0: nothing open); returns what this line leaves.
pub type Highlight = unsafe extern "C" fn(
    user: *mut c_void,
    line: *const u8,
    len: usize,
    state: u32,
    sink: *mut c_void,
    push: Push,
) -> u32;
/// Where a highlighter hands each span.
pub type Push = unsafe extern "C" fn(sink: *mut c_void, from: u32, to: u32, kind: u32);
/// What a highlighter returns when it failed (a Rust one, as it panicked):
/// the runtime turns it off for that textarea and says so. A line's state is
/// below [`HIGHLIGHT_STATES`].
pub const HIGHLIGHT_FAILED: u32 = u32::MAX;
/// Every state a highlighter leaves is below this (the runtime keeps a
/// textarea's states one up, below 2^31); one at or past it is a failure.
pub const HIGHLIGHT_STATES: u32 = (1 << 31) - 1;

/// A module's entry, exported as [`ENTRY`]: called once per host it is
/// loaded into. Returns 0, or a reason it can't load.
pub type Entry = unsafe extern "C" fn(api: *const Api, registrar: *const Registrar) -> u32;

/// The runtime speaks another [`ABI`], or a shorter [`Api`] than this
/// module needs.
pub const REFUSED_ABI: u32 = 1;
/// The module's setup panicked as it loaded: nothing of it is registered.
pub const SETUP_PANICKED: u32 = 2;

/// What the symbol of a module's manifest begins with, its name following
/// as [`ENTRY`]'s. A manifest says what the module is called, what it
/// registers and with which types, as JSON (docs/modules.md has its
/// grammar). Data, not a function, all in one piece with no pointer in it,
/// so it reads the same in the library's file as in memory:
///
/// | bytes | |
/// | --- | --- |
/// | 16 | [`MANIFEST_MAGIC`] |
/// | 4 | [`MANIFEST_FORMAT`], little-endian |
/// | 4 | the JSON's length in bytes, little-endian |
/// | 4 | [`manifest_sum`] of the JSON, little-endian |
/// | 4 | zero |
/// | its length | the JSON, UTF-8 |
/// | 16 | [`MANIFEST_END`] |
///
/// Every module has one: it is how the runtime learns a library's name, and
/// so its symbols. The runtime finds it in the library's file (the magic,
/// then the first place where the whole of the above holds), loads the
/// library, reads the manifest again through this symbol, and refuses a
/// module where the two differ or that registers anything else than it
/// says. Tools read it from the file the same way, without running the
/// module. A manifest that lists no functions (one registering by hand,
/// `register!`) is held to nothing but its name, and nothing is known of
/// its types.
pub const MANIFEST: &str = "zgx_module_manifest_";
/// What a manifest begins with.
pub const MANIFEST_MAGIC: [u8; 16] = *b"zgx-module-json\0";
/// What a manifest ends with.
pub const MANIFEST_END: [u8; 16] = *b"\0zgx-module-end\0";
/// The manifest format's own number (and `"zgx_module"` in the JSON).
pub const MANIFEST_FORMAT: u32 = 1;
/// How many bytes come before a manifest's JSON.
pub const MANIFEST_HEAD: usize = 32;

/// A manifest's checksum of its JSON: FNV-1a, 32 bits.
pub const fn manifest_sum(json: &[u8]) -> u32 {
    let mut sum: u32 = 0x811c_9dc5;
    let mut i = 0;
    while i < json.len() {
        sum = (sum ^ json[i] as u32).wrapping_mul(0x0100_0193);
        i += 1;
    }
    sum
}

/// A manifest of `json`, as a module's symbol holds it ([`MANIFEST`]): `N`
/// is the JSON's length and 48 (`zgx_module::module` and `register!` write
/// this for a module).
///
/// # Panics
/// If `N` isn't that.
pub const fn manifest<const N: usize>(json: &str) -> [u8; N] {
    let json = json.as_bytes();
    assert!(N == MANIFEST_HEAD + json.len() + MANIFEST_END.len());
    assert!(json.len() <= u32::MAX as usize);
    let mut out = [0u8; N];
    let mut at = 0;
    while at < 16 {
        out[at] = MANIFEST_MAGIC[at];
        out[N - 16 + at] = MANIFEST_END[at];
        at += 1;
    }
    let words = [MANIFEST_FORMAT, json.len() as u32, manifest_sum(json)];
    let mut w = 0;
    while w < 3 {
        let bytes = words[w].to_le_bytes();
        let mut b = 0;
        while b < 4 {
            out[16 + w * 4 + b] = bytes[b];
            b += 1;
        }
        w += 1;
    }
    let mut i = 0;
    while i < json.len() {
        out[MANIFEST_HEAD + i] = json[i];
        i += 1;
    }
    out
}

/// A manifest of `json`, as [`manifest`] makes one: for a tool that writes
/// a module's manifest, and for tests.
pub fn manifest_bytes(json: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(MANIFEST_HEAD + json.len() + 16);
    out.extend_from_slice(&MANIFEST_MAGIC);
    for word in [
        MANIFEST_FORMAT,
        json.len() as u32,
        manifest_sum(json.as_bytes()),
    ] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(json.as_bytes());
    out.extend_from_slice(&MANIFEST_END);
    out
}

/// The JSON of the first manifest in `bytes` (a library's file, or what
/// its manifest's symbol points at), if one is whole there: its magic, this format,
/// a length inside `bytes`, the checksum and the end all as they should be.
pub fn manifest_in(bytes: &[u8]) -> Option<&str> {
    let word = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let mut from = 0;
    while let Some(found) = find(&bytes[from..], &MANIFEST_MAGIC) {
        let at = from + found;
        from = at + 1;
        let whole = || {
            let (format, len, sum) = (word(at + 16)?, word(at + 20)? as usize, word(at + 24)?);
            let json = bytes.get(at + MANIFEST_HEAD..at + MANIFEST_HEAD + len)?;
            let end = bytes.get(at + MANIFEST_HEAD + len..at + MANIFEST_HEAD + len + 16)?;
            (format == MANIFEST_FORMAT && end == MANIFEST_END && manifest_sum(json) == sum)
                .then(|| std::str::from_utf8(json).ok())?
        };
        if let Some(json) = whole() {
            return Some(json);
        }
    }
    None
}
fn find(bytes: &[u8], what: &[u8; 16]) -> Option<usize> {
    // (By its first byte, then the rest: a library is megabytes, and its
    // manifest is looked for once as it loads.)
    let mut from = 0;
    while let Some(at) = bytes[from..].iter().position(|b| *b == what[0]) {
        let at = from + at;
        if bytes[at..].starts_with(what) {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// Whether `name` can be a module's: letters, digits and `_`, and not
/// empty.
pub const fn is_name(name: &str) -> bool {
    let name = name.as_bytes();
    let mut i = 0;
    while i < name.len() {
        if !(name[i].is_ascii_alphanumeric() || name[i] == b'_') {
            return false;
        }
        i += 1;
    }
    !name.is_empty()
}
