"""gate.py's counting: each binary warmed once, uncounted, before its first
count, each with a cache directory of its own.

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

    def test_a_failed_warm_up_is_the_gates_failure(self):
        failed = SimpleNamespace(returncode=1, stdout='', stderr='boom')
        with patch.object(gate.subprocess, 'run', return_value=failed):
            with self.assertRaisesRegex(gate.Infra, 'warm-up'):
                gate.count(self.head, {})


if __name__ == '__main__':
    unittest.main()
