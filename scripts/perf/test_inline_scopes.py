"""inline_scopes.py's reading of MIR and its lines: which functions' scopes
and sizes it finds, and which changes fail (the counts of #1740 and #1824).

    python3 scripts/perf/test_inline_scopes.py -v
"""
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import inline_scopes  # noqa: E402

MIR = """\
fn scene::<impl at crates/zgx/src/scene.rs:3150:1: 3150:11>::walk(_1: &Scene) -> () {
    debug self => _1;
    scope 1 (inlined Scene::node) {
        scope 2 (inlined <Node as Deref>::deref) {
        }
    }
    scope 3 (inlined Scene::node) {
    }
    bb0: {
        return;
    }
}

fn scene::<impl at crates/zgx/src/scene.rs:9000:1: 9000:11>::walk(_1: &Scene) -> () {
    bb0: {
        return;
    }
}

fn batches(_1: &[u32]) -> usize {
    bb0: {
        return;
    }
}
"""


class ReadTests(unittest.TestCase):
    def test_scopes_by_function_and_impl_lines_dropped(self):
        found, lines = inline_scopes.scopes(MIR)
        walk = "scene::<impl in crates/zgx/src/scene.rs>::walk"
        self.assertEqual(found[walk], ["Scene::node", "<Node as Deref>::deref", "Scene::node"])
        # A second function of the same name is told apart, not merged.
        self.assertEqual(found[walk + "#2"], [])
        self.assertEqual(found["batches"], [])
        self.assertEqual(lines["batches"], 4)

    def test_callees_gained_and_lost_by_count(self):
        gained, lost = inline_scopes.changed(["a", "a", "b"], ["a", "c"])
        self.assertEqual(gained, "c")
        self.assertEqual(lost, "a, b")


class VerdictTests(unittest.TestCase):
    def test_1740_lost_scopes_fail(self):
        self.assertIn("inlines 411 scopes, 520 recorded", inline_scopes.verdict(520, 5495, 411, 5583))

    def test_1824_grown_body_fails(self):
        self.assertIn("grew to 136 lines from 69", inline_scopes.verdict(1, 69, 5, 136))

    def test_small_changes_pass(self):
        # Within the slack of scopes, and growth under either line.
        self.assertEqual(inline_scopes.verdict(520, 5495, 518, 5495), "")
        self.assertEqual(inline_scopes.verdict(0, 39, 0, 69), "")
        self.assertEqual(inline_scopes.verdict(300, 3000, 300, 3600), "")
        # Gaining scopes is no failure.
        self.assertEqual(inline_scopes.verdict(506, 5366, 520, 5400), "")


if __name__ == "__main__":
    unittest.main()
