# zgx as a runtime

Status: design, with its first stage built and measured (native modules: `crates/zgx-module`, zgx-script's `modules` feature, `Value`'s stated layout). Everything else here is proposed.

An app written in Luau should ship without compiling zgx. Today an app with any Rust of its own links zgx into its binary, so a change to its Rust, or to zgx, rebuilds and relinks the whole program, and a release is a full Rust build. This design makes zgx a prebuilt, versioned runtime, makes an app's own Rust a set of native modules the runtime loads, and makes a release packaging.

The measurements are of Zeron, the largest zgx app: its interface is about 296,000 lines of Luau and its Rust host about 52,600 lines outside tests, linked with zgx into one binary.

## What it costs today

Zeron's desktop binary, on Linux (Ryzen 9 5900X, 24 threads, rustc 1.100.0, the default linker, no compiler cache). Wall-clock seconds; the median where there are three or more runs.

| Change | Dev build | Release build (thin LTO, one codegen unit) | What rebuilds |
| --- | --- | --- | --- |
| Clean | 159.9, 147.6 | 229.2, 228.2 | 562 compile units |
| A Luau file | 0.31 (n=3) | 0.31 (n=2) | nothing: the app reloads it |
| One line of the app's Rust | 3.13 (3.11–5.20, n=5) | 101.1 (100.0–101.5, n=5) | the app's crate, and its link |
| One line of `crates/zgx` | 8.43 (n=3) | 184.5, 184.6 | zgx, eight zgx crates above it, the app, its link |
| One line of zgx-script | 5.59 (n=3) | 175.3, 179.8 | zgx-script, zgx-run, the app, its link |
| One line of zgx-desktop | 6.03 (4.48–7.04, n=3) | 141.5, 136.1 | zgx-desktop, zgx-run, the app, its link |

The zgx rows are with zgx as path dependencies. An app that pins zgx by commit, as Zeron does, takes a zgx change as a new pin, which rebuilds every zgx crate: between the 184 s above and a clean build. Zeron moved its pin 88 times in 60 days.

A release, end to end (Zeron's engine, a separate binary, then the app, then its archive): 194.7 s + 263.8 s = 458.5 s, one run. With both binaries already built, making the archive again takes 9.7 s.

So a Luau change already compiles nothing. The costs are:

- **A release build of the app's Rust.** One line costs 101 s, because thin LTO in one codegen unit redoes the whole binary. This is the baseline the runtime removes, not something to tune.
- **Every zgx change**, which rebuilds zgx inside the app.
- **Every release**, which is minutes of Rust for an app whose interface is script.

## Goal

| | Today | With the runtime |
| --- | --- | --- |
| A Luau change | compiles nothing | compiles nothing |
| A change to one module of the app's Rust | rebuilds and relinks the app | rebuilds that module, in seconds; no app link, no zgx build |
| A zgx change | rebuilds zgx in the app | a new runtime version, downloaded |
| A release | a full Rust build | bytecode, the prebuilt runtime, the modules already built |

Performance stays what it is: the interface is Luau now and stays Luau, the engine's work stays Rust, and an app's own hot code stays Rust, in modules.

Not in scope: moving an app's Rust to Luau, any change to what an app looks like or does, and native modules for third-party scripts. Scripts beside the app stay Luau.

## The parts

```
my-app/
  zgx.json          the window, the app's id, the runtime it needs, its modules
  app.luau …        the interface
  modules/
    store/          a Rust crate, built to a library on its own
      Cargo.toml    crate-type = ["cdylib"], depends on zgx-module
      src/lib.rs

zgx-run             the runtime: one prebuilt binary per platform and version
```

In development, `zgx run my-app` starts the runtime on the directory: it reads the scripts as source and reloads them as they change, as now, and loads each module's library. For a release, `zgx build my-app` writes a bundle: the scripts as bytecode, the assets, each module's library for each platform, and a manifest of their hashes. An installed app is the runtime beside its bundle.

## 1. The runtime

`zgx-run` is already most of a runtime: it finds an app (`script_path`, `crates/zgx-run/src/lib.rs:317`), reads `zgx.json` (`AppConfig::read`, `lib.rs:153`), opens the window, installs the network (`fetch`, `socket`), runs the script and watches it (`run_hosting`, `lib.rs:368`). What it lacks is a release of its own and a way to carry an app's Rust.

- **Built once per platform and version**, by zgx's CI on a release tag, with the `dist` profile (`Cargo.toml`), and published as a release artifact per target: `zgx-run-<version>-<target>`. Its features are the shipping set its `Cargo.toml` already names (no `inspect`), with a second build that has the devtools, for development.
- **An app says which it needs**: `"runtime": "0.2"` in `zgx.json`, a version requirement as cargo reads one. `zgx run` and `zgx build` fetch that runtime into a cache (`~/.cache/zgx/runtimes/<version>-<target>/`) and use it. `zgx_script::app_config::KEYS` (`crates/zgx-script/src/app_config.rs:38`) gains `runtime` and `modules`.
- **What a version promises** is the Luau API (`ui.*`, `crates/zgx-script/types/zgx.d.luau`) and the module interface's number (below). The Rust API of zgx's crates promises nothing to an app on the runtime: it no longer links them.

What stays in the runtime is what an app must not have to build: layout, text, the renderer, the Luau VM and `ui.*`, the terminal, the editor, markdown, the component library, the network transports, accessibility, and the platform host.

## 2. Native modules

Built: `crates/zgx-module` (the interface and a Rust API over it), `crates/zgx-script/src/modules.rs` (the runtime's side), `crates/zgx-script/tests/native_modules.rs`.

A module is a Rust crate built as a `cdylib`. It depends on `zgx-module` (and its macros) and on nothing of zgx. It exports two symbols ending in its name: its entry, `zgx_module_v1_<name>`, which the runtime calls with two tables of C functions, and its manifest, `zgx_module_manifest_<name>`, which says what it registers with which types. [docs/modules.md](../modules.md) is the guide.

```rust
#[zgx_module::module(name = "words")]
mod words {
    #[zgx_module::export]
    fn count(text: &str) -> f64 {
        text.split_whitespace().count() as f64
    }
}
```

Scripts call it as a member of the module's table, typed from its manifest (`ui.module("words").count(text)`), or by name as they call a host's function (`ui.call_sync("words.count", text)`), untyped. Every script reaches it, as every script reaches a host function.

### Why a C interface

Rust has no stable ABI: a library built by one compiler, or against one build of zgx, can't safely be loaded by another. So what crosses is C (`zgx_module::abi`): `#[repr(C)]` structs and `extern "C"` functions. A module built once runs in any runtime that speaks its interface number, whatever compiled either.

Rejected: building zgx as a Rust `dylib` and linking modules against it. Calls would cost nothing, but every module would be tied to the exact compiler and the exact build of zgx, and the runtime could not be prebuilt and versioned.

### Values

The host API's one data type is `zgx_script::Value` (`crates/zgx-script/src/value.rs`): strings, lists and maps shared rather than copied. A module must neither copy them nor pay a call to read them: code linked into an app reads a row's field with a few loads, and a module has to cost the same.

So what crosses is the value's own layout. A module is lent a pointer to a `Value` where the runtime holds it, and reads it there.

| | Where | Stated by |
| --- | --- | --- |
| which kind | the value's first byte | `Value`: `#[repr(u8)]`, its variants numbered |
| a boolean, a number | in the value | the same |
| a string | a pointer to its UTF-8, then its length | `Str`: a `Slice<u8>` |
| a list | a pointer to its items, each a value, then its length | `Slice<Value>` (`List`): `#[repr(C)]` |
| a map | the same; its fields 40 bytes apart, sorted by key | `Slice<Field>` (`Map`) |
| a field | its key, a string, then its value | `Field(Str, Value)`: `#[repr(C)]` |
| how many hold a string, a list or a map | the word 16 bytes before its text or its items | `Slice`'s counts: `#[repr(C)]` |

Every row is a layout zgx states in its own types (`crates/zgx-script/src/counted.rs`). None is the compiler's choice, and nothing of Rust's `Rc` is in it, so none can move with a compiler: zgx-script fails to build if its types stop matching the numbers a module reads by (`const` assertions in `crates/zgx-script/src/modules.rs`), and a test reads the words themselves (`the_layout_is_the_one_stated`).

- **A string, a list and a map are one type**, zgx's own counted slice (`Slice<T>`): as `Rc<[T]>` is, but with its counts and its items at stated places. A string is a slice of bytes that are UTF-8.
- **It points at its items, with its counts before them.** Holding and letting go of a string, a list and a map are then the same instructions, which the compiler folds into one path, as it did when all three were `Rc`s. (Pointing at the counts cost each clone and drop 3 instructions more than `Rc`'s.)
- **A `Str` is not an `Rc<str>`.** Turning one into the other copies the text. What keeps a value's string keeps a `Str`; zgx-script's own holders all do (a literal, a list's key, a sort key, a span's font, markdown's field names).

With that a module, with no call into the runtime:

- reads a string's text, a list's items and a map's fields in place;
- finds a field by name as zgx-script's own lookup does (`find_key`, `key_eq`: the same code, in `zgx-module`);
- holds a string, a list or a map once more, or lets go of a hold that isn't the last, by its count.

It calls the runtime (`abi::Api`) to make a string, a list or a map; to free the last hold on one; and for the kinds it can't read: a row kept folded (`open`), a string past 64 KiB, which the runtime holds in pieces (`flat`, a copy), an image or an element (passed on, cloned and freed by the runtime). 64-bit targets only.

The counts are not atomic: a module touches a value, by a function or in place, only on the thread that called it.

**What stating the layout cost zgx-script.** Nothing, and most cells gained. The whole change against main, by CI's instruction gate (cachegrind; the median of three heap placements where they differ; #2406):

| Cell | Against main |
| --- | --- |
| script-update | -0.20% |
| list-state | -0.60% |
| list-estimate | -0.45% |
| md-stream, md-stream-text | -0.25%, -0.36% |
| spans-plain, spans-follow, spans-md | -0.08%, -0.23%, -0.06% |
| library-scroll, library-scroll-lazy | -0.06%, -0.11% |
| micro-state-luau, micro-vscroll-fixed-luau, micro-vscroll-variable-luau | -0.01%, -0.01%, -0.08% |
| app-mail, app-chat, app-sketch | -0.56%, -0.17%, -0.08% |
| app-dashboard, app-dashboard-sort | -0.47%, -0.55% |
| startup | -0.60% |
| video | +0.02% |
| mount | +0.06% |
| every other cell (anime, script-layout, the Rust-only scenes) | 0.00% |

`mount` is the one cell above the bar of +0.02%, and it is not zgx's code: zgx's own instructions in it moved by 18, its allocations are the same in number, and the whole of the difference is in libc's allocator and `memcpy`, with a sign that changed between runs of this branch (+0.01%, -0.08%, +0.08%). The gains come mostly from a table's items moving into a list in one copy, a map's fields being sorted as fields, and a string read with one load fewer.

Getting here took the gate's rows. The first cut cost `list-state` +2.66% and a string over std's `Rc<str>` left two cells at +0.08%. Each cause, found by a row or by disassembly, and removed:

- a vector moved into a slice item by item, where `Rc` copied it whole; then out of line, with a call to drop none of its items (`md-stream` +347 k);
- pairs turned into fields one at a time, and arrays of pairs copied twice;
- a slice pointing at its counts (3 instructions a clone and a drop);
- a table's items drained into a list through an iterator, with a check for each; the same in `reconcile` (`script-layout` +440 k);
- a slice's clone stopping the program on an overflowed count with a call, which gave every function it is inlined into a frame and made `Value`'s clone too much to inline (`anime` +1.58 M, 2 instructions a clone; `script-update` +558 k; `video` +841 k);
- a string over `Rc<str>`: freeing its last hold rebuilt an `Rc` on the stack for `Rc`'s own free (5 to 9 instructions a string; +0.065% of `library-scroll`), which is why the string is zgx's own;
- a string's deref going through `str`'s and the slice's makers, which the compiler's first inliner counts as steps (`video` +0.03%);
- a following span's font copied every run (`spans-follow` +0.07%).

### What a module registers

Built: `function` (`ui.call_sync`) and `action` (`ui.action`, `ui.invoke`), the same registrations as `ScriptHost::function` (`crates/zgx-script/src/lib.rs:2735`) and `ScriptHost::action` (`lib.rs:2723`); and `Host::state::<T>()`, a module's own state.

A module's state is one per thread, kept by the runtime and handed over as the module loads. Hosts run on more than one thread (tests, render workers), so a plain static would be shared between them, and a `thread_local!` in a library is a call each time it is read. The handle a module keeps is a pointer: reading its state is a load.

To build, in this order, each already a method of `ScriptHost`:

| | For | The host's today |
| --- | --- | --- |
| shared state: make, read, set, append, observe | a module publishing its model to scripts | Built: `Api::shared` and its `get`, `set` (reconciling), `append`, `observe` (an effect on the state, from its next change): the host's own states |
| keyed rows: upsert, remove | a module that diffs its model on its own thread and posts only what changed | Built: `Api::shared_upsert` and `shared_remove`, by a string field; one pass over the list (the batch's keys in a table), the list written in place when nothing else holds it |
| keyed rows placed: insert and move | a module whose list's order changes (a row made at its sorted place, one moved) | Built: `Api::shared_upsert_at`, each row before a key, at the end or where it is; one walk of the list, the rows placed before each key put as it is met (a chain without recursion); a gone anchor, a row before itself or a cycle refused, nothing written |
| a call back onto the interface's thread, from any thread | a module's worker threads | Built: `Api::post` (a host handle, `Registrar::host_ref`), drained by one task that waits on a waker, made the first time a module asks |
| async actions: a promise the module settles later | `ui.call` | Built: `Registrar::async_action`, `Api::settle` (the interface's thread) and `promise_release` (any thread: a dropped promise rejects, naming the module and the action) |
| timers | a module that waits | Built: `Api::after`, on the host's timers (zgx's clock: a test's, frozen, virtual or the wall's) |
| images by name | a module that decodes or draws | Built: `Api::image_buffer` (any thread: RGBA8, allocated as the image's `Arc<[u8]>`, so set uncopied), `image_set` (the interface's thread, its length checked against width × height × 4), `image_drop`, `image_remove`; named `<module>.<name>`, gone with the host or the module's next load into it |

Zeron's host makes 107 actions, 181 async actions, 11 functions and 128 shared states, and reaches zgx's own Rust beyond them mostly for the thread hand-off and timers (59 uses). With the rows above, that surface is covered.

### Versions

`abi::ABI` is the interface's number, now 1. Each table begins with its own size. A module refuses a runtime with another number, or with a shorter table than it was built against (`zgx_module::enter`), and the runtime reports it by the module's path. Adding a function to the end of a table changes no number: old modules never read that far. The number changes only when something is removed or changes meaning, and a runtime may then carry both.

### Who may load one

Only the app, by name in its `zgx.json` (in development, by path) or in its bundle's manifest (released, by hash). `ScriptHost::load_module` is `unsafe` and takes a path from Rust. No Luau API loads a library, and a script's directory is never searched for one. Third-party scripts stay Luau.

### Failures

Nothing unwinds across an `extern "C"` function. A panic in a module's function or action is caught on the module's side of the boundary (`zgx_module::__private::guard`, around every call the macros and `Host` register) and fails the call as a returned error would, raised in the script that called; the module stays loaded. One in a module's setup refuses the module (`abi::SETUP_PANICKED`). A line highlighter's (`Host::highlighter`) is caught the same way, and turns it off for its textarea. An async action's rejects its promise with the panic's message (a promise dropped as the start unwinds is kept and settled once the panic is caught). A job's, timer's, observer's or drop's is caught and printed (`guard_quiet`): a repeating timer that panics stops. A state's `Default` that panics ends the process, as there is no state to keep.

## 3. What a module call costs

The requirement is no measurable cost. As built: a call into a loaded module costs 14 instructions more than the same function linked into the program (0.57% of the smallest call a script can make), a module's reads cost what linked-in reads cost, and Zeron's module costs less than the code it replaced on four of its five paths.

Two measurements, both in instructions as valgrind counts them, release builds.

**The same function, linked in and loaded** (`crates/zgx-script/examples/module_call_bench.rs`): a script calls it in a loop, each arm run at 5 and at 25 rounds and the two subtracted, so the process's setup is not in it. Instructions a call:

| A script's `ui.call_sync` of | Linked in | Loaded | Difference |
| --- | --- | --- | --- |
| a function of a string, returning a number | 2,449.07 | 2,463.07 | +14.00 (+0.57%) |
| a function reading four fields of each of 200 rows | 71,463.68 | 71,584.67 | +121.00 (+0.17%) |
| a function returning its argument, lent it | 3,904.27 | 3,959.27 | +55.00 (+1.4%) |
| a function returning its argument, handed it (`function_taking`) | 3,895.27 | 3,912.93 | +17.67 (+0.45%) |

- **Entering a module costs 14 instructions.** One more indirect call than a host's own function, which is itself called through a pointer; and the runtime frees the lent argument by `Value`'s general drop, where a linked-in function that knows it was handed a string frees it in place.
- **Reading costs what linked-in code's reading costs**: 121 over 800 fields read, about half an instruction a row once entering is taken off.
- **Returning the argument costs 55** when the function is lent it: it counts the string to keep it, where a linked-in function is handed it. **Handed it, 18**: a function registered as `function_taking` counts nothing, and costs 4 more than entering alone.

**Zeron's long-reply code, as its own Rust and as a module** (section 2's proof; both in one binary, chosen at launch). Each loop of 20,000 calls was run at 2 to 22 rounds, a process each, and the row is the median of the 20 differences between one count of rounds and the next; setting aside is the difference between 20 and 220 rounds. Instructions are the test's own thread's:

| | Linked in | Loaded |
| --- | --- | --- |
| `ui.call_sync("chat.reply_blocks", id)`: a string in, a held list of 2,001 out | 3,911 | 3,921 |
| `ui.call_sync("chat.reply_run", { id, key })`, the last run: two fields read, a held list out | 9,391 | 9,287 |
| The same, the first run | 8,594 | 8,510 |
| `ui.call_sync("chat.reply_run_of", { id, block })`: two fields read, a key out | 8,545 | 8,423 |
| The same loop calling a function that does nothing with `{ id, key }` | 6,688 | 6,675 |
| Setting a 2,001-block reply aside, once as its row is built | 3,404,160 (234 µs) | 1,985,702 (151 µs) |

- **A row is good to about 10 instructions.** The collector's work lands in one round or the next, so single rounds range widely (`reply_blocks`: 3,284 to 4,604); the median and the mean over all 20 rounds agree to within 13 on every row.
- **The two arms' empty loops differ by 13**, the fifth row. That is not attributed; a row's difference is good to that much less.
- **`reply_blocks` is the boundary alone**, a string in and a held list out: 11 over, 24 once the empty loop's 13 is taken off. The other three are under the linked-in code by 84 to 122.
- **The call rows have no timings.** Earlier figures for the first two rows (2,347 instructions; 7,010 and 6,909) were of calls made after the reply had been let go of, which answer nil at once: the bench set the reply aside again before its loops. It now checks in the script that each function answers before any loop runs. Setting aside was measured rightly then and counts the same now.

The module is the app's code moved line for line, with two differences that explain why it comes out under the linked-in code rather than level with it: it reads each block's fields once where the original reads some twice, and it borrows strings the original clones. Neither is the boundary; both are what a port to borrowed values does. What the boundary itself costs is the first table.

How it got here, on the 2,001-block pass, as each version of the interface was measured: a call for each field read, +55%; a row's fields in one call, +23%; the fields named, in one call, +10.7%; reading in place, the table above.

Both tables were counted on the dev machine (Ryzen 9 5900X) under valgrind 3.22, as CI's gate counts (`scripts/perf/same_run.c` preloaded, addresses not randomized): the first by cachegrind at a55d2b713, Zeron's by callgrind's count of each thread, on its branch. Setting aside's times are the kernel counter's run's, taken earlier.

## 4. Building modules, and the cache

A module is its own cargo workspace, so nothing of the app or of zgx is in its build. Measured on Zeron's module (258 lines, one small dependency):

| After a one-line edit | The module | The app, today |
| --- | --- | --- |
| Dev build | 0.18 s (0.16–0.21, n=5) | 3.13 s |
| Release build (thin LTO, one codegen unit) | 2.21 s (1.95–2.26, n=5) | 101.1 s |
| The app's own build afterwards | nothing (0.33 s of cargo finding nothing to do) | |
| Clean build | 0.46 s dev, 2.05 s release | 147.6 s, 228.2 s |

The app picks the new library up at its next launch (hot reload of modules is section 7, not built).

- **`zgx module build`** runs cargo for each module in `zgx.json`'s `modules`, for the host's target or the ones named, and copies each library into the cache: `~/.cache/zgx/modules/<key>/<name>.<so|dylib|dll>`.
- **The key** is a hash of the module's sources, its `Cargo.lock`, the compiler's version, the target, the profile and `abi::ABI`. A module whose key is in the cache isn't built. CI restores the cache by the same keys, so a release where no module changed compiles no Rust.
- **`zgx run`** builds what is stale before it starts and, watching, as a module's sources change.

Granularity is a real cost. Each library links its own copy of Rust's standard library and of every crate it uses: Zeron's small module is 341 KB stripped. A module's dependencies are compiled into it, so two modules that both use a large crate both carry it and both link it. A module made to carry what an app's model code does (JSON, SQLite, a WebSocket, HTTPS, an async runtime) is 4.1 MB stripped, takes 44.5 s to build clean, and after a one-line edit rebuilds in 0.56 s in dev (n=5) and 3.63 s in release (3.57–6.56, n=5): still seconds, but its dependencies' 4 MB and 44 s are paid again by each module that uses them. So: a few coarse modules, split where the code changes at different rates, not one per file.

## 5. The bundle

```
MyApp.zgx/
  manifest.json
  ui/…            Luau as bytecode; themes, locales, images as they are
  modules/<target>/<name>.<so|dylib|dll>
```

```json
{
  "app_id": "my-app",
  "version": "1.4.0",
  "runtime": "0.2.3",
  "abi": 1,
  "entry": "ui/app.luau",
  "files": { "ui/app.luau": "sha256:…", "modules/linux-x86_64/store.so": "sha256:…" },
  "signature": "…"
}
```

`zgx_script::build::bundle` (`crates/zgx-script/src/bundle.rs:266`) already compiles a directory to bytecode and keeps its other files, for `include_bundle!`. `zgx build` writes the same thing as files, with the manifest. The runtime loads it through what `ScriptHost::from_files` (`lib.rs:2192`) already takes: files that aren't the disk.

### Bytecode stays safe

Luau checks no bytecode it loads: bytecode made to corrupt the VM leaves the sandbox. So today zgx never reads bytecode from a file (`sources.rs:16`), and `Code` (`lib.rs:1231`) says which files may be bytecode: `VerifiedBytecode` for a bundle the app's own build compiled, `Source` for everything from the disk.

The bundle keeps that line where it is:

- **Bytecode is accepted from one place**: the bundle the runtime was started on, the install's own. Each file is read only if its hash is the manifest's, and the manifest only if its signature verifies.
- **The key that signs it is the app's**, and what vouches for the key is what vouches for the runtime binary itself: on macOS the app's code signature, which covers the runtime, the bundle and the modules together; elsewhere a public key in a file the installer writes beside the runtime. Whoever can replace that file can replace the runtime, so the bundle is trusted exactly as far as the executable is, and no further.
- **Everything else is source, always**: a script's directory beside the app's, a person's templates or themes, anything an app downloads or writes, `ui.persist`'s data. These load with `Code::Source`, which refuses bytecode, as now. Nothing in a manifest can widen this.
- **Modules are in the same manifest**, loaded by hash. A library that isn't the one the manifest names doesn't load.

An update is a new bundle, verified whole before it replaces the old one.

## 6. The release pipeline

`zgx build --release`:

1. **Scripts to bytecode** (`build::compile`, `bundle.rs:305`), with templates applied where they are on: literal interface trees rewritten to stamps as a script is compiled (PR #2368, `templates::for_compile`, on `script/templates`). The rewrite happens at build time, so a released app pays none of it at load.
2. **Modules from the cache**, for each target. One that is missing is built, on a machine that can build for that target.
3. **The runtime for each target**, by version, from zgx's releases.
4. **The manifest**, hashed and signed.
5. **The platform's package** around runtime and bundle: a `.app`, an installer, an archive. On macOS the app signs the runtime and its modules with its own identity, as an app signs any library it embeds: a library loaded into a hardened process must be signed by the same team.

For Zeron, steps 1, 3, 4 and 5 are today's 9.7 s of packaging plus compiling the Luau, and step 2 is nothing in a release where no module changed.

**Luau native code** is left out. It was measured and shelved (`docs/performance/luau-native-code.md`): it costs at load even where unused, and it gains in pure Luau loops, which an app with modules moves to Rust instead. The pipeline has the place for it (step 1, per file) if an app ever shows a Luau loop as its bottleneck.

## 7. Hot reload

Scripts reload as now (`ScriptHost::watch`, `lib.rs:4022`): state is the runtime's, so it survives.

A module, in development:

1. Its sources change; `zgx run` rebuilds it.
2. The runtime loads the new library from a copy under a new name (a loader gives back the library it already has for a path it has seen).
3. The new library's entry registers its names, which replace the old ones (a registration is an insert by name).
4. The scripts reload, so nothing built from the old library's answers stays on screen.

Rules:

- **The old library is never unloaded.** Values it made are the runtime's and stay valid, but its code may still be on a stack or in a thread, and unloading code that has thread-local destructors is not safe on any platform. A development session leaks one library per rebuild.
- **A module's own memory doesn't cross.** What must survive a swap lives in shared state, which is the runtime's. A module that keeps more exports `zgx_module_save` and `zgx_module_restore`, a value handed from the old library to the new.
- **A module with threads** exports `zgx_module_stop`, which the runtime calls and waits for before loading the new one. Without it, a module with threads is not swapped: the runtime says so and the app restarts, which costs a launch, not a build.
- **Released, nothing is swapped.** Modules load once, as the app starts, by the manifest's hashes.

Not built: the first stage loads modules once, at start.

## 8. What an app does in Rust that isn't a module

Zeron's host goes past functions and state in six places. The target is a generic prebuilt runtime, so each needs a script-level equivalent in zgx. Until they exist, the fallback is a thin build of the runtime with the app's own setup code in it (`zgx_run::run_hosting`), rebuilt only when that code or zgx changes. That is interim only: it brings back a link per zgx version.

| What the host does in Rust | What zgx has | What the runtime needs |
| --- | --- | --- |
| Opens its windows: which one first, where, with what frame, transparency and blur | `zgx.json`'s window keys; `ui.window` declarations (`follow_windows`, `crates/zgx-run/src/lib.rs`); `ui.args` | The first window chosen by script from `ui.args`; window options for transparency and desktop blur |
| Draws a bar of its own around the app's view, above anything a script opens | `View::top_layer` in Rust | A script root named in `zgx.json`, composed above the app's view in the top layer |
| The application menu and its standard items | `ui.command` (menu items and shortcuts) | The platform's standard menus (About, Services, Window) declared in `zgx.json` |
| File and folder dialogs | `ui.open_files`, `ui.open_folder`, `ui.save_file` | Nothing |
| Renders agent visuals offscreen with its own GPU renderer | `ui.shader`, `ui.scene3d`, `ui.render_canvas`; `ScriptHost::image` | Images by name through the module interface (section 2), so a module that renders hands pixels to `ui.image` |
| Tests its host against a headless app (`zgx_script::testing::TestApp`) | `zgx test` for Luau tests | `zgx test` loading the app's modules; a module's own tests take zgx-script as a dev-dependency, which is a test build, not the dev loop |

## 9. Costs and risks

- **Modules are bound to 64-bit targets**, and to the layout section 2 states: changing it is a new interface number.
- **A `Str` is not an `Rc<str>`** (section 2): Rust that hands a value's string to something wanting an `Rc<str>` copies it.
- **Size.** Each module carries its own standard library and dependencies. Zeron's small module is 341 KB stripped; one with an app's usual dependencies 4.1 MB (section 4).
- **Two allocators.** A module frees what it allocates and the runtime what it allocates; values are always the runtime's, so nothing crosses to be freed by the other. A module that wants mimalloc sets it itself.
- **A module is trusted code.** It runs in the app's process with the app's rights. A script runs in a Luau VM within its limits; nothing here extends that to modules, which is why only the app may name one.
- **Signing.** macOS's hardened runtime loads only libraries signed by the app's team (section 6, step 5). Windows and Linux ask nothing.
- **A panic is caught wherever the runtime calls a module** (section 2), but in a state's `Default`.
- **The runtime's features are fixed at its build.** An app that needs a feature the published runtime lacks (an audio codec, say) needs another published flavour, or the interim build of section 8.
- **Open: templates in a bundle.** An app whose people lay their own templates over its interface (Zeron's) runs that interface from a copy with the template applied. A bundle's interface is bytecode, loaded only from the bundle itself, so the copy can't be made from it, and the bundle doesn't carry the sources a template would be laid over. Until it does (sources beside the bytecode, or templates applied at load), an app whose people use templates is built with `zgx build --ui source`, which keeps `ui/` as source and checks its modules alone.
- **What a bundle checks is what its host reads.** Each file a host reads from the bundle (scripts, and the images, themes and locales it reads through its files) is checked as it is read. A file an app reads itself, with its own Rust (Zeron's template survey, say), isn't: it is the app's data, never run as code, and the app is trusted as its binary is.
- **Not covered:** iOS and Android, which load no libraries at run time. A mobile app links its modules statically (`ScriptHost::enter_module` registers one by its entry, with no loading).

## 10. Order of work

1. **Functions, actions and a module's state; values read in place; loading at start.** Built and measured here.
2. **Shared state, the thread hand-off, async actions, timers, images** (section 2's table). With these an app's model code can be a module, not only its pure functions.
3. **`zgx.json`'s `modules`, `zgx module build`, the cache** (section 4).
4. **The bundle and its trust** (section 5), and `zgx build` (section 6).
5. **The script-level pieces of section 8**, each of which removes a reason for an app-specific runtime build.
6. **Published runtimes** (section 1). From here an app with no setup code of its own builds no zgx.
7. **Module hot reload** (section 7).

For Zeron the same order moves its host across: its pure functions first, its chat and session models after stage 2, and its window setup after stage 5, at which point its release is the pipeline of section 6.

## Appendix: what stating the layout changed for apps

[docs/value-types.md](../value-types.md) has each pattern that changed for Rust code, with its fix. Counted from the change itself: 460 lines in 36 files of zgx's own crates (about 634,000 lines of Rust).
