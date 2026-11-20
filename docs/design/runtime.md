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

The host API's one data type is `zgx_script::Value` (`crates/zgx-script/src/value.rs`): strings, lists and maps held in `Rc`s, which state shares rather than copies. A module must neither copy them nor pay a call to read them: code linked into an app reads a row's field with a few loads, and a module has to cost the same.

So what crosses is the value's own layout. A module is lent a pointer to a `Value` where the runtime holds it, and reads it there.

| | Where |
| --- | --- |
| which kind | the value's first byte (`Value` is `#[repr(u8)]`, its variants numbered) |
| a boolean, a number | in the value |
| a string | a counted pointer and a length: its UTF-8 is 16 bytes past the pointer |
| a list | the same: its items, each a value, 24 bytes apart |
| a map | the same: its fields, each a key and a value, 40 bytes apart, sorted by key |
| how many hold a string, a list or a map | the word at the counted pointer |

With that a module, with no call into the runtime:

- reads a string's text, a list's items and a map's fields in place;
- finds a field by name as zgx-script's own lookup does (`find_key`, `key_eq`: the same code, in `zgx-module`);
- holds a string, a list or a map once more, or lets go of a hold that isn't the last, by its count, as `Rc` does.

It calls the runtime (`abi::Api`) to make a string, a list or a map, to free the last hold on one, and for the kinds it can't read: a row kept folded (`open`), a string past 64 KiB, which the runtime holds in pieces (`flat`, a copy), an image or an element (passed on, cloned and freed by the runtime).

**What stating the layout cost zgx-script.** Nothing measurable. The whole change against main, in retired instructions on zgx's perf scenes (`scripts/perf/suite.py ab`, and the gate's Luau cells run by hand at the gate's settings, the least of 5 runs each):

| Cell | Against main |
| --- | --- |
| mount | -0.06% |
| script-update | 0.00% (the suite's), +0.01% (the gate's settings) |
| micro-state-luau, micro-vscroll-fixed-luau, micro-vscroll-variable-luau | 0.00%, +0.03%, 0.00% |
| library-scroll, library-scroll-lazy | -0.03%, -0.06% |
| anime, script-layout, list-state, list-estimate | +0.02%, +0.01%, +0.01%, 0.00% |
| spans-plain, spans-follow, spans-md | -0.23%, +0.01%, +0.02% |
| md-stream, md-stream-text | -0.02%, -0.08% |
| app-mail, app-dashboard, app-sketch, app-code | +0.11%, 0.00%, -0.02%, 0.00% |

None is outside its own spread between runs (app-mail's was 0.19%). These are the kernel's counts, not the gate's own (cachegrind, which the machine didn't have), so CI's gate on the branch is the exact check.

**What this rests on.** `Value`'s layout is stated in its source and tested. The layout of an `Rc` of a slice (a pointer to two counts and then the contents, with a length beside it) and of a map's field (a key, then a value) is the compiler's, the same in every Rust so far but promised by none. So the runtime measures its own as it loads a module (`measured`, `crates/zgx-script/src/modules.rs`): it makes a string, a list and a map, and checks where their parts lie. If that isn't the layout modules read (`abi::LAYOUT`), no module loads, with an error that says so, and a module refuses a runtime that reports another layout. A compiler that changed one would fail zgx-script's own test of it first (`this_build_lays_values_out_as_modules_read_them`). The lasting fix, if that day comes, is a map's field as a `#[repr(C)]` struct and zgx's own counted slice; neither is needed now. 64-bit targets only.

A function's argument is lent for the call. What it returns is owned by the runtime from then.

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

The requirement is no measurable cost. As built, the first stage meets it: a call into a loaded module costs 7 instructions more than the same function linked into the program, a module's reads cost what linked-in reads cost, and Zeron's module costs no more than the code it replaced.

Two measurements, both in retired instructions (user mode), release builds.

**The same function, linked in and loaded** (`crates/zgx-script/examples/module_call_bench.rs`): a script calls it a million times (rows: 100,000), each arm run three times, the process's setup subtracted.

| A script's `ui.call_sync` of | Linked in | Loaded | Difference |
| --- | --- | --- | --- |
| a function of a string, returning a number | 2,460.1 instr | 2,467.1 instr | +7.0 (+0.28%) |
| a function reading four fields of each of 200 rows | 71,467 instr | 71,615 instr | +148 (+0.21%) |
| a function returning its argument | 3,915 instr | 3,965 instr | +50 (+1.3%) |

- **Entering a module costs 7 instructions**: one more indirect call than a host's own function, which is itself called through a pointer. Runs agreed to within 0.01 of an instruction a call.
- **Reading costs what linked-in code's reading costs**: under one instruction a row.
- **Returning the argument itself costs 50**, the one case that differs by design: a module is lent its argument and counts it to keep it, where a linked-in function is handed it. A function that hands back what it was given is rare; one that needs to can be given its argument owned, a change to the interface not made here.

**Zeron's long-reply code, as its own Rust and as a module** (section 2's proof; both in one binary, chosen at launch; medians of 5 runs of the process, the two ways interleaved, each run the median of 21 rounds of 20,000 calls or one setting aside):

| | Linked in | Loaded |
| --- | --- | --- |
| `ui.call_sync("chat.reply_blocks", id)`: a string in, a held list out | 2,359 instr, 171 ns (166–176) | 2,358 instr, 161 ns (158–167) |
| `ui.call_sync("chat.reply_run", { id, key })`: two fields read, a held list out | 7,138 instr, 568 ns (550–582) | 7,013 instr, 554 ns (533–580) |
| Setting a 2,001-block reply aside, once as its row is built | 3.45 M instr, 227 µs (220–233) | 1.94 M instr, 152 µs (148–155) |

Instructions agreed between runs to within 2 a call.

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

- **The layout modules read is partly the compiler's** (section 2): checked as a module loads, and a mismatch loads nothing rather than misreading. A module is also bound to 64-bit targets.
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
