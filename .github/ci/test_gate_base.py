"""gate_base.py: which of main's kept perf binaries a perf run measures against."""

import pathlib
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))

import gate_base  # noqa: E402


class GateBaseTests(unittest.TestCase):
    def test_choose(self):
        diffs = {("c3", "b"): ["crates/zgx/src/a.rs"], ("c2", "b"): ["docs/x.md", "crates/zgx/tests/t.rs"],
                 ("c1", "b"): []}
        diff = lambda a, b: diffs.get((a, b))  # noqa: E731
        self.assertEqual(gate_base.choose("b", ["c3", "b", "c2"], diff), "b")  # its own first
        self.assertEqual(gate_base.choose("b", ["c3", "c2", "c1"], diff), "c2")  # newest with the same sources
        self.assertIsNone(gate_base.choose("b", ["c3", "unknown"], diff))
        # Linux-only code is linked into the binaries though perf doesn't run it.
        diffs[("c2", "b")] = ["crates/zgx-desktop/src/x11_startup.rs"]
        self.assertEqual(gate_base.choose("b", ["c3", "c2", "c1"], diff), "c1")

    def test_never_an_entry_the_pull_request_saved_itself(self):
        # A pull request's earlier commit saved `gate-bin-b` (or `-c2`) in its own scope: a restore
        # would take it over main's. As if main kept none for that commit.
        diffs = {("c2", "b"): ["docs/x.md"], ("c1", "b"): []}
        diff = lambda a, b: diffs.get((a, b))  # noqa: E731
        own = {"b"}
        self.assertEqual(gate_base.choose("b", ["b", "c2", "c1"], diff, lambda s: s in own), "c2")
        own = {"b", "c2"}
        self.assertEqual(gate_base.choose("b", ["b", "c2", "c1"], diff, lambda s: s in own), "c1")
        own = {"b", "c2", "c1"}
        self.assertIsNone(gate_base.choose("b", ["b", "c2", "c1"], diff, lambda s: s in own))

    def test_pick_checks_the_runs_own_scope(self):
        import contextlib
        import io
        import os
        listed = {"refs/heads/main": ["gate-bin-b", "gate-bin-c1"], "refs/pull/7/merge": ["gate-bin-b"]}
        calls = []

        def fake(path):
            calls.append(path)
            ref = path.split("&ref=")[1].split("&")[0].replace("%2F", "/")
            key = path.split("?key=")[1].split("&")[0]
            return {"actions_caches": [{"key": k, "ref": ref} for k in listed.get(ref, []) if k.startswith(key)]}

        def pick(env, api=fake):
            saved_env, saved_api, saved_argv, saved_changed = dict(os.environ), gate_base.plan.api, sys.argv, \
                gate_base.changed
            os.environ.update({"GITHUB_REPOSITORY": "o/r", **env})
            gate_base.plan.api, sys.argv = api, ["gate_base.py", "pick", "b"]
            gate_base.changed = lambda a, b: []
            out = io.StringIO()
            try:
                with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
                    gate_base.main()
            finally:
                os.environ.clear()
                os.environ.update(saved_env)
                gate_base.plan.api, sys.argv, gate_base.changed = saved_api, saved_argv, saved_changed
            return out.getvalue().split()

        self.assertEqual(pick({"GITHUB_REF": "refs/heads/main"}), ["key=gate-bin-b", "from=b"])
        self.assertFalse(any("refs%2Fpull" in c for c in calls))  # main's own scope is main's
        # The pull request saved gate-bin-b itself: c1's, which nothing perf builds separates from b.
        self.assertEqual(pick({"GITHUB_REF": "refs/pull/7/merge"}), ["key=gate-bin-c1", "from=c1"])
        self.assertEqual(pick({"GITHUB_REF": "refs/pull/8/merge"}), ["key=gate-bin-b", "from=b"])

        def down(path):
            if "refs%2Fpull" in path:
                raise OSError("API down")
            return fake(path)
        self.assertEqual(pick({"GITHUB_REF": "refs/pull/8/merge"}, down), [])  # fail closed: build the base

    def test_adopt(self):
        with tempfile.TemporaryDirectory() as d:
            stamp = pathlib.Path(d) / "built-from"
            self.assertFalse(gate_base.adopt(pathlib.Path(d), "aaa", "bbb"))
            stamp.write_text("aaa\nrustc 1.95.0\nhost: x\n")
            self.assertFalse(gate_base.adopt(pathlib.Path(d), "ccc", "bbb"))
            self.assertTrue(gate_base.adopt(pathlib.Path(d), "aaa", "bbb"))
            self.assertEqual(stamp.read_text(), "bbb\nrustc 1.95.0\nhost: x\n")

    def test_needed(self):
        compare = lambda files: (lambda a, b: files)  # noqa: E731
        self.assertTrue(gate_base.needed("a", "b", compare(["crates/zgx/src/a.rs"])))
        self.assertTrue(gate_base.needed("a", "b", compare(["crates/zgx-desktop/src/x11_startup.rs"])))
        self.assertFalse(gate_base.needed("a", "b", compare(["docs/a.md", "crates/zgx/tests/t.rs",
                                                              "crates/zgx-gpu/src/metal_frame.rs"])))
        self.assertTrue(gate_base.needed("a", "b", compare(None)))  # unknown

        def broken(a, b):
            raise OSError("down")
        self.assertTrue(gate_base.needed("a", "b", broken))

    def test_stamp_format_is_gate_pys(self):
        """adopt rewrites the first line of gate.py's stamp: the commit, then rustc -vV."""
        gate = ROOT / "scripts/perf/gate.py"
        if not gate.exists():
            self.skipTest("no full checkout")
        self.assertIn("return git('rev-parse', 'HEAD') + rustc", gate.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
