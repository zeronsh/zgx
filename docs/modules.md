# Native modules

> **Unstable before 1.0.** How a module is written (`zgx-module` and its macros), its manifest and the interface between it and the runtime may change in any release until then.

A native module is an app's own Rust, built apart from zgx as a library the runtime loads, whose functions the app's scripts call. A change to a module rebuilds that module alone: no build of the app, none of zgx. Use one for what an app does in Rust that it changes often: its model code, parsing, the work behind a view. The interface (windows, state, layout) stays in Luau.

```rust
#[zgx_module::module(name = "words")]
mod words {
    use zgx_module::export;

    /// How many words `text` has.
    #[export]
    fn count(text: &str) -> f64 {
        text.split_whitespace().count() as f64
    }
}
```

```lua
local words = ui.module("words")
ui.text { `{words.count(draft:get())} words` }
```

Only the app loads modules, from its Rust (`ScriptHost::load_module`). No script can load one; every script calls its functions. Modules are for desktop builds: iOS, Android and the web load no libraries at run time.

The design, and what each part costs, is in [design/runtime.md](design/runtime.md).

## A module's crate

A module is a crate of its own, built as a `cdylib`, in a cargo workspace of its own so nothing of the app is in its build:

```toml
[workspace]

[package]
name = "words"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
zgx-module = "0.2"

[profile.release]
lto = "thin"
codegen-units = 1
```

`zgx-module` brings the macros (`zgx-module-macros`) and nothing of zgx itself. A module built against one version of the interface (`zgx_module::abi::ABI`) loads in any runtime that speaks it.

## Writing one

`#[module(name = "…")]` goes on a `mod`. Each function in it marked `#[export]` is registered for scripts: as `<module>.<function>` by name, and as a member of the module's table. The name is letters, digits and `_`, and is what scripts ask for (`ui.module("words")`).

A function takes the types below and returns one of them. What a script passes is read as the function's types say, where the runtime holds it: a `&str` is the runtime's own text, a `List`'s items and a record's borrowed fields are read in place, and nothing is copied unless a type says so (`String`, `Vec`, the maps).

| Rust argument | What a script passes | Luau type |
| --- | --- | --- |
| `bool` | a boolean | `boolean` |
| `f64`, `f32` | a number | `number` |
| `i8` … `i64`, `u8` … `u64`, `isize`, `usize` | a whole number the type holds | `number` |
| `&str` | a string up to 64 KiB, lent | `string` |
| `Text` | a string of any length (past 64 KiB the runtime holds one in pieces, made one for the call) | `string` |
| `String` | a string, copied | `string` |
| `Option<T>` | `nil` (or nothing passed), or a `T` | `T?` |
| `List<T>` | a list whose items are all `T`s, lent | `{ T }` |
| `Vec<T>` | the same, copied | `{ T }` |
| `Map<T>` | a table of string keys and `T` values, lent | `{ [string]: T }` |
| `HashMap<String, T>`, `BTreeMap<String, T>` | the same, copied | `{ [string]: T }` |
| a struct deriving `Record` | a table with its fields | the record's type |
| `Ref` | anything, lent, read by hand (`.str()`, `.field("id")`, `.items()`) | `any` |
| `Value` | anything, held (counted, not copied) | `any` |

A function returns any of these too, or `()`, `&str`, `&[T]` or a `Value` it made (`Value::list`, `Value::record`).

### Records

```rust
#[derive(Record)]
pub struct Query<'a> {
    pub id: &'a str,       // borrowed: read where the runtime holds it
    pub limit: Option<u32>,
}

#[derive(Record)]
pub struct Found {
    pub id: String,
    pub tags: Vec<String>,
}

#[export]
fn find(query: Query) -> Option<Found> { … }
```

A record is a table of its fields by name. A record with a lifetime borrows its fields from the argument. Records are declared inside the module's `mod`: its manifest names them.

### Errors

An argument of the wrong kind is an error raised in the script that called, at its line, naming the function, the module, the argument and what it must be:

```
app.luau:12: words.count (module `words`): argument 1 (text) must be a string, not a number
```

A field of a record, and an item of a list, are named the same way:

```
argument 1 (query) field `limit` must be a whole number from 0 to 4294967295 or nil, not 1.5
```

A function returning `Result<T, E>` fails when it returns `Err`: the error is raised the same way, the function's own message after its name. `E` is `zgx_module::Failed` or anything that becomes one (`String`, `&str`). Say what was wrong with what the script passed, as the end of a sentence: `Err("has no row 12".into())`.

A panic in a module's function or action doesn't end the app: it is caught where the runtime called the module, and raised in the script that called as an error the function returned would be, its message after `panicked:`. The module stays loaded, its state as the panic left it (a `#[state]` borrowed by the call is let go). An async action that panics rejects its promise with the same message. A panic as the module loads (in `register!`'s setup) refuses it: nothing of it is registered, and `load_module` says so. A posted job, a timer, an observer or a drop that panics is caught where the runtime called it and printed, with no script to raise it in: the jobs and timers after it run, a repeating timer that panics stops, and an observer goes on observing. The one panic that still ends the process is a module state's `Default` (`#[state]`, `Host::state`): there is no state to go on with. Return an error instead all the same: a panic is printed as any is, and leaves whatever it was doing half done.

### Actions

`#[export(action)]` registers an action instead: one argument, nothing returned (or `Result<(), E>`), bound with `ui.action(name, arg)` or called with `ui.invoke(name, arg)`. A failing action bound to a control is reported as a handler's error is.

`#[export(name = "chat.reply_run")]` registers a function or an action under a name of its own: an app moving existing host functions into a module keeps their names, and the scripts that call them by name don't change.

### Async actions

`#[export(async)]` registers an async action: a script calls it with `ui.call(name, arg)`, or as a member of the module's table, and gets a `Promise`. Its first parameter (after its state) is the `Promise<T>` it settles, then one argument at most; it returns at once, and settles the promise later, on the interface's thread:

```rust
/// The page `id` is on, read on a thread of the module's.
#[export(async)]
fn load(promise: Promise<Page>, id: &str) {
    let id = id.to_owned();
    std::thread::spawn(move || {
        let read = read_page(&id); // a `String` and numbers: they may cross threads
        promise.resolve_with(move |_host| read.map(Page::from).map_err(Failed::from));
    });
}
```

`resolve_with` posts its closure to the interface's thread, which makes the result there (a value is that thread's) and settles the promise with it. On that thread, `promise.resolve(host, value)` and `promise.reject(host, why)` settle it at once; `promise.ui()` is a handle to post by. A promise is `Send`, so a thread may hold it while it works. A rejection is raised in the script that waits, naming the action and the module (`load (module `pages`): has no page 3`); a promise dropped unsettled rejects, saying the module dropped it; one settled after its script's host is gone does nothing. `host.async_action(name, |arg, promise| …)` registers one by hand. `zgx types` types its member `(id: string) -> Promise<PagesPage>`.

### State

A module's state is one per thread, kept by the runtime. A first parameter marked `#[state]` is the module's state of that type (`Default`), lent for the call:

```rust
#[derive(Default)]
pub struct Kept { last: zgx_module::Value }

#[export(action)]
fn keep(#[state] kept: &mut Kept, value: Ref) {
    kept.last = value.to_owned();
}

#[export]
fn last(#[state] kept: &Kept) -> Value {
    kept.last.clone()
}
```

Every host on a thread shares it; a host on another thread has its own. Reading it is a load, not a lookup.

### Threads

Every call into a module is on the thread that loaded it (the interface's), and values are counted without atomics: a value is made, read and kept on that thread only. A module's own threads reach the host through a `Ui`, a handle that is `Send`, and post jobs to it, which run on the interface's thread:

```rust
zgx_module::register!("feed", |host: &Host| {
    let ui = host.ui();
    std::thread::spawn(move || {
        for line in lines_from_somewhere() {
            // `line` is a `String`, which may cross threads; a value isn't.
            ui.post(move |host: &Host| {
                host.state::<Feed>().with(|feed| feed.push(Value::from(line.as_str())));
            });
        }
    });
});
```

A `&Host` is how a module knows it is on the interface's thread: setup is lent one, and so is each job and each timer's call. `Host` isn't `Send`, so a value never leaves that thread by mistake. A job runs at the host's next turn, in the order posted, in one reactive batch (what it sets updates once); `post` wakes the host only as it posts, and a host whose module holds a handle costs nothing at rest. Once the host is gone, `post` returns false and the job is dropped unrun.

### Shared state

`host.shared(key, initial)` is the host's shared state `key`: the one scripts read with `ui.shared(key)` and the host with `ScriptHost::shared`, made with `initial` if there is none yet. A module publishes its model there, and scripts show it and follow it:

```rust
let feed = host.shared("chat.items", Value::NIL).unwrap();
// …later, in a job a thread posted:
feed.set(Value::list(rows))?;     // only what changed costs anything
```

`set` keeps the parts of what it held that are equal (publish a whole model each time), and says whether anything changed. A long list needn't be published whole: diff it on the module's thread and post the rows that changed, written by key:

```rust
// …in a job a thread posted, with only what changed:
feed.upsert("id", changed)?;          // replace by `id` in place, add the new at the end
feed.remove("id", ["c-41", "c-7"])?;  // every row with one of these ids
```

`upsert(key, rows)` takes rows that each have a string at `key`. A row whose key the list has replaces the first row with that key, where it is, keeping its equal parts; the others are added at the end, in order; of two given with one key, the last is written. The list is walked once (a batch of 500 rows into 10,000 is one walk, not 500), and written in place when nothing else holds it. A state holding `nil` becomes a list of the rows; one holding anything else, or a row without its key, is an error, and nothing is written. `remove(key, keys)` removes every row whose key is one of `keys`, and says whether any was.

A list whose order changes (a row made lands at its sorted place, one moves to the top) needn't go whole either. `upsert_at(key, rows)` gives each row a `Place`: `Before(key)` just before that key's row, `End` at the end, or `Stay` where its key's row is (as `upsert`). A row the list has moves to its place, so a module that diffs a reordered list posts only the rows that moved, each placed before the row that follows it:

```rust
use zgx_module::Place;
feed.upsert_at("id", [
    (made, Place::Before("c-12")),     // a chat made, at its sorted place
    (bumped, Place::Before("c-3")),    // one moved up
])?;
```

A row placed before another goes wherever that one goes, after the rows placed before it in turn; rows placed before one key go in the order given. The list is walked once. A row placed before itself, before a key neither the list nor the rows have, or rows placed before one another (`a` before `b` before `a`) are an error, and nothing is written.

`append(text)` adds to the end of a string, as a script's `state:append` does. `get` counts what it holds, copying nothing; read while a binding asks the module (`ui.call_sync` in a binding's function), it is followed as a script's `get` is. `observe(|host, now| …)` calls on the interface's thread each time it changes, from the next change on, until the `Observer` is dropped: how a module hears what scripts write. A handle is the interface thread's (`Shared` isn't `Send`). Once its host is gone it holds `nil`, and every write (`set`, `append`, `upsert`, `upsert_at`, `remove`) is an error saying its host is gone, so a module can tell that what it wrote went nowhere.

### Images

A module that decodes or draws a picture shows it by name, as `host:<name>` (`ui.image { src = "host:words.cover" }`, `bg_image = "host:words.cover"`). It writes the pixels on any thread into an `ImageBuffer`, whose bytes are allocated as what the image keeps, and sets it from a job, so nothing is copied on the way:

```rust
let ui = host.ui();
std::thread::spawn(move || {
    let mut cover = ImageBuffer::new(width, height).unwrap();
    decode_into(cover.pixels());             // width * height * 4 bytes
    ui.post(move |host| host.image("words.cover", cover).unwrap());
});
```

The pixels are RGBA8: red, green, blue and alpha a byte each, straight (not premultiplied), rows top to bottom with nothing between them (a row is `width * 4` bytes). A buffer starts all 0. It is the module's to write, from one thread at a time, until it is given to `host.image` (on the interface's thread) or dropped (anywhere). An image a module sets is named `<module>.<name>`, as its functions are; `host.remove_image(name)` removes it. What a module sets goes with its host, and as the module is loaded into its host again, so nothing is left from its last load. (`zgx run` restarts the app when a module changes, a new process.) From C, `image_buffer`, `image_set`, `image_drop` and `image_remove` are the same, and `image_set` checks the buffer's length against `width * height * 4`.

### Timers

`host.after(delay, |host| …)` calls once, `host.every(period, |host| …)` every period, on the interface's thread and on zgx's clock: a test's (its frames move it), devtools' frozen one, video's virtual one, else the wall's. Each returns a `Timer`, which stops it when dropped (`forget` lets it run on until its host goes). A module's state is reached from a job or a timer's call as from setup, `host.state::<T>()`.

## Loading one

```rust
// SAFETY: The app's own module, built with it.
unsafe { host.load_module("modules/words/target/release/libwords.so") }?;
```

`load_module` reads the module's manifest from its file, loads the library, reads the manifest again from the library and holds the module to it: a module that registers anything else than its manifest says, or whose manifest is damaged, is refused, and nothing of it is registered. The library stays loaded as long as the process runs.

A module can be linked into the app instead (one crate built as an `rlib` too), and registered by its entry and manifest: `host.enter_module(words::zgx_module_v1, &words::zgx_module_manifest)`. It answers the same.

`zgx.json` names an app's modules, by the folder of their crate (the newest library built under its `target/` is read) or by a library's path:

```json
{ "modules": { "words": "modules/words" } }
```

`zgx-run` loads them from there before the app's scripts run (`zgx-run --module words=<library>` hands it another library for one). A module `zgx.json` names that has no built library stops `zgx-run` before its window opens, naming it; one that won't load (built for another interface, or not what its manifest says) is shown in the error banner, the scripts' `ui.module` saying why. `zgx types` and `zgx check` read the same key.

### `zgx run`

`zgx run my-app` builds each module whose sources changed, then starts the app with them. A module's **key** is a SHA-256 over every file in its crate's folder (but `target/` and hidden ones), the compiler, the target, the profile and the interface's number; a library built for a key is kept in the cache (`~/.cache/zgx/modules/<name>/<key>/`, or `ZGX_CACHE`) and taken from there whenever that key comes up again, so an unchanged module is never built twice, for any app or checkout. A miss runs cargo with a target directory of the module's own in the cache, so a rebuild compiles only what changed. `--release` builds them for release.

While the app runs, Luau files reload as always. When a module's sources change, `zgx run` builds it again and, since a loaded library can't be replaced, starts the app again: "module `words` changed, restarting" on the terminal, and in the restarted app's banner. A change that doesn't build leaves the app running as it was, cargo's errors on the terminal.

The key can't see a path dependency outside the module's folder: keep those inside it, or depend by version or git, which `Cargo.lock` pins.

### `zgx build`

`zgx build my-app` writes the app as a bundle, `dist/<app_id>/` (or `--out`):

```text
dist/my-app/
  manifest.json            the app's id and version, every hash
  ui/                      the app's folder: .luau as bytecode, themes, images and the rest as they are
  modules/<platform>/      each module's library: x86_64-linux, aarch64-macos…
```

Each module is built for release for each `--target` (the host's by default) where its key is new, and taken from the cache otherwise, so a release in which only Luau changed compiles no Rust. `zgx-run dist/my-app` runs it: the scripts are read from `ui/` as they are asked for, as the build compiled them. `--version` sets the bundle's version over `zgx.json`'s `version`.

`ui/` holds the files of the kinds a host reads (scripts, `.json`, images, models, `.bin`, `.luaurc`) and the licences beside them (`LICENSE*`, `NOTICE*`, `COPYING*`, and the files of a folder called so). Hidden files and folders stay out, and so does anything else; `zgx build` lists what it left out, so a file your app's own Rust reads is missed at the build, not where it runs.

Luau checks no bytecode it loads, so a bundle is trusted as native code is: zgx-run runs one a person or the app's install names, and nothing a script names. It is checked against its manifest, which holds each file's SHA-256: each module's library as the bundle is opened, every launch (it is native code, loaded then), and each file of `ui/` as it is read, so the bytes checked are the bytes run and a launch hashes only what it loads. zgx-run reads and checks the scripts ahead on worker threads while its window is made (`Opened::read_ahead`, as `compile_ahead` compiles an app's sources), and a load takes the bytes a worker checked: no script is read or hashed on the thread that loads, if it was ready in time. A library that isn't the one its manifest names stops zgx-run before a window opens; a script that isn't (changed since the build), or one the manifest doesn't name, fails to load as a missing or broken script does, with the reason, and nothing of it runs.

The manifest isn't signed, and sits beside what it names: its hashes catch damage and mistakes, not a person or a program that can write both. What an app's package is signed with (macOS's code signature over the `.app`) covers both.

`zgx build --ui source` keeps the scripts as source, for an app whose files people lay their own over or edit where it is installed (templates, themes). Every file of the app's folder is kept, hidden ones too (but `.git`, `.zgx` and `.DS_Store`), where bytecode keeps the kinds a host reads; a link in it to a folder, or out of it, fails the build. **So whatever is left in the folder ships**: an editor's file, a test, a password, and every folder: keep `target/`, `node_modules/` and other build output out of it (a module's own crate, which zgx.json names, is left out). A file that looks like a secret (`.env*` but `.env.example`, `.npmrc`, `.netrc`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `id_rsa`, `id_ed25519`, `id_ecdsa` and `id_dsa` with an extension or without) fails the build, and so does a folder of such a name, unless `--allow <file>` names it: its path in the app's folder, or for a package's file its path in the bundle, `.packages/<name>/…`, as the error says it. An `--allow` for nothing that ships fails too. That knows names, not contents: a key in a `.json` goes out with the rest, so keep them out of the folder. The manifest says `"ui": { "source": true }` and has no hashes of `ui/`, which is run as any app's folder is: compiled and checked by Luau as it loads, reloaded as it changes where its host watches, and never read as bytecode. A script that doesn't parse still fails the build, and the modules are built, cached, named and checked as in any bundle. A Rust host opens either kind with `app_bundle::open`: `Opened::files()` is the checked files, or `None` for source, whose `ui` it runs with `ScriptHost::new`. A runtime from before source bundles refuses one; this one opens both kinds.

**Nothing hashes or signs a source `ui/`**: zgx checks none of it, at the build or as it runs. What zgx-run does hold is that its scripts are that folder's own: the first script, a named window's, its packages' and every module required are files in `ui/`, and a path with `..` or a symbolic link out of it is refused. (Scripts `zgx.json` names under `scripts` are where it says; images and other files a script names are data, read as in any app.) That keeps a mistake from running another app's files; it is no defence against whoever can write to `ui/`, who can write the code there. What protects the folder is the platform's:

| | What seals `ui/` |
| --- | --- |
| macOS | the `.app`'s code signature, where `ui/` is in `Contents/Resources`: a changed file breaks the signature, which Gatekeeper checks when a downloaded app is first opened. zgx doesn't check it again as it reads |
| Windows | nothing, in a folder a plain installer writes (a signed executable doesn't cover the files beside it) |
| Linux | nothing but the folder's owner and permissions (a person's own, for an install in their home) |

## Calling one from Luau

`ui.module(name)` gives a loaded module's functions as a table, each called with its own arguments:

```lua
local words = ui.module("words")
local n = words.count(text)
local found = ui.module("search").find({ id = "x", limit = 3 })
```

This is the typed way: `zgx types` gives each module's table its type, so an editor completes the functions and a wrong argument, a missing one or a misspelled name is a type error before the app runs.

A function can also be called by its name, as a host's own function is: `ui.call_sync("words.count", text)`, `ui.action("words.keep", value)`, `ui.invoke(…)`. That is the untyped fallback, for code that already calls the host by name: it passes one argument, and neither the argument nor the result is typed. The run-time checks and their errors are the same either way.

## Types

`zgx types <app>` writes `<app>/.zgx/types.d.luau` (zgx's definitions with the app's own names, which an editor lists in place of `zgx.d.luau`). With modules in `zgx.json`, it adds each module's types from its manifest, read from the library without running it:

```lua
export type WordsModule = {
	-- How many words `text` has.
	count: (text: string) -> number,
}
-- …
	module: ((name: "words") -> WordsModule) & ((name: string) -> any),
```

A function's doc comment is its comment here. Each module's own types are also written beside its library (`words.d.luau`). `zgx check` checks with them. A module named in `zgx.json` but not built yet is left untyped, and `zgx types` says so.

## The manifest and the interface

Every module carries a manifest: its name, version, records, functions and actions, with their types, as JSON. The macros write it. It is one piece of read-only data with no pointer in it, so it reads the same in the library's file as in memory: a 16-byte magic, the format's number, the JSON's length and a checksum, the JSON, and a 16-byte end. Its symbol is `zgx_module_manifest_<name>`, beside the entry `zgx_module_v1_<name>`, so two modules linked into one program don't collide.

```json
{
  "zgx_module": 1, "abi": 1, "name": "words", "version": "0.1.0",
  "records": { "Query": { "id": "string", "limit": { "optional": "number" } } },
  "functions": {
    "words.count": { "member": "count", "args": [["text", "string"]], "returns": "number",
                     "doc": "How many words `text` has." }
  },
  "actions": {},
  "async": {
    "words.later": { "member": "later", "args": [["text", "string"]], "resolves": "number" }
  }
}
```

Types are `"nil"`, `"boolean"`, `"number"`, `"string"`, `"any"`, `{ "list": T }`, `{ "map": T }` (string keys), `{ "record": "Name" }` and `{ "optional": T }`.

The interface (`zgx_module::abi`) is C: a value's stated layout (three words: a kind byte, then a number, or a pointer and a length), a table of the runtime's functions (make a string, a list or a map; hold a value once more; let go of one), and the registrations. Each table carries its size, so functions are added at its end without a new interface number.

### Without the macros

`zgx_module::register!("words", |host: &Host| { … })` registers closures by hand (`host.function`, `host.try_function`, `host.function_taking`, `host.action`, `host.state`). Its manifest names the module and nothing else, so its functions are checked at run time as they read their arguments, and untyped in the editor.

### From C

[`crates/zgx-module/include/zgx_module.h`](../crates/zgx-module/include/zgx_module.h) is the interface for C, and for anything that speaks C. A test compiles it and holds every size and offset to Rust's, and a module written against it alone loads and answers (`crates/zgx-script/tests/c_module/words.c`). Its `static inline` functions read and count values in place: optional, and a module using none of them is still right. A C module writes its manifest's bytes itself.

## What a call costs

A call into a loaded module costs 14 instructions more than the same function linked into the app (on a call of about 2,450); a module's reads cost what linked-in reads cost; a function returning its argument costs 55 more when it is lent it and 18 when it takes it. Zeron's first module answers the transcript's calls in fewer instructions than the code it replaced. The tables are in [design/runtime.md](design/runtime.md), section 3.
