#!/bin/sh
# What a release publishes, checked as crates.io will build it, uploading
# nothing (`.github/workflows/release.yml`; see docs/releasing.md):
#
#     scripts/release_check.sh [--allow-dirty]
#
# 1. The forks zgx publishes as its own crates (vendor/harfrust,
#    cosmic-text, taffy, mlua, alacritty-terminal, zune-jpeg as zgx-harfrust,
#    zgx-cosmic-text, zgx-taffy, zgx-mlua, zgx-alacritty-terminal,
#    zgx-zune-jpeg) are each packaged and built from their package, against
#    crates.io and the forks they depend on (zgx-cosmic-text shapes with
#    zgx-harfrust), given from vendor/.
# 2. Every published workspace crate is packaged and built from its package
#    (`cargo publish --workspace --dry-run`), in dependency order. That
#    build leaves out the workspace's `[patch.crates-io]`: winit, wgpu-hal
#    and AccessKit's consumer and adapters come from crates.io (their
#    `links` fallbacks built), as an app depending on
#    zgx gets them, so what only builds with a vendored patch fails here
#    (its fallbacks are what is built). The forks, not on crates.io until
#    the release publishes them first, are given from vendor/ as `[patch]`
#    configuration for this run alone.
# 3. A new app with no lockfile, resolving every other crate afresh from
#    crates.io (as `cargo add zgx-script` does), depends on what 1 and 2
#    packaged and runs a script headless: zgx's own lockfile holds its
#    dependencies where they were tested, which a new app's does not
#    (luau0-src 0.22.1 failed every Luau app at start, unseen by 1 and 2).
#    Another builds every published crate's features that this platform has
#    from those packages: 2 builds default features alone, and a feature's
#    files left out of a package, or code only it compiles, would otherwise
#    first fail for an app turning it on. Ghostty's terminals need Zig 0.16,
#    as a user's build does (zgx-ghostty-sys).
# 4. Each published crate's previous release, as crates.io has it, builds
#    against every other zgx crate 2 packaged: an app on it that runs
#    `cargo update` gets them, and it calls them in places a doc(hidden),
#    return-type or literal change breaks without semver-checks seeing it
#    (zgx-desktop 0.1.3 calls `zoom::wheel`; zgx-script 0.1.6 built
#    zgx-components' `TabsOptions` field by field).
# 5. cargo semver-checks of each against its previous release adds a pub
#    field only to structs with Default (scripts/semver_defaults.py): an
#    app's literal of one without it has no way to keep compiling
#    (zgx-script's `Line` and `Window` at 0.1.7's first freeze). Its other
#    lints are left to the release's rulings (docs/releasing.md). Needs
#    cargo-semver-checks.
# Before them: changelog.d/'s entries are well formed, and none is left out
# of a CHANGELOG.md that has this version's section (scripts/changelog.py);
# each fork whose version crates.io has is what crates.io has
# (scripts/check_forks.py; 2 and 3 are given vendor/, and would hide a change
# made without a version), and zgx's Thai word dictionary (crates/zgx/data/thaidict.bin) is
# what scripts/extract_thai_dictionary.py extracts from the locked ICU4X
# data, and zgx's package carries its Unicode licence.
set -eu
cd "$(git rev-parse --show-toplevel)"
# `--allow-dirty`: check uncommitted changes too (a release itself is clean).
dirty=""
if [ "${1:-}" = --allow-dirty ]; then dirty=--allow-dirty; fi
forks="harfrust cosmic-text taffy mlua alacritty-terminal zune-jpeg"
# Where packages are unpacked, the forks' beside the workspace's (3 reads
# them there).
target="${CARGO_TARGET_DIR:-target}"
set --
for fork in $forks; do
    set -- "$@" --config "patch.crates-io.zgx-$fork.path=\"vendor/$fork\""
done
for fork in $forks; do
    echo "== zgx-$fork (vendor/$fork)"
    # mlua builds for one Lua, chosen by a feature: zgx's, Luau.
    features=""
    if [ "$fork" = mlua ]; then features="--features luau"; fi
    # A fork is given only the forks it depends on: a patch it never uses
    # is written into its own lockfile (`[[patch.unused]]`, harfrust's and
    # taffy's are committed), which leaves the checkout dirty.
    deps=""
    if [ "$fork" = cosmic-text ]; then
        deps='patch.crates-io.zgx-harfrust.path="vendor/harfrust"'
    fi
    cargo package --manifest-path "vendor/$fork/Cargo.toml" $features $dirty \
        --target-dir "$target" ${deps:+--config "$deps"}
done
# The first Python that runs: Windows' runners name it python, and a
# Windows without Python has a python3 that only opens the Store (it is
# found, and exits 49: "Python was not found").
python=""
for candidate in python3 python; do
    if "$candidate" -c '' >/dev/null 2>&1; then
        python=$candidate
        break
    fi
done
if [ -z "$python" ]; then
    echo "no Python runs (python3 or python): the checks below need one" >&2
    exit 1
fi
echo "== the changelog's entries"
"$python" scripts/changelog.py check --release "$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
echo "== the forks against crates.io"
"$python" scripts/check_forks.py
echo "== zgx's vendored Thai dictionary"
"$python" scripts/extract_thai_dictionary.py --check
cargo package -p zgx --list $dirty | grep -qx data/LICENSE-UNICODE
echo "== the workspace's published crates"
cargo publish --workspace --dry-run --locked $dirty "$@"
echo "== a new app, resolving afresh, runs a script"
version() { grep -m1 '^version' "$1" | cut -d'"' -f2; }
zgx=$(version Cargo.toml)
app="$target/fresh-app"
rm -rf "$app"
mkdir -p "$app/src"
{
    printf '[package]\nname = "zgx-fresh-app"\nversion = "0.0.0"\nedition = "2024"\npublish = false\n\n'
    # A workspace of its own, not a member of zgx's when under its target/.
    printf '[workspace]\n\n[dependencies]\nzgx-script = "=%s"\n\n[patch.crates-io]\n' "$zgx"
    for crate in zgx zgx-components zgx-gpu zgx-desktop zgx-script zgx-run zgx-terminal zgx-ghostty-sys; do
        printf '%s = { path = "../package/%s-%s" }\n' "$crate" "$crate" "$zgx"
    done
    for fork in $forks; do
        printf 'zgx-%s = { path = "../package/zgx-%s-%s" }\n' "$fork" "$fork" "$(version "vendor/$fork/Cargo.toml")"
    done
} > "$app/Cargo.toml"
cat > "$app/src/main.rs" <<'RUST'
// A Luau script loaded, mounted and clicked, headless.
fn main() {
    let mut app = zgx_script::testing::TestApp::inline(
        r#"local count = ui.state("count", 1)
return ui.column {
    ui.text { id = "count", "Count: {count}" },
    ui.button { id = "more", label = "More", on_click = function() count:set(count:get() + 1) end },
}"#,
    );
    app.click("more");
    assert_eq!(app.text("count"), "Count: 2");
    println!("a script ran: {}", app.text("count"));
}
RUST
cargo run --manifest-path "$app/Cargo.toml" --target-dir "$target"
echo "== every published feature this platform has, built from its package"
case "$(uname -s)" in
    Darwin) platform=macos ;;
    Linux) platform=linux ;;
    *) platform=windows ;;
esac
features="$target/features-app"
rm -rf "$features"
mkdir -p "$features/src"
"$python" - "$platform" "$zgx" "$target" $forks > "$features/Cargo.toml" <<'PY'
import json, re, subprocess, sys
platform, zgx, target, forks = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:]
platform_features = {}
# Features for some platforms alone, and which. Every other feature of a
# published crate is built everywhere: a new one is covered unless it says
# here where it isn't.
ONLY = {
    "zgx-desktop/webview2": {"windows"},
    "zgx-desktop/d3d11": {"windows"},
    "zgx-gpu/d3d11": {"windows"},
    "zgx-gpu/composition": {"windows"},
    "zgx-desktop/vulkan": {"linux", "windows"},
    "zgx-gpu/vulkan": {"linux", "windows"},
    "zgx-script/vulkan": {"linux", "windows"},
    "zgx-desktop/ghostty": {"macos"},
    # Compiles to nothing elsewhere; WebKitGTK is opened at run time.
    "zgx-desktop/webkitgtk": {"linux"},
    # Turned on through zgx-desktop/ghostty, which gives it what it links to.
    "zgx-ghostty-sys/link": set(),
}
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"]))
published = [p for p in metadata["packages"] if p["publish"] is None and p["name"].startswith("zgx")
             and p["manifest_path"].replace("\\", "/").split("/")[-3] == "crates"]
print('[package]\nname = "zgx-features-app"\nversion = "0.0.0"\nedition = "2024"\npublish = false\n')
print("[workspace]\n\n[dependencies]")
for package in sorted(published, key=lambda p: p["name"]):
    name = package["name"]
    turned = sorted(f for f in package["features"]
                    if f != "default" and platform in ONLY.get(f"{name}/{f}", {platform}))
    print(f'{name} = {{ version = "={zgx}", features = {json.dumps(turned)} }}')
    platform_features[name] = turned
# The same features for semver-checks' run at the end (5).
with open(f"{target}/platform-features.json", "w") as file:
    json.dump(platform_features, file)
print("\n[patch.crates-io]")
for package in sorted(published, key=lambda p: p["name"]):
    print(f'{package["name"]} = {{ path = "../package/{package["name"]}-{zgx}" }}')
for fork in forks:
    manifest = open(f"vendor/{fork}/Cargo.toml").read()
    version = re.search(r'(?m)^version = "([^"]+)"', manifest)[1]
    print(f'zgx-{fork} = {{ path = "../package/zgx-{fork}-{version}" }}')
PY
grep -E '^zgx[a-z0-9-]* = \{ version' "$features/Cargo.toml"
"$python" - "$features/Cargo.toml" > "$features/src/main.rs" <<'PY'
import re, sys
for name in re.findall(r'^(zgx[a-z0-9-]*) = \{ version', open(sys.argv[1]).read(), re.M):
    print(f"use {name.replace('-', '_')} as _;")
print("fn main() {}")
PY
# Ghostty fetched and built as a user's first build does, not from the
# machine's shared cache (zgx-ghostty-sys keys it by commit, patch and Zig).
# Emptied first: CI's cache of target/ can keep part of an earlier one.
ghostty_cache="$(cd "$target" && pwd)/ghostty-cache"
rm -rf "$ghostty_cache"
ZGX_GHOSTTY_CACHE="$ghostty_cache" cargo build --manifest-path "$features/Cargo.toml" --target-dir "$target"
echo "every published feature for $platform built from its package"
echo "== the previous release's crates against this release"
# Each published crate, its newest version on crates.io below this one ("-"
# for a crate new in this release), and the features this platform builds
# of each (the features app's, those the previous version has): one
# "crate version features previous-features" a line, lists comma-joined.
"$python" - "$zgx" "$target" > "$target/previous.txt" <<'PY'
import json, subprocess, sys, urllib.error, urllib.request
zgx, target = sys.argv[1], sys.argv[2]
sys.stdout.reconfigure(newline="\n")  # read by the shell: no CR on Windows
platform_features = json.load(open(f"{target}/platform-features.json"))
def key(version):
    return tuple(int(part) for part in version.split("-")[0].split("."))
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"]))
for package in sorted(metadata["packages"], key=lambda p: p["name"]):
    name = package["name"]
    if package["publish"] is not None or not name.startswith("zgx") \
            or package["manifest_path"].replace("\\", "/").split("/")[-3] != "crates":
        continue
    path = {1: "1/", 2: "2/", 3: f"3/{name[0]}/"}.get(len(name), f"{name[:2]}/{name[2:4]}/") + name
    request = urllib.request.Request(f"https://index.crates.io/{path}", headers={"User-Agent": "zgx-release-check"})
    try:
        lines = urllib.request.urlopen(request, timeout=30).read().decode().splitlines()
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        lines = []
    versions = {entry["vers"]: entry for entry in (json.loads(line) for line in lines if line.strip())
                if "-" not in entry["vers"] and key(entry["vers"]) < key(zgx)}
    previous = max(versions, key=key) if versions else None
    features = platform_features.get(name, [])
    had = {**versions[previous].get("features", {}), **versions[previous].get("features2", {})} if previous else {}
    print(name, previous or "-", ",".join(features) or "-", ",".join(f for f in features if f in had) or "-")
PY
if ! cut -d' ' -f2 "$target/previous.txt" | grep -qv '^-$'; then
    echo "no earlier release on crates.io"
    exit 0
fi
# Each, as crates.io has it, with every other zgx crate this release's
# package: an app on it that runs `cargo update` gets them, and it calls
# them in places a doc(hidden), return-type or literal change breaks
# without semver-checks seeing it (zgx-desktop 0.1.3 calls `zoom::wheel`;
# zgx-script 0.1.6 wrote zgx-components' `TabsOptions` out field by field).
while read -r crate previous _; do
    if [ "$previous" = - ]; then continue; fi
    compat="$target/compat-$crate"
    rm -rf "$compat"
    mkdir -p "$compat/src"
    {
        printf '[package]\nname = "zgx-compat-app"\nversion = "0.0.0"\nedition = "2024"\npublish = false\n\n'
        printf '[workspace]\n\n[dependencies]\n%s = "=%s"\n\n[patch.crates-io]\n' "$crate" "$previous"
        for other in $(cut -d' ' -f1 "$target/previous.txt"); do
            if [ "$other" != "$crate" ]; then
                printf '%s = { path = "../package/%s-%s" }\n' "$other" "$other" "$zgx"
            fi
        done
        for fork in $forks; do
            printf 'zgx-%s = { path = "../package/zgx-%s-%s" }\n' "$fork" "$fork" "$(version "vendor/$fork/Cargo.toml")"
        done
    } > "$compat/Cargo.toml"
    printf 'use %s as _;\nfn main() {}\n' "$(echo "$crate" | tr - _)" > "$compat/src/main.rs"
    cargo check --manifest-path "$compat/Cargo.toml" --target-dir "$target" < /dev/null
    echo "$crate $previous builds against this release's crates"
done < "$target/previous.txt"
echo "== new public fields only on structs with Default (cargo semver-checks)"
if ! cargo semver-checks --version >/dev/null 2>&1; then
    echo "cargo-semver-checks is needed: cargo install cargo-semver-checks --locked" >&2
    exit 1
fi
semver="$target/semver"
rm -rf "$semver"
mkdir -p "$semver"
while read -r crate previous current had; do
    if [ "$previous" = - ]; then continue; fi
    # Default features and this platform's others, as the features app
    # built them, not semver-checks' guess at them (it took
    # zgx-ghostty-sys/link, macOS's alone, everywhere).
    set -- --default-features
    for feature in $(echo "$current" | tr , ' '); do
        if [ "$feature" != - ]; then set -- "$@" --current-features "$feature"; fi
    done
    for feature in $(echo "$had" | tr , ' '); do
        if [ "$feature" != - ]; then set -- "$@" --baseline-features "$feature"; fi
    done
    # Its own exit is not the verdict: the release's rulings take some
    # lints (docs/releasing.md). A run with no summary did not check.
    cargo semver-checks -p "$crate" --baseline-version "$previous" --color never "$@" \
        < /dev/null > "$semver/$crate.log" 2>&1 || true
    if ! grep -q 'Summary' "$semver/$crate.log"; then
        cat "$semver/$crate.log" >&2
        echo "cargo semver-checks did not check $crate against $previous" >&2
        exit 1
    fi
    echo "$crate against $previous: $(grep -m1 'Summary' "$semver/$crate.log" | sed 's/^ *Summary *//')"
done < "$target/previous.txt"
"$python" scripts/semver_defaults.py "$semver"/*.log
