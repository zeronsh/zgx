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



class FairTests(unittest.TestCase):
    def fair_counts(self, name, package):
        asked = []

        def count(binary, env, checkout=None, placement=None, out='/dev/null', fair=False):
            asked.append(fair)
            return 100

        with patch.object(gate, 'count', side_effect=count):
            gate.compared(name, package, {}, ('base', None), ('head', None))
        return asked

    def test_the_span_scenes_alone_are_counted_under_fair_scheduling(self):
        self.assertEqual(self.fair_counts('spans-follow', 'zgx-script'), [True, True])
        self.assertEqual(self.fair_counts('script-update', 'zgx-script'), [True, True])
        self.assertEqual(self.fair_counts('list-estimate', 'zgx-script'), [False, False])
        self.assertEqual(self.fair_counts('heavy-both', 'zgx-desktop'), [False, False])

    def test_valgrind_is_asked_for_it(self):
        with patch.object(gate, 'warm'), patch.object(gate, 'environment', return_value={}), \
                patch.object(gate.subprocess, 'run') as run:
            run.return_value = SimpleNamespace(returncode=0, stdout='', stderr='I   refs: 1,000\nsame_run: ok')
            gate.count(pathlib.Path('bench'), {}, fair=True)
            gate.count(pathlib.Path('bench'), {})
        commands = [call.args[0] for call in run.call_args_list]
        self.assertIn('--fair-sched=yes', commands[0])
        self.assertNotIn('--fair-sched=yes', commands[1])


class ReadsTests(unittest.TestCase):
    def test_a_scene_naming_a_setting_runs_only_where_its_bench_reads_it(self):
        with tempfile.TemporaryDirectory() as root:
            checkout = pathlib.Path(root)
            examples = checkout / 'crates' / 'zgx-terminal' / 'examples'
            examples.mkdir(parents=True)
            bench = examples / 'terminal_bench.rs'
            env = {'LINES': '1000', 'PER_FRAME': '1', 'READS': 'PER_FRAME'}
            bench.write_text('std::env::var("LINES")')
            self.assertFalse(gate.runs_scene(checkout, env, 'zgx-terminal', 'terminal_bench'))
            bench.write_text('std::env::var("LINES"); std::env::var("PER_FRAME")')
            self.assertTrue(gate.runs_scene(checkout, env, 'zgx-terminal', 'terminal_bench'))
            # Without `READS`, as every other scene: run.
            bench.write_text('')
            self.assertTrue(gate.runs_scene(checkout, {'LINES': '1000'}, 'zgx-terminal', 'terminal_bench'))

    def test_the_trickling_terminal_is_new_to_a_bench_without_per_frame(self):
        name, package, example, env = next(s for s in gate.SCENES if s[0] == 'terminal-slow')
        self.assertEqual(env['READS'], 'PER_FRAME')
        self.assertTrue(gate.runs_scene(gate.ROOT, env, package, example))


class AllocationsTests(unittest.TestCase):
    """The allocations table: per a scene's tick, frame or op, a scene whose
    allocator bypasses glibc said to be uncounted, and the counter preloaded
    after `same_run.so` in a native run of its own."""

    def counts(self, allocations, reallocations=0, bytes_=0, peak=0):
        return {'allocations': allocations, 'reallocations': reallocations, 'frees': allocations,
                'bytes': bytes_, 'peak': peak}

    def test_a_scene_with_ticks_is_reported_per_tick(self):
        env = {'ZGX_MODE': 'scroll', 'TICKS': '300'}
        self.assertEqual(gate.unit_of(env), ('TICKS', 300))
        self.assertIsNone(gate.unit_of({'APP': 'mail'}))
        # 33 more a tick over 300 ticks: +33.0 a tick.
        rows = [('heavy-scroll', env, self.counts(30_000, 300, 900_000, 5_000),
                 self.counts(39_900, 600, 1_200_000, 5_064))]
        line = gate.allocation_lines(rows)[-1]
        self.assertIn('| heavy-scroll | tick | 100.0 | 133.0 | +33.0 (+33.00%) | +1.0 | +1,000 | +64 |', line)

    def test_a_scene_without_a_length_is_reported_per_run(self):
        rows = [('app-mail', {'APP': 'mail'}, self.counts(50_000), self.counts(50_010))]
        self.assertIn('| app-mail | run | 50,000.0 | 50,010.0 | +10.0 (+0.02%) |', gate.allocation_lines(rows)[-1])

    def test_an_allocator_bypassing_glibc_is_not_read_as_zero(self):
        rows = [('heavy-both', {'TICKS': '300'}, self.counts(12), self.counts(12))]
        self.assertIn('not counted: its allocator bypasses glibc', gate.allocation_lines(rows)[-1])

    def test_a_failed_run_and_a_new_scene_say_so(self):
        failed = [('mount', {}, {'failed': True}, self.counts(5_000))]
        self.assertIn('the run failed', gate.allocation_lines(failed)[-1])
        new = [('mount', {}, None, self.counts(5_000))]
        self.assertIn('| mount | run | (new) | 5,000.0 |', gate.allocation_lines(new)[-1])

    def test_the_counter_runs_natively_after_same_run(self):
        with tempfile.TemporaryDirectory() as root:
            work = pathlib.Path(root)
            seen = {}

            def run(command, env=None, **_):
                seen['command'], seen['env'] = command, env
                pathlib.Path(env['ZGX_ALLOC_COUNT_OUT']).write_text('100 2 98 4096 2048\n')
                return SimpleNamespace(returncode=0, stdout='', stderr='')

            with patch.object(gate, 'WORK', work), \
                    patch.object(gate, 'same_run', return_value=work / 'same_run.so'), \
                    patch.object(gate, 'count_allocs', return_value=work / 'count_allocs.so'), \
                    patch.object(gate, 'WARM', {work / 'bench'}), \
                    patch.object(gate.subprocess, 'run', side_effect=run):
                counts = gate.allocations(work / 'bench', {'TICKS': '3'}, None)
        self.assertEqual(seen['command'], [str(work / 'bench')])
        self.assertEqual(seen['env']['LD_PRELOAD'], f'{work / "same_run.so"} {work / "count_allocs.so"}')
        self.assertEqual(counts, {'allocations': 100, 'reallocations': 2, 'frees': 98, 'bytes': 4096,
                                  'peak': 2048})


if __name__ == '__main__':
    unittest.main()
