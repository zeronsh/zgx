#!/usr/bin/env bash
# CI's Linux and macOS steps, run on one commit on machines at hand, for
# when a branch lands without waiting on GitHub's CI.
#
#   scripts/local_ci.sh COMMIT [--base REF] [--linux] [--gate] [--keep]
#
# On this machine (macOS): the macOS job's lines, and those of the Linux
# job that don't need Linux. The commit is checked out in a worktree of its
# own under $LOCAL_CI_DIR (default /tmp/zgx-local-ci), built into one
# target directory there, deleted at the end unless --keep:
#   fmt; clippy 1.99 on the workspace, on zgx-script without default
#   features and on zgx-3d with every feature and with each alone; the MSRV check; the Python tests; changelog.py check against
#   --base; StyLua; the script lint and, when luau-lsp is on PATH or in
#   $LUAU_LSP, the script type check; the tests of every crate the jobs
#   test, zgx-audio's and zgx-video's (with ffmpeg) among them; the doc
#   tests and cargo doc; zgx-3d's GPU tests on Metal (ZGX_REQUIRE_GPU=1).
#
# With --linux, in a Linux machine reached by $LOCAL_CI_LINUX (a command
# prefix, e.g. `orb -m zgx-linux`; it must see this repository at the same
# path) under $LOCAL_CI_LINUX_DIR: clippy with the Linux job's features,
# the Linux job's tests, zgx-gpu's Vulkan tests and zgx-3d's GPU tests on
# its software Vulkan (llvmpipe). With --gate, the instruction gate there
# against --base (cachegrind, a ZGX_PERF_DIR of its own), its rows printed as
# numbers and read against +0.02% for the UI cells and each 3D cell's spread
# (scripts/perf/read_gate.py).
#
# Not mirrored, and said so in the summary:
#   - Windows (its own machine);
#   - the Linux probes (real X11 and Wayland windows), the macOS native
#     validation and component-host smokes (windows on screen);
#   - the patched Vulkan loader CI builds;
#   - CI's x86-64 runners (Linux here is whatever the machine is).
#
# Each step's log is kept under $LOCAL_CI_DIR/logs. Exits 1 if any step
# failed.
set -uo pipefail

commit=${1:?usage: scripts/local_ci.sh COMMIT [--base REF] [--linux] [--gate] [--keep]}
shift
base=origin/main
linux=0 gate=0 keep=0
while [ $# -gt 0 ]; do
  case $1 in
    --base) base=$2; shift ;;
    --linux) linux=1 ;;
    --gate) gate=1; linux=1 ;;
    --keep) keep=1 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done

repo=$(git rev-parse --show-toplevel)
sha=$(git -C "$repo" rev-parse --verify "$commit^{commit}") || exit 2
base_sha=$(git -C "$repo" rev-parse --verify "$base^{commit}") || exit 2
work=${LOCAL_CI_DIR:-/tmp/zgx-local-ci}
src=$work/src
logs=$work/logs
export CARGO_TARGET_DIR=$work/target CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0
mkdir -p "$logs"
rm -f "$logs"/*.log "$work/summary.txt"

git -C "$repo" worktree remove --force "$src" 2>/dev/null
rm -rf "$src"
git -C "$repo" worktree add --detach "$src" "$sha" >/dev/null || exit 2
cd "$src" || exit 2

failed=0
n=0
summary() { echo "$1" | tee -a "$work/summary.txt"; }
# step NAME COMMAND...: runs COMMAND in the checkout, its output to a log.
step() {
  local name=$1; shift
  n=$((n + 1))
  local log
  log=$logs/$(printf '%02d' "$n")-$(echo "$name" | tr -c 'A-Za-z0-9\n' '-').log
  local started=$SECONDS
  if "$@" >"$log" 2>&1; then
    summary "PASS  $(printf '%4d' $((SECONDS - started)))s  $name"
  else
    summary "FAIL  $(printf '%4d' $((SECONDS - started)))s  $name  ($log)"
    failed=1
  fi
}
skip() { summary "SKIP          $1"; }

summary "local CI on $sha (base $base_sha), $(date -u +%Y-%m-%dT%H:%MZ), $(uname -sm)"

# The Linux job's checks that need no Linux.
step "fmt" cargo fmt --all -- --check
step "python: test_compare" python3 scripts/test_compare.py -v
step "python: test_windows_guard" python3 scripts/test_windows_guard.py -v
step "python: test_semver_verdict" python3 scripts/test_semver_verdict.py -v
step "python: test_xvfb_retry" python3 scripts/test_xvfb_retry.py -v
step "python: test_gate" python3 scripts/perf/test_gate.py -v
step "python: test_inline_scopes" python3 scripts/perf/test_inline_scopes.py -v
[ -f scripts/perf/test_sizes.py ] && step "python: test_sizes" python3 scripts/perf/test_sizes.py -v
step "forks" python3 scripts/check_forks.py --offline-ok
step "changelog check" python3 scripts/changelog.py check --base "$base_sha"
if command -v stylua >/dev/null; then
  step "stylua --check" stylua --check .
else
  skip "stylua (not on PATH)"
fi
msrv=$(grep -m1 '^rust-version' Cargo.toml | cut -d'"' -f2)
step "msrv $msrv check (no default features)" \
  cargo "+$msrv" check -p zgx-components -p zgx-desktop -p zgx-script --no-default-features --locked

# The macOS job.
step "clippy 1.99 workspace" cargo +1.99 clippy --workspace --all-targets --locked -- -D warnings
step "clippy 1.99 zgx-script no default features" \
  cargo +1.99 clippy -p zgx-script --no-default-features --all-targets --locked -- -D warnings
# zgx-3d with every feature, its examples and tests among the targets: the
# workspace's clippy builds its default features only, and a lint that
# shows only with them all on (93cc82512's world_bench) reached every lane.
step "clippy 1.99 zgx-3d every feature" \
  cargo +1.99 clippy -p zgx-3d --all-features --all-targets --locked -- -D warnings
# The workspace's clippy unifies zgx-gpu's `view-features` on (zgx-3d asks
# for it): what a UI app builds, without it, is linted here.
step "clippy 1.99 zgx-gpu, zgx-desktop without view-features" \
  cargo +1.99 clippy -p zgx-gpu -p zgx-desktop --all-targets --locked -- -D warnings
# And with each feature alone, its library and tests: a cfg gate that holds
# only beside another feature (`post` without `compute`) passes both with
# every feature and with none. The features are read from the manifest, so
# a new one is checked from the commit that adds it.
for feature in $(cargo metadata --no-deps --format-version 1 --locked 2>/dev/null | python3 -c '
import json, sys
package = [p for p in json.load(sys.stdin)["packages"] if p["name"] == "zgx-3d"][0]
print(" ".join(sorted(f for f in package["features"] if f != "default")))'); do
  step "clippy 1.99 zgx-3d $feature alone" \
    cargo +1.99 clippy -p zgx-3d --no-default-features --features "$feature" --lib --tests --locked -- -D warnings
done
step "script lint" cargo run -q -p zgx-script --bin zgx --locked -- lint \
  crates/zgx-script/examples crates/zgx-desktop/examples crates/zgx-fx/examples/gallery
lsp=${LUAU_LSP:-$(command -v luau-lsp || true)}
if [ -n "$lsp" ]; then
  step "script type check" env LUAU_LSP="$lsp" cargo run -q -p zgx-script --bin zgx --locked -- check \
    crates/zgx-script/examples crates/zgx-desktop/examples crates/zgx-fx/examples/gallery
else
  skip "script type check (no luau-lsp; set LUAU_LSP)"
fi
step "test zgx, components, desktop, workload" \
  cargo test -p zgx -p zgx-components -p zgx-desktop -p zgx-workload --all-targets --locked
step "test zgx-gpu text_scripts, editor_selection, gpu_views" \
  cargo test -p zgx-gpu --test text_scripts --test editor_selection --test gpu_views --locked
# GPU views' extras (Viewport::rest, ray queries, view features) are
# zgx-gpu's `view-features` feature: their tests run with it too.
step "test zgx-gpu view-features" \
  cargo test -p zgx-gpu --features view-features --lib --test gpu_views --locked
step "test zgx-fx" cargo test -p zgx-fx --locked
step "test zgx-audio" cargo test -p zgx-audio --locked
step "test zgx-terminal" cargo test -p zgx-terminal --locked
step "test zgx-script soak" cargo test -p zgx-script --test soak --locked
step "test zgx-script all features" cargo test -p zgx-script --all-features --locked
step "test zgx-run audit" cargo test -p zgx-run --test audit --locked
step "test zgx trace" cargo test -p zgx --features trace --lib --locked -- trace::
step "test zgx no default features" cargo test -p zgx --no-default-features --lib --test editor --test text \
  --locked -- words:: chinese_japanese_or_thai
step "doc tests" cargo test --workspace --doc --locked
step "doc tests zgx-3d gltf" cargo test -p zgx-3d --features gltf --doc --locked
step "cargo doc" cargo doc --workspace --no-deps --locked
step "zgx-3d GPU tests, every feature (Metal)" env ZGX_REQUIRE_GPU=1 cargo test -p zgx-3d --all-features --locked
if command -v ffmpeg >/dev/null; then
  step "test zgx-video" cargo test -p zgx-video --locked
else
  skip "zgx-video (no ffmpeg)"
fi
skip "macOS native validation, component host smoke (windows on screen)"

if [ $linux = 1 ]; then
  lx=${LOCAL_CI_LINUX:?--linux needs LOCAL_CI_LINUX, e.g. "orb -m zgx-linux"}
  ldir=${LOCAL_CI_LINUX_DIR:?--linux needs LOCAL_CI_LINUX_DIR}
  # A clone of this repository at the commit, its own target.
  in_linux() { $lx bash -lc "cd '$ldir/src' && export CARGO_TARGET_DIR='$ldir/target' CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 XDG_RUNTIME_DIR=/tmp && $1"; }
  step "linux: checkout" $lx bash -lc "mkdir -p '$ldir' && { [ -d '$ldir/src/.git' ] || git clone -q --no-checkout '$repo' '$ldir/src'; } && cd '$ldir/src' && git fetch -q '$repo' '$sha' '$base_sha' && git checkout -q --detach '$sha'"
  step "linux: clippy 1.99 workspace (webkitgtk)" in_linux \
    "cargo +1.99 clippy --workspace --all-targets --features zgx-desktop/webkitgtk --locked -- -D warnings"
  step "linux: test zgx, components, desktop, workload, script (scene3d)" in_linux \
    "cargo test -p zgx -p zgx-components -p zgx-desktop -p zgx-workload -p zgx-script --all-targets --features zgx-script/scene3d --locked"
  step "linux: test zgx-terminal, zgx-fx, zgx-audio" in_linux \
    "cargo test -p zgx-terminal --locked && cargo test -p zgx-fx --locked && cargo test -p zgx-audio --locked"
  step "linux: software Vulkan adapter" in_linux "vulkaninfo --summary | grep -E 'deviceName|driverName|driverInfo'"
  step "linux: zgx-gpu (Vulkan)" in_linux "cargo test -p zgx-gpu --locked"
  step "linux: zgx-gpu view-features (Vulkan)" in_linux \
    "cargo test -p zgx-gpu --features view-features --locked --lib --test gpu_views"
  step "linux: zgx-3d GPU tests, every feature (llvmpipe)" in_linux "ZGX_REQUIRE_GPU=1 cargo test -p zgx-3d --all-features --locked"
  skip "linux: native probes (real X11/Wayland windows), the patched Vulkan loader"
  if [ $gate = 1 ]; then
    step "linux: gate against $base_sha" in_linux \
      "ZGX_PERF_DIR='$ldir/perf' python3 scripts/perf/gate.py '$base_sha' --markdown '$ldir/perf.md'"
    # This script's own reader (the commit may predate it).
    step "linux: gate rows read" in_linux "python3 '$repo/scripts/perf/read_gate.py' '$ldir/perf.md'"
    $lx bash -lc "cat '$ldir/perf.md'" > "$logs/gate-table.md" 2>/dev/null
  fi
  if [ $keep = 0 ]; then
    $lx bash -lc "rm -rf '$ldir/target' '$ldir/perf/gate-target'" >/dev/null 2>&1
  fi
fi
skip "windows job (its own machine)"

cd "$repo" || exit 2
if [ $keep = 0 ]; then
  rm -rf "$CARGO_TARGET_DIR"
  git -C "$repo" worktree remove --force "$src" >/dev/null 2>&1
fi
summary "$( [ $failed = 0 ] && echo 'LOCAL CI PASSED' || echo 'LOCAL CI FAILED' ) on $sha"
exit $failed
