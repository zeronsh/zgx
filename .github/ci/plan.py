#!/usr/bin/env python3
"""Which of ci.yml's jobs a change needs, and the runners they will take.

Run by the `plan` job of .github/workflows/ci.yml. It lists the changed
files (a pull request's from the API; a push's from the compare of `before`
and `after`) and gives each one the jobs it can affect, by the first rule in
RULES that matches it. A file no rule matches runs every job, as does a
change it cannot list (too many files, the API failing, a manual run): when
unsure, everything runs.

  - Docs, the site, the changelog and other workflows run nothing; the
    Markdown that tests read (doctests, the reference) runs those tests' jobs.
  - Code compiled for one platform alone (cfg(windows) modules, the Metal
    and Direct3D backends, the vendored platform crates) runs that platform's
    job; tests run the three platform jobs, not perf.
  - Anything else (the crates, vendor/, Cargo.*, CI itself) runs all four.

perf (about 70 minutes, and 42% of the runner time when it ran on every pull
request push) counts the instructions of headless scenes built from the
crates on Linux. On a pull request it runs when the change touches what those
scenes run (rules with "perf": the crates' sources and examples, vendor/,
Cargo.*, scripts/perf; not tests, docs or one platform's code), or with the
`ci:perf` label (add it, then re-run). On main it runs with every run that
tests anything. (The binaries pull requests measure against come from
perf-base.yml: gate_base.py.)

Outputs, to $GITHUB_OUTPUT:

  jobs         JSON object, job -> true/false
  reservation  "linux=N windows=N macos=N": a runner for each job, and one Linux
               runner for the `CI result` job. The admit job carries it in its
               name, where every run in the queue (zgx's and zeron's) reads it.
  pr           the pull request's number (pull requests only), for the admit job name
  any          "true" when some job runs
"""

from __future__ import annotations

import json
import os
import re
import sys
import urllib.request

# Job -> the runner it takes (ci.yml's runs-on).
JOBS = {"linux": "linux", "perf": "linux", "macos": "macos", "windows": "windows"}
# In the shape zeron's plan.py has (suite -> [(job, runner)]), which test_ci_admit.py reads.
SUITE_JOBS = {job: [(job, runner)] for job, runner in JOBS.items()}

ALL = ("linux", "perf", "macos", "windows")
PLATFORMS = ("linux", "macos", "windows")
NOTHING = ()
PERF_LABEL = "ci:perf"

# Built only for Windows: cfg(windows) modules and Windows-only vendored code.
WINDOWS_ONLY = [
    "vendor/accesskit_windows/**",
    "vendor/winit/src/platform_impl/windows/**",
    "vendor/winit/src/platform/windows.rs",
    "vendor/wgpu-hal/src/dx12/**",
    "crates/zgx-desktop/src/*/win32.rs",
    "crates/zgx-gpu/src/dx12_fill.rs",
    "crates/zgx-gpu/src/dxgi.rs",
    "crates/zgx-gpu/src/dxgi_present.rs",
    "crates/zgx-gpu/src/fxc_child.rs",
    "crates/zgx-gpu/src/d3d11_frame.rs",
    "crates/zgx-gpu/src/*.hlsl",
    "crates/zgx-gpu/src/dxbc/**",
    "crates/zgx-script/src/file_events_windows.rs",
    "scripts/d3d11_shaders.py",
]
# Built only for macOS.
MACOS_ONLY = [
    "vendor/accesskit_macos/**",
    "vendor/winit/src/platform_impl/macos/**",
    "vendor/winit/src/platform/macos.rs",
    "vendor/wgpu-hal/src/metal/**",
    "crates/zgx-desktop/src/native_host/macos.rs",
    "crates/zgx-gpu/src/metal_frame.rs",
    "crates/zgx-gpu/src/core_text_glyphs.rs",
    "crates/zgx-gpu/src/*.metal",
    "crates/zgx-script/src/file_events.rs",
    "scripts/macos_*",
    "scripts/macos_*/**",
    "scripts/*.swift",
]
# Built only for Linux: its windowing, accessibility and desktop services,
# which perf's headless scenes never run (but do link: gate_base.py).
LINUX_ONLY = [
    "vendor/accesskit_unix/**",
    "vendor/accesskit_atspi_common/**",
    "vendor/winit/src/platform_impl/linux/**",
    "vendor/winit/src/platform/x11.rs",
    "vendor/winit/src/platform/wayland.rs",
    "crates/zgx-desktop/src/linux_*.rs",
    "crates/zgx-desktop/src/x11_*.rs",
    "crates/zgx-desktop/src/wayland_*.rs",
    "crates/zgx-desktop/src/*/x11.rs",
    "crates/zgx-desktop/src/*/wayland.rs",
    "crates/zgx-desktop/src/*/linux.rs",
    "crates/zgx-desktop/src/native_host/gtk.rs",
    "crates/zgx-desktop/src/native_host/webkitgtk.rs",
    "crates/zgx-desktop/src/portal_parent/**",
    "crates/zgx-script/src/file_events_linux.rs",
]

# (jobs, patterns): a file takes the jobs of the first rule it matches; no match, ALL.
RULES: list[tuple[tuple[str, ...], list[str]]] = [
    # CI itself: everything, so a change to it is tried in full.
    (ALL, [".github/workflows/ci.yml", ".github/ci/**"]),
    # Nothing this workflow builds, tests or reads.
    (NOTHING, [
        "site/**", "website/**", "changelog.d/**", "CHANGELOG.md", "LICENSE", ".gitignore", ".zed/**",
        ".github/workflows/site.yml", ".github/workflows/changelog.yml", ".github/workflows/release.yml",
        ".github/workflows/d3d12-debug-layer.yml", ".github/workflows/ci-caches.yml",
        ".github/workflows/perf-base.yml",
        "docs/results/**", "docs/platform-validation/**", "docs/performance/**", "scripts/**/*.md",
        # Comparisons outside the workspace and the fuzz crate: no job builds them.
        "comparisons/gpui/**", "comparisons/gpui-fast/**", "comparisons/gpui-fast-micro/**",
        "comparisons/quickgui/**", "comparisons/mygo/**", "comparisons/rune/**", "fuzz/**",
    ]),
    # Markdown compiled as doctests (`#[doc = include_str!]`): `cargo test --doc` runs on Linux and macOS.
    (("linux", "macos"), ["README.md", "docs/3d.md", "docs/composition.md", "docs/i18n.md",
                          "docs/styling.md", "docs/window-controls.md"]),
    # Also read by zgx-script's reference tests (Linux, and Windows' reference_docs::).
    (PLATFORMS, ["docs/getting-started.md"]),
    (("linux", "windows"), ["docs/reference/**"]),
    # zgx-script's ui test compares components.png.
    (("linux",), ["docs/images/**"]),
    (("windows",), WINDOWS_ONLY),
    (("macos",), MACOS_ONLY),
    (("linux",), LINUX_ONLY),
    # The Linux job's packages, Luau formatting and Vulkan loader; perf installs with apt-install.sh too.
    (("linux",), [".github/linux-packages.txt", "stylua.toml", ".styluaignore", "scripts/build_vulkan_loader.sh"]),
    (("linux", "perf"), [".github/apt-install.sh", "scripts/perf/**"]),
    # The probes and checks the Linux and macOS jobs run (scripts/linux_smokes.py, component_host_smoke.py...).
    (("linux", "macos"), ["scripts/**"]),
    # Tests: each platform job builds and runs them; perf's scenes are examples, not tests.
    (PLATFORMS, ["crates/*/tests/**", "comparisons/demo/**"]),
    # Markdown nothing compiles (Markdown inside a crate's sources or templates may be include_str!'d: code).
    (NOTHING, ["docs/**", "*.md", "crates/*/README.md", "vendor/**/*.md", "comparisons/**/*.md"]),
]


def _glob(pattern: str) -> re.Pattern:
    out, i = "", 0
    while i < len(pattern):
        if pattern.startswith("**/", i):
            out, i = out + "(?:.*/)?", i + 3
        elif pattern.startswith("**", i):
            out, i = out + ".*", i + 2
        elif pattern[i] == "*":
            out, i = out + "[^/]*", i + 1
        elif pattern[i] == "?":
            out, i = out + "[^/]", i + 1
        else:
            out, i = out + re.escape(pattern[i]), i + 1
    return re.compile(out + r"\Z")


_COMPILED = [(jobs, [_glob(p) for p in patterns]) for jobs, patterns in RULES]
_LINUX_ONLY = [_glob(p) for p in LINUX_ONLY]


def jobs_for(path: str) -> tuple[str, ...]:
    """The jobs a change to `path` can affect."""
    for jobs, patterns in _COMPILED:
        if any(p.match(path) for p in patterns):
            return jobs
    return ALL


def perf_builds(path: str) -> bool:
    """Whether `path` can change perf's binaries or what they run: what perf
    measures, and the Linux-only code linked in beside it. Binaries built from
    two commits differing in nothing like this are the same (gate_base.py)."""
    return "perf" in jobs_for(path) or any(p.match(path) for p in _LINUX_ONLY)


def select(paths: list[str] | None, event: str, labels: set[str] = frozenset()) -> tuple[dict[str, bool], dict[str, str]]:
    """Job -> whether it runs, and job -> why (the first file that needs it)."""
    if paths is None:
        return {job: True for job in JOBS}, {job: "changed files unknown: everything runs" for job in JOBS}
    jobs = {job: False for job in JOBS}
    why: dict[str, str] = {}
    for path in paths:
        for job in jobs_for(path):
            if not jobs[job]:
                jobs[job], why[job] = True, f"`{path}`"
    if event == "pull_request":
        if PERF_LABEL in labels and not jobs["perf"]:
            jobs["perf"], why["perf"] = True, f"label `{PERF_LABEL}`"
    elif any(jobs.values()) and not jobs["perf"]:
        # main: every tested commit gets measured, and keeps its binaries for pull requests.
        jobs["perf"], why["perf"] = True, "main: with every run"
    return jobs, why


def jobs_to_run(suites: dict[str, bool], skip: dict[str, list[str]] | None = None) -> dict[str, list[tuple[str, str]]]:
    skip = skip or {}
    return {suite: [job for job in SUITE_JOBS[suite] if job[0] not in skip.get(suite, ())]
            for suite, enabled in suites.items() if enabled}


def reservation(running: dict[str, list[tuple[str, str]]], event: str = "", ref: str = "") -> dict[str, int]:
    counts = {"linux": 1, "windows": 0, "macos": 0}  # 1: the `CI result` job
    for jobs in running.values():
        for _, runner in jobs:
            counts[runner] += 1
    return counts


def format_reservation(counts: dict[str, int]) -> str:
    return " ".join(f"{k}={counts[k]}" for k in ("linux", "windows", "macos"))


# GitHub API --------------------------------------------------------------------------------------

def api(path: str):
    url = os.environ.get("GITHUB_API_URL", "https://api.github.com") + path
    request = urllib.request.Request(url, headers={
        "Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    })
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def pull_request_files(repo: str, number: int, expected: int) -> list[str] | None:
    files: list[dict] = []
    page = 1
    while True:
        batch = api(f"/repos/{repo}/pulls/{number}/files?per_page=100&page={page}")
        files.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    # The API lists at most 3000 files: run everything rather than guess.
    return paths_of(files) if len(files) >= expected else None


def compare_files(repo: str, base: str, head: str) -> list[str] | None:
    if not base or set(base) == {"0"}:
        return None  # a new branch: nothing to compare with
    files = api(f"/repos/{repo}/compare/{base}...{head}").get("files")
    # compare lists at most 300 files.
    if files is None or len(files) >= 300:
        return None
    return paths_of(files)


def paths_of(files: list[dict]) -> list[str]:
    """Each file's path, and a renamed file's old path too (it left that place)."""
    paths = []
    for f in files:
        paths.append(f["filename"])
        if f.get("previous_filename"):
            paths.append(f["previous_filename"])
    return paths


def main() -> int:
    event = os.environ["GITHUB_EVENT_NAME"]
    repo = os.environ["GITHUB_REPOSITORY"]
    with open(os.environ["GITHUB_EVENT_PATH"]) as f:
        payload = json.load(f)

    paths: list[str] | None = None
    labels: set[str] = set()
    try:
        if event == "pull_request":
            # The pull request as it is now, not as in the event: labels added before a re-run count.
            pr = api(f"/repos/{repo}/pulls/{payload['pull_request']['number']}")
            labels = {label["name"] for label in pr.get("labels", [])}
            paths = pull_request_files(repo, pr["number"], pr.get("changed_files", 0))
        elif event == "push":
            paths = compare_files(repo, payload.get("before", ""), payload.get("after", ""))
        # workflow_dispatch and anything else: everything.
    except (OSError, KeyError, ValueError) as error:  # URLError, HTTPError, timeouts, bad JSON
        print(f"::warning::could not list the changed files ({error}); running every job")
        paths = None

    jobs, why = select(paths, event, labels)
    counts = reservation(jobs_to_run(jobs))
    out = {
        "jobs": json.dumps(jobs, separators=(",", ":")),
        "reservation": format_reservation(counts),
        "pr": str(payload["pull_request"]["number"]) if event == "pull_request" else "",
        "any": "true" if any(jobs.values()) else "false",
    }
    with open(os.environ["GITHUB_OUTPUT"], "a") as f:
        for key, value in out.items():
            f.write(f"{key}={value}\n")

    lines = ["### CI plan", "", f"Changed files: {'unknown' if paths is None else len(paths)}", "",
             "| job | runs | because of |", "|---|---|---|"]
    for job, on in jobs.items():
        lines.append(f"| {job} | {'yes' if on else '–'} | {why.get(job, 'not affected')} |")
    lines += ["", f"Reserved runners: `{out['reservation']}`"]
    if event == "pull_request" and not jobs["perf"]:
        lines += ["", f"perf runs on pull requests that change what it measures. Add the `{PERF_LABEL}` "
                  "label and re-run to measure this one anyway."]
    text = "\n".join(lines)
    print(text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(text + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
