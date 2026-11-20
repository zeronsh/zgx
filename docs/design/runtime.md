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

Not in scope: moving an app's Rust to Luau, any change to what an app looks like or does, and native modules for third-party plugins. Plugins stay Luau, in their sandbox.

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

A module is a Rust crate built as a `cdylib`. It depends on `zgx-module`, which has no dependencies, and on nothing of zgx. It exports one symbol, `zgx_module_v1`, and the runtime calls it with two tables of C functions.

```rust
use zgx_module::{Host, Value};

zgx_module::module!(|host: &Host| {
    host.function("words.count", |text| {
        let n = text.str().map_or(0, |s| s.split_whitespace().count());
        Value::from(n as f64)
    });
});
```

Scripts call it as they call a host's function today: `ui.call_sync("words.count", text)`. To a script, and to `expose_action`'s rules for plugins, a module's function is a host function.

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
| a string | a pointer to its UTF-8, then its length | `Str`: `#[repr(C)]` |
| a list | a pointer to its items, each a value, then its length | `Slice<Value>` (`List`): `#[repr(C)]` |
| a map | the same; its fields 40 bytes apart, sorted by key | `Slice<Field>` (`Map`) |
| a field | its key, a string, then its value | `Field(Str, Value)`: `#[repr(C)]` |
| how many hold a list or a map | the word 16 bytes before its items | `Slice`'s counts: `#[repr(C)]` |

Every row is a layout zgx states in its own types (`crates/zgx-script/src/counted.rs`). None is the compiler's choice, so none can move with a compiler: zgx-script fails to build if its types stop matching the numbers a module reads by (`const` assertions in `crates/zgx-script/src/modules.rs`), and a test reads the words themselves (`the_layout_is_the_one_stated`).

- **A list and a map are zgx's own counted slice** (`Slice<T>`), as `Rc<[T]>` is but with its counts and items at stated places. It points at its items with its counts before them, as an `Rc<str>`'s text has its own: holding and letting go of a string, a list and a map are then the same instructions, which the compiler folds into one path as it did when all three were `Rc`s. (With the counts first, each clone and drop of a list or a map cost 3 instructions more than `Rc`'s.)
- **A string is std's own `Rc<str>`**, held as a pointer to its text and a length and made and let go of through `Rc`'s documented raw functions alone (`into_raw`, `from_raw`, `increment_strong_count`). So a `Str` and an `Rc<str>` turn into each other with no copy, and a value's string still shares its text with everything in zgx that holds an `Rc<str>`: ids, row identities, a text's source.

With that a module, with no call into the runtime:

- reads a string's text, a list's items and a map's fields in place;
- finds a field by name as zgx-script's own lookup does (`find_key`, `key_eq`: the same code, in `zgx-module`);
- holds a list or a map once more, or lets go of a hold that isn't the last, by its count.

It calls the runtime (`abi::Api`) to make a string, a list or a map; to hold or let go of a string (where std keeps an `Rc`'s count is std's, so a module doesn't touch it); to free the last hold on a list or a map; and for the kinds it can't read: a row kept folded (`open`), a string past 64 KiB, which the runtime holds in pieces (`flat`, a copy), an image or an element (passed on, cloned and freed by the runtime). 64-bit targets only.

**What stating the layout cost zgx-script.** The whole change against main, in retired instructions on zgx's perf scenes (`scripts/perf/suite.py ab`, and the gate's Luau cells run by hand at the gate's settings, the least of 5 runs each):

| Cell | Against main |
| --- | --- |
| mount | -0.04% |
| script-update | +0.12% (the suite's), +0.11% (the gate's settings) |
| micro-state-luau, micro-vscroll-fixed-luau, micro-vscroll-variable-luau | -0.01%, +0.03%, +0.03% |
| library-scroll, library-scroll-lazy | +0.08%, +0.02% |
| anime, script-layout, list-estimate | +0.04%, +0.05%, -0.02% |
| list-state | -0.42% |
| spans-plain, spans-follow, spans-md | -0.05%, +0.02%, -0.02% |
| md-stream, md-stream-text | +0.22%, +0.04% |
| app-mail | -0.17% (the suite's), -0.24% (the gate's settings) |
| app-dashboard, app-sketch, app-code | -0.07%, -0.02%, -0.01% |

Within 0.1% but for four cells. Two are better: `list-state` and `app-mail`, because a table's items now move into a list in one copy. Two are worse and not explained to the instruction: `script-update` by 0.12% and `md-stream` by 0.22%. Both make and free many short strings and small maps. What is known of the difference: a string made and freed costs 205 instructions where an `Rc<str>`'s costs 202 (its last hold is found by reading its count first), a string held and let go of costs the same 22, and a list or a map held and let go of costs the same 33.

The first cut of the layout was far from this (`list-state` +2.66%), from causes each since removed:

- a vector moved into a slice item by item, where `Rc` copied it whole;
- pairs turned into fields one at a time;
- a slice pointing at its counts, which cost each clone and drop of a list or a map 3 instructions;
- a table's items drained into a list through an iterator, with a check for each;
- a string's drop making room on the stack for its last hold on every drop (about 7 instructions each).

These are the kernel's counts, not the gate's own (cachegrind, which the machine didn't have), so CI's gate on the branch is the exact check.

A function's argument is lent for the call, and what it returns is owned by the runtime from then. One that keeps or returns what it is given registers as `Host::function_taking` and is handed its argument instead: nothing is counted. That is the exception, not the default, because a function handed its argument must free it, and a table fresh from a script is held once, so freeing it is a call into the runtime on every invocation.

### What a module registers

Built: `function` (`ui.call_sync`) and `action` (`ui.action`, `ui.invoke`), the same registrations as `ScriptHost::function` (`crates/zgx-script/src/lib.rs:2735`) and `ScriptHost::action` (`lib.rs:2723`); and `Host::state::<T>()`, a module's own state.

A module's state is one per thread, kept by the runtime and handed over as the module loads. Hosts run on more than one thread (tests, render workers), so a plain static would be shared between them, and a `thread_local!` in a library is a call each time it is read. The handle a module keeps is a pointer: reading its state is a load.

To build, in this order, each already a method of `ScriptHost`:

| | For | The host's today |
| --- | --- | --- |
| shared state: make, read, set, append, observe | a module publishing its model to scripts | `ScriptHost::shared` (`lib.rs:3030`), `Shared::set` and `append` (`value.rs:2801`, `2813`) |
| a call back onto the interface's thread, from any thread | a module's worker threads | `zgx::task::latest` (`crates/zgx/src/task.rs:280`), the executor's wake (`task.rs:113`) |
| async actions: a promise the module settles later | `ui.call` | `ScriptHost::async_action` (`lib.rs:2747`) |
| timers | a module that waits | `zgx::timer::sleep` |
| images by name | a module that decodes or draws | `ScriptHost::image` (`lib.rs:2761`) |

Zeron's host makes 107 actions, 181 async actions, 11 functions and 128 shared states, and reaches zgx's own Rust beyond them mostly for the thread hand-off and timers (59 uses). With the rows above, that surface is covered.

### Versions

`abi::ABI` is the interface's number, now 1. Each table begins with its own size. A module refuses a runtime with another number, or with a shorter table than it was built against (`zgx_module::enter`), and the runtime reports it by the module's path. Adding a function to the end of a table changes no number: old modules never read that far. The number changes only when something is removed or changes meaning, and a runtime may then carry both.

### Who may load one

Only the app, by name in its `zgx.json` (in development, by path) or in its bundle's manifest (released, by hash). `ScriptHost::load_module` is `unsafe` and takes a path from Rust. No Luau API loads a library, and a plugin's directory is never searched for one. Third-party plugins stay Luau.

### Failures

A panic in a module ends the process: nothing unwinds across an `extern "C"` function. That is what a panic in a host's own function does to most apps now. A module that wants to survive one catches it itself.

## 3. What a module call costs

The requirement is no measurable cost. As built it is close and not there: a call into a loaded module costs 16 instructions more than the same function linked into the program (0.65% of the smallest call a script can make), a module's reads cost what linked-in reads cost, and Zeron's module costs less than the code it replaced.

Two measurements, both in retired instructions (user mode), release builds.

**The same function, linked in and loaded** (`crates/zgx-script/examples/module_call_bench.rs`): a script calls it a million times (rows: 100,000), each arm the least of three runs, the process's setup subtracted.

| A script's `ui.call_sync` of | Linked in | Loaded | Difference |
| --- | --- | --- | --- |
| a function of a string, returning a number | 2,466.1 instr | 2,482.1 instr | +16.0 (+0.65%) |
| a function reading four fields of each of 200 rows | 71,437 instr | 71,614 instr | +177 (+0.25%) |
| a function returning its argument, lent it | 3,927 instr | 3,988 instr | +62 (+1.6%) |

- **Entering a module costs 16 instructions.** One more indirect call than a host's own function, which is itself called through a pointer; and the runtime frees the lent argument by `Value`'s general drop, where a linked-in function that knows it was handed a string frees it in place. With the layout measured from `Rc` this was 7; the stated layout's string is 4 instructions longer to let go of on this path, and the rest of the difference is not attributed.
- **Reading costs what linked-in code's reading costs**: under one instruction a row.
- **Returning the argument costs 62** when the function is lent it: it counts the string to keep it, through the runtime, where a linked-in function is handed it. A function registered as `function_taking` is handed its argument too; that case was not measured.

**Zeron's long-reply code, as its own Rust and as a module** (section 2's proof; both in one binary, chosen at launch; medians of 5 runs of the process, the two ways interleaved, each run the median of 21 rounds of 20,000 calls or one setting aside):

| | Linked in | Loaded |
| --- | --- | --- |
| `ui.call_sync("chat.reply_blocks", id)`: a string in, a held list out | 2,367 instr, 181 ns (179–218) | 2,366 instr, 172 ns (171–174) |
| `ui.call_sync("chat.reply_run", { id, key })`: two fields read, a held list out | 7,085 instr, 589 ns (574–610) | 6,959 instr, 570 ns (563–586) |
| Setting a 2,001-block reply aside, once as its row is built | 3.45 M instr, 249 µs (247–278) | 2.00 M instr, 156 µs (150–241) |

Instructions agreed between runs to within 1 a call.

The module is the app's code moved line for line, with two differences that explain why it comes out under the linked-in code rather than level with it: it reads each block's fields once where the original reads some twice, and it borrows strings the original clones. Neither is the boundary; both are what a port to borrowed values does. What the boundary itself costs is the first table.

How it got here, on the 2,001-block pass, as each version of the interface was measured: a call for each field read, +55%; a row's fields in one call, +23%; the fields named, in one call, +10.7%; reading in place, the table above.

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
- **Everything else is source, always**: a plugin's directory, a person's templates or themes, anything an app downloads or writes, `ui.persist`'s data. These load with `Code::Source`, which refuses bytecode, as now. Nothing in a manifest can widen this.
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
| Draws a bar of its own around the app's view, above anything a plugin opens | `View::top_layer` in Rust; `ui.decision` and `ui.sealed` for regions plugins can't cover | An app-only script root named in `zgx.json`, composed above the app's view in the top layer, in a VM no plugin reaches |
| The application menu and its standard items | `ui.command` (menu items and shortcuts) | The platform's standard menus (About, Services, Window) declared in `zgx.json` |
| File and folder dialogs | `ui.open_files`, `ui.open_folder`, `ui.save_file` | Nothing |
| Renders agent visuals offscreen with its own GPU renderer | `ui.shader`, `ui.scene3d`, `ui.render_canvas`; `ScriptHost::image` | Images by name through the module interface (section 2), so a module that renders hands pixels to `ui.image` |
| Tests its host against a headless app (`zgx_script::testing::TestApp`) | `zgx test` for Luau tests | `zgx test` loading the app's modules; a module's own tests take zgx-script as a dev-dependency, which is a test build, not the dev loop |

## 9. Costs and risks

- **Modules are bound to 64-bit targets**, and to the layout section 2 states: changing it is a new interface number.
- **Size.** Each module carries its own standard library and dependencies. Zeron's small module is 341 KB stripped; one with an app's usual dependencies 4.1 MB (section 4).
- **Two allocators.** A module frees what it allocates and the runtime what it allocates; values are always the runtime's, so nothing crosses to be freed by the other. A module that wants mimalloc sets it itself.
- **A module is trusted code.** It runs in the app's process with the app's rights. The sandbox is for scripts and plugins; nothing here extends it to modules, which is why only the app may name one.
- **Signing.** macOS's hardened runtime loads only libraries signed by the app's team (section 6, step 5). Windows and Linux ask nothing.
- **A panic is an abort** (section 2).
- **The runtime's features are fixed at its build.** An app that needs a feature the published runtime lacks (an audio codec, say) needs another published flavour, or the interim build of section 8.
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

`Value`'s variants hold other types than they did, so Rust that matches on them or builds them by hand changes. `Value`'s functions are as they were: `Value::record`, `Value::from`, `field`, `list`, `items`, `at`, `set_path`, `to_json`, `from_json`.

| | Was | Is |
| --- | --- | --- |
| `Value::Str` | `Rc<str>` | `Str` |
| `Value::List` | `Rc<[Value]>` | `List` (`Slice<Value>`) |
| `Value::Map` | `Rc<[(Rc<str>, Value)]>` | `Map` (`Slice<Field>`) |
| a map's field | `(Rc<str>, Value)` | `Field(Str, Value)` |

What keeps compiling: reading a string as `&str` (`&**s`, `s.len()`, comparing with a `&str`), a list as a slice (`items.iter()`, `items[i]`, `items.len()`), `field.0` and `field.1`, `Value::List(vec.into())`, `Value::List(iter.collect())`, `Value::Str(text.into())` from a `&str`, a `String` or an `Rc<str>`, and `Value::Map(vec![("key".into(), value)].into())`.

What changes, each a compile error with the fix beside it:

| Pattern | Before | After |
| --- | --- | --- |
| an `Rc<str>` out of a value | `Value::Str(s) => s.clone()` | `Value::Str(s) => s.rc()` (or `.into()`; held once more, not copied) |
| a value from an `Rc<str>` by the variant as a function | `label.map_or(Value::Nil, Value::Str)` | `label.map_or(Value::Nil, Value::from)` |
| a pair pattern over a map's fields | `for (key, value) in fields.iter()` | `for Field(key, value) in fields.iter()` |
| a field built as a pair where a `Field` is wanted | `fields.push((key, value))` | `fields.push(Field(key, value))` |
| `Rc`'s functions on a list or a map | `Rc::ptr_eq(a, b)`, `Rc::get_mut(items)`, `Rc::strong_count`, `Rc::downgrade` | `Slice::ptr_eq(a, b)`, `Slice::get_mut(items)`, … (`Str::ptr_eq` for strings) |
| a list or a map made through `Rc` | `Value::List(Rc::from(rows))` | `Value::List(rows.into())` |
| a type naming the old payload | `Rc<[Value]>`, `Vec<(Rc<str>, Value)>` | `List`, `Vec<Field>` (or `Vec<(Str, Value)>`) |
| fields kept as `(Rc<str>, Value)` pairs, into a map | `Value::Map(pairs.into())` | keys as `Str` (`Str::from(rc)`, no copy), or `Value::record(pairs)` |

One conversion does copy and is there only so old code keeps working: `Slice::from(Rc<[T]>)` clones the items into a slice of zgx's own. Code on a hot path holds a `List` from the start.

How many sites, counted from the changes themselves:

| | Lines changed | Of |
| --- | --- | --- |
| zgx's own crates, outside the new types (zgx-script, its tests and examples, zgx-desktop's examples) | 302 | about 634,000 lines of Rust |
| Zeron's host (`apps/zgx/src`, 11 files) | 98 | 71,796 |

In Zeron 81 of its first 124 errors were fixed by rule (a pattern becomes `Field(…)`, `.clone()` becomes `.rc()`, `Rc::from` becomes `Slice::from`) and 14 sites by hand: a struct holding `Rc<[Value]>`, a function taking `&mut Rc<str>`, three places building fields as pairs.
