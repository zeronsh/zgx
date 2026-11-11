"""plan.py's rules and the jobs it selects, and admit.py being zeron's."""

import hashlib
import pathlib
import re
import subprocess
import sys
import unittest

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))

import plan  # noqa: E402

NONE = ()
ALL = plan.ALL


class RuleTests(unittest.TestCase):
    def check(self, cases):
        for path, jobs in cases:
            self.assertEqual(set(plan.jobs_for(path)), set(jobs), path)

    def test_nothing(self):
        self.check([
            ("site/index.html", NONE), ("website/package.json", NONE), ("CHANGELOG.md", NONE),
            ("changelog.d/1234.md", NONE), ("docs/layout.md", NONE), ("docs/results/x/report.json", NONE),
            ("docs/platform-validation/linux/run.log", NONE), ("AGENTS.md", NONE), ("CLAUDE.md", NONE),
            ("crates/zgx-gpu/README.md", NONE), ("vendor/winit/ZGX_PATCH.md", NONE),
            ("scripts/perf/README.md", NONE), (".github/workflows/release.yml", NONE),
            (".github/workflows/site.yml", NONE),
            (".github/workflows/perf-base.yml", NONE),
            ("comparisons/gpui/src/main.rs", NONE), ("fuzz/fuzz_targets/parse.rs", NONE),
        ])

    def test_markdown_the_tests_read(self):
        self.check([
            ("README.md", ("linux", "macos")), ("docs/styling.md", ("linux", "macos")),
            ("docs/3d.md", ("linux", "macos")), ("docs/getting-started.md", plan.PLATFORMS),
            ("docs/reference/state.md", ("linux", "windows")), ("docs/images/components.png", ("linux",)),
            # Markdown inside a crate's sources or templates may be compiled in (scaffold.rs).
            ("crates/zgx-script/templates/common/README.md", ALL),
        ])

    def test_one_platform(self):
        self.check([
            ("crates/zgx-desktop/src/popup_window/win32.rs", ("windows",)),
            ("crates/zgx-gpu/src/dx12_fill.rs", ("windows",)),
            ("crates/zgx-gpu/src/draw.hlsl", ("windows",)),
            ("crates/zgx-gpu/src/dxbc/draw_vs.dxbc", ("windows",)),
            ("vendor/winit/src/platform_impl/windows/window.rs", ("windows",)),
            ("vendor/accesskit_windows/src/node.rs", ("windows",)),
            ("crates/zgx-gpu/src/metal_frame.rs", ("macos",)),
            ("crates/zgx-gpu/src/draw.metal", ("macos",)),
            ("crates/zgx-desktop/src/native_host/macos.rs", ("macos",)),
            ("vendor/winit/src/platform_impl/macos/view.rs", ("macos",)),
            ("scripts/macos_native_validation.py", ("macos",)),
            ("scripts/framepace.swift", ("macos",)),
            ("crates/zgx-desktop/src/x11_startup.rs", ("linux",)),
            ("crates/zgx-desktop/src/popup_window/wayland.rs", ("linux",)),
            ("crates/zgx-desktop/src/native_host/gtk.rs", ("linux",)),
            ("vendor/accesskit_unix/src/lib.rs", ("linux",)),
            ("scripts/linux_smokes.py", ("linux", "macos")),
            (".github/linux-packages.txt", ("linux",)),
            (".github/apt-install.sh", ("linux", "perf")),
        ])

    def test_tests_skip_perf(self):
        self.check([
            ("crates/zgx/tests/overlays.rs", plan.PLATFORMS),
            ("crates/zgx-gpu/tests/d3d11_parity.rs", plan.PLATFORMS),
            ("comparisons/demo/src/main.rs", plan.PLATFORMS),
        ])

    def test_core_and_ci_run_everything(self):
        self.check([
            ("crates/zgx/src/layout.rs", ALL), ("crates/zgx-script/src/windows.rs", ALL),
            ("crates/zgx-desktop/examples/heavy_headless.rs", ALL), ("crates/zgx-gpu/build.rs", ALL),
            ("vendor/taffy/src/lib.rs", ALL), ("vendor/wgpu-hal/src/vulkan/device.rs", ALL),
            ("comparisons/workload/src/lib.rs", ALL), ("Cargo.lock", ALL), ("Cargo.toml", ALL),
            (".cargo/config.toml", ALL), (".gitattributes", ALL), ("scripts/perf/gate.py", ("linux", "perf")),
            (".github/workflows/ci.yml", ALL), (".github/ci/plan.py", ALL),
            ("something/new.txt", ALL),  # unknown: everything
        ])

    def test_patterns_still_name_files(self):
        """Each platform pattern matches a tracked file: a rename doesn't leave a rule behind."""
        listed = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True)
        files = listed.stdout.split()
        if listed.returncode != 0 or not any(f.startswith("crates/") for f in files):
            self.skipTest("no full checkout")
        for pattern in plan.WINDOWS_ONLY + plan.MACOS_ONLY + plan.LINUX_ONLY:
            glob = plan._glob(pattern)
            self.assertTrue(any(glob.match(f) for f in files), pattern)

    def test_glob(self):
        self.assertTrue(plan._glob("crates/*/tests/**").match("crates/zgx/tests/a/b.rs"))
        self.assertFalse(plan._glob("crates/*/tests/**").match("crates/zgx/src/tests/b.rs"))
        self.assertTrue(plan._glob("*.md").match("AGENTS.md"))
        self.assertFalse(plan._glob("*.md").match("docs/AGENTS.md"))
        self.assertTrue(plan._glob("vendor/**/*.md").match("vendor/winit/src/changelog/v0.8.md"))


class SelectTests(unittest.TestCase):
    def test_docs_only_runs_nothing(self):
        jobs, _ = plan.select(["docs/layout.md", "site/a.html", "CHANGELOG.md"], "pull_request")
        self.assertFalse(any(jobs.values()))
        self.assertEqual(plan.reservation(plan.jobs_to_run(jobs)), {"linux": 1, "windows": 0, "macos": 0})
        jobs, _ = plan.select(["docs/layout.md"], "push")
        self.assertFalse(any(jobs.values()))  # main too: nothing to measure

    def test_platform_change(self):
        jobs, why = plan.select(["crates/zgx-gpu/src/dx12_fill.rs"], "pull_request")
        self.assertEqual(jobs, {"linux": False, "perf": False, "macos": False, "windows": True})
        self.assertEqual(why["windows"], "`crates/zgx-gpu/src/dx12_fill.rs`")
        self.assertEqual(plan.format_reservation(plan.reservation(plan.jobs_to_run(jobs))),
                         "linux=1 windows=1 macos=0")

    def test_core_change(self):
        jobs, _ = plan.select(["crates/zgx/src/layout.rs", "docs/layout.md"], "pull_request")
        self.assertTrue(all(jobs.values()))
        self.assertEqual(plan.reservation(plan.jobs_to_run(jobs)), {"linux": 3, "windows": 1, "macos": 1})

    def test_perf_on_pull_requests_only_for_what_it_measures_or_the_label(self):
        tests_only = ["crates/zgx/tests/overlays.rs"]
        jobs, _ = plan.select(tests_only, "pull_request")
        self.assertFalse(jobs["perf"])
        jobs, why = plan.select(tests_only, "pull_request", {"ci:perf"})
        self.assertTrue(jobs["perf"])
        self.assertIn("ci:perf", why["perf"])
        jobs, _ = plan.select(["crates/zgx-desktop/src/x11_startup.rs"], "pull_request")
        self.assertFalse(jobs["perf"])  # linked, but no headless scene runs it

    def test_perf_with_every_main_run(self):
        jobs, _ = plan.select(["crates/zgx/tests/overlays.rs"], "push")
        self.assertTrue(jobs["perf"])

    def test_unknown_changes_run_everything(self):
        for event in ("pull_request", "push", "workflow_dispatch"):
            jobs, _ = plan.select(None, event)
            self.assertTrue(all(jobs.values()), event)

    def test_renames_count_both_paths(self):
        paths = plan.paths_of([{"filename": "docs/x.md", "previous_filename": "crates/zgx/src/x.md"}])
        self.assertEqual(paths, ["docs/x.md", "crates/zgx/src/x.md"])

    def test_workflow_jobs_match(self):
        """ci.yml's jobs are plan.JOBS, each gated on the plan and admission, all in the result's needs."""
        text = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        for job, runner in plan.JOBS.items():
            block = re.search(rf"^  {job}:\n((?:    .*\n|\s*\n)*)", text, re.M)
            self.assertIsNotNone(block, job)
            self.assertIn("needs: [plan, admit]", block.group(1), job)
            self.assertIn(f"if: fromJSON(needs.plan.outputs.jobs).{job}", block.group(1), job)
            runs_on = re.search(r"runs-on: (\S+)", block.group(1)).group(1)
            self.assertIn({"linux": "ubuntu", "macos": "macos", "windows": "windows"}[runner], runs_on, job)
        needs = re.search(r"name: CI result\n\s+needs: \[(.*)\]", text).group(1)
        self.assertEqual(set(needs.replace(" ", "").split(",")), {"plan", "admit", *plan.JOBS})


class VendoredTests(unittest.TestCase):
    def test_admit_is_zerons(self):
        """admit.py below its header is byte for byte the zeron commit its header names."""
        lines = (HERE / "admit.py").read_bytes().splitlines(keepends=True)
        header = 1
        while lines[header].startswith(b"#"):
            header += 1
        upstream = re.search(rb"upstream sha256: ([0-9a-f]{64})", b"".join(lines[:header])).group(1).decode()
        body = lines[0] + b"".join(lines[header:])
        self.assertEqual(hashlib.sha256(body).hexdigest(), upstream)


if __name__ == "__main__":
    unittest.main()
