"""gate.py's counting: each binary warmed once, uncounted, before its first
count, each with a cache directory of its own; and which cells it explains
by function, within its budget.

    python3 scripts/perf/test_gate.py -v
"""
import pathlib
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import gate  # noqa: E402

COUNTED = SimpleNamespace(returncode=0, stdout='', stderr='same_run: 1 clock reads\n==1== I   refs:      1,234,567\n')
WARMED = SimpleNamespace(returncode=0, stdout='', stderr='')


class WarmTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        work = pathlib.Path(self.work.name)
        self.base = work / 'gate-bin-base' / 'heavy_headless'
        self.head = work / 'gate-bin-head' / 'heavy_headless'
        self.patches = [patch.object(gate, 'WORK', work), patch.object(gate, 'WARM', set()),
                        patch.object(gate, 'same_run', return_value=work / 'same_run.so')]
        for p in self.patches:
            p.start()
        self.calls = []

    def tearDown(self):
        for p in self.patches:
            p.stop()
        self.work.cleanup()

    def run_(self, command, env=None, **_):
        self.calls.append((command, env))
        return COUNTED if 'valgrind' in command else WARMED

    def test_a_binary_runs_once_uncounted_before_its_first_count(self):
        with patch.object(gate.subprocess, 'run', side_effect=self.run_):
            self.assertEqual(gate.count(self.head, {'ZGX_MODE': 'both'}), 1234567)
            self.assertEqual(gate.count(self.head, {'ZGX_MODE': 'stream'}), 1234567)
        commands = [c for c, _ in self.calls]
        self.assertEqual(commands[0], [str(self.head)])
        self.assertTrue(all('valgrind' in c for c in commands[1:]))
        self.assertEqual(len(commands), 3)

    def test_the_warm_up_and_the_count_read_the_same_cache(self):
        with patch.object(gate.subprocess, 'run', side_effect=self.run_):
            gate.count(self.head, {})
        (_, warm), (_, counted) = self.calls
        self.assertEqual(warm['XDG_CACHE_HOME'], counted['XDG_CACHE_HOME'])
        self.assertTrue(pathlib.Path(warm['XDG_CACHE_HOME']).is_dir())

    def test_each_side_has_a_cache_of_its_own_as_long_as_the_other(self):
        with patch.object(gate.subprocess, 'run', side_effect=self.run_):
            gate.count(self.base, {})
            gate.count(self.head, {})
        caches = {env['XDG_CACHE_HOME'] for _, env in self.calls}
        self.assertEqual(len(caches), 2)
        self.assertEqual(len({len(c) for c in caches}), 1)

    def test_a_scenes_tunables_hold_under_every_placement(self):
        env = {'TICKS': '200', 'GLIBC_TUNABLES': 'glibc.malloc.mxfast=0'}
        alone = gate.environment(self.head, env, None)
        placed = gate.environment(self.head, env, None, 'glibc.malloc.top_pad=1048576')
        self.assertEqual(alone['GLIBC_TUNABLES'], 'glibc.malloc.mxfast=0')
        self.assertEqual(placed['GLIBC_TUNABLES'], 'glibc.malloc.mxfast=0:glibc.malloc.top_pad=1048576')
        self.assertNotIn('ZGUI_GLIBC_TUNABLES', placed)

    def test_a_failed_warm_up_is_the_gates_failure(self):
        failed = SimpleNamespace(returncode=1, stdout='', stderr='boom')
        with patch.object(gate.subprocess, 'run', return_value=failed):
            with self.assertRaisesRegex(gate.Infra, 'warm-up'):
                gate.count(self.head, {})


class WorkDirTests(unittest.TestCase):
    def test_each_checkout_has_a_work_directory_of_its_own(self):
        # Two worktrees' gates at once never count each other's binaries.
        one = gate.work_dir('/home/a/gate/wt-one', {})
        two = gate.work_dir('/home/a/gate/wt-two', {})
        self.assertNotEqual(one, two)
        self.assertEqual(one.parent, pathlib.Path('/tmp/zgx-perf'))
        # The same checkout finds it again: its target, its cached base, and
        # what `--stage head` built for `--stage run`.
        self.assertEqual(one, gate.work_dir('/home/a/gate/wt-one', {}))

    def test_zgx_perf_dir_still_chooses_it(self):
        self.assertEqual(gate.work_dir('/home/a/gate/wt-one', {'ZGX_PERF_DIR': '/tmp/mine'}),
                         pathlib.Path('/tmp/mine'))



def row(name, delta):
    return (name, 1_000_000, 1_000_000, delta, [delta])


class ExplainTests(unittest.TestCase):
    def test_only_cells_past_the_floor_and_their_spread_largest_first(self):
        rows = [row('quiet', 0.01), row('moved', 0.03), row('most', 0.5), row('shrank', -0.4),
                row('noisy', 0.1), row('new', None)]
        chosen = gate.worth_explaining(rows, floor=0.02, spread={'noisy': 0.2}, most=10)
        self.assertEqual([r[0] for r in chosen], ['most', 'moved'])
        self.assertEqual([r[0] for r in gate.worth_explaining(rows, floor=0.02, spread={}, most=1)], ['most'])

    def test_explaining_stops_at_its_budget_and_says_what_it_left(self):
        rows = [row('a', 0.5), row('b', 0.4), row('c', 0.3)]
        scenes = {name: (name, 'zgx', 'heavy_headless', {}) for name in 'abc'}
        times = iter([0.0, 0.0, 30.0, 700.0, 700.0])
        with tempfile.TemporaryDirectory() as out, \
                patch.object(gate, 'FUNCTIONS', out), \
                patch.object(gate, 'count', side_effect=OSError('not run here')):
            table = gate.by_function(rows, scenes, None, budget=600, clock=lambda: next(times))
        self.assertTrue(any('(out of time)' in line and 'c' in line for line in table), table)


if __name__ == '__main__':
    unittest.main()
