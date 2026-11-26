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



class LayoutTests(unittest.TestCase):
    """The heap layout rows: the base's own range says what is layout, and a
    change's own work is never taken for it (each with the mutant that would
    slip past a weaker rule)."""

    # What the base asks: the same in every test but where one says.
    ASKED = {'memcpy': (10_000, 500_000), 'memmove': (100, 5_000), 'memset': (300, 9_000),
             'memcmp': (300_000, 2_000_000), 'malloc': (20_000, 800_000), 'realloc': (900, 60_000)}

    def told(self, delta=0.10, groups=None, head=None, shifts=(0, 400_000), total=200_000, name='cell', max_=8,
             clock=None, parts=None):
        before = 100_000_000
        base = list(shifts) + [0] * (len(gate.SHIFTS) - len(shifts))
        found = {name: ({'zgx': 1_000, 'luau': 500, 'libc': total - 1_500, 'rust': 0, 'linker': 0, 'other': 0,
                         **(groups or {})}, total, *([parts] if parts else []))}
        scenes = {name: (name, 'zgx-script', 'bench', {})}
        counts = iter([before + extra for extra in base])
        sides = iter([self.ASKED, {**self.ASKED, **(head or {})}])
        rows = [(name, before, before + total, delta, [delta])]
        with patch.object(gate, 'LAYOUT_MAX', max_):
            return gate.layout_rows(
                rows, found, scenes, None, clock=clock or (lambda: 0.0),
                counted=lambda *a, **k: next(counts), asked=lambda *a: next(sides), changes={name: total})[0][2]

    def test_a_rise_inside_the_bases_own_range_is_layout(self):
        self.assertTrue(self.told().startswith('layout:'), self.told())
        self.assertIn('over 8 shifts (300,000, 0.3000%; the rise 200,000)', self.told(shifts=(0, 3_000_000)))

    def test_the_rise_is_the_medians_own_difference_kept_exactly(self):
        # Three placements, the middle percent's own count difference, not a
        # percent turned back into one (its base is not the row's).
        # (`compared` counts the head first, then the base; each placement's
        # base, then its head.)
        counts = iter([1_000_400, 1_000_000,   # the row's own: +0.04%
                       2_000_000, 2_001_000,   # a placement: +0.05%
                       1_000_000, 1_000_300])  # another: +0.03%
        with patch.object(gate, 'count', side_effect=lambda *a, **k: next(counts)), \
                patch.object(gate, 'CHANGES', {}):
            _, _, median, _ = gate.compared('app-mail', 'zgx-script', {}, ('b', None), ('h', None))
            self.assertAlmostEqual(median, 0.04)
            self.assertEqual(gate.CHANGES['app-mail'], 400)

    def test_a_rise_equal_to_the_range_is_inside_it(self):
        # md-stream on #2463: a rise and a range that print alike are compared
        # in instructions, not as rounded percents.
        self.assertTrue(self.told(delta=0.25, total=250_000, shifts=(0, 250_000)).startswith('layout:'))
        self.assertIn("past the base's own range", self.told(delta=0.25001, total=250_010, shifts=(0, 250_000)))

    def test_a_rise_past_it_is_not(self):
        self.assertIn("past the base's own range", self.told(shifts=(0, 50_000)))

    def test_zgxs_own_work_is_the_changes_however_wide_the_range(self):
        # +0.05% of zgx: the mutant a wide range would hide.
        self.assertIn("the change's: zgx", self.told(groups={'zgx': 50_000}, total=60_000, delta=0.06))
        self.assertIn("the change's: luau", self.told(groups={'luau': 50_000}, total=60_000, delta=0.06))
        # What fell is not work.
        self.assertTrue(self.told(groups={'zgx': -90_000}).startswith('layout:'))

    def test_a_rise_in_other_alone_is_compared_by_the_range_and_printed(self):
        # The collector's unattributed frames move with the heap, as libc's do.
        why = self.told(groups={'other': 120_000}, total=200_000)
        self.assertTrue(why.startswith('layout:'), why)
        self.assertIn('other +120,000', why)
        self.assertIn("past the base's own range", self.told(groups={'other': 120_000}, shifts=(0, 50_000)))

    # Luau by part (`cgdiff.luau_parts`): a cell of 100,000,000, 0.02% being 20,000.
    def luau(self, **parts):
        full = {'parse': 0, 'tables': 0, 'interpreter': 0, 'api': 0, 'rest': 0, **parts}
        return self.told(groups={'luau': sum(full.values())}, parts=full)

    def test_luaus_compiler_moving_is_compared_by_the_range(self):
        why = self.luau(parse=60_000)
        self.assertTrue(why.startswith('layout:'), why)
        self.assertIn('luau parse +60,000', why)

    def test_luaus_tables_moving_with_their_callers_still_are_too(self):
        why = self.luau(tables=50_000, interpreter=5_000, api=5_000)
        self.assertTrue(why.startswith('layout:'), why)
        self.assertIn('luau tables +50,000', why)

    def test_more_lookups_from_the_interpreter_are_the_changes(self):
        # Each lookup costs its caller too: the tables are then the change's.
        self.assertIn("the change's: luau", self.luau(tables=50_000, interpreter=25_000))

    def test_more_lookups_through_the_c_api_are_the_changes(self):
        self.assertIn("the change's: luau", self.luau(tables=50_000, api=25_000))

    def test_the_rest_of_luau_stays_the_changes(self):
        # The collector's named work, the VM's: ours, as zgx's is.
        self.assertIn("the change's: luau", self.luau(parse=60_000, rest=25_000))

    def test_luaus_parts_are_read_from_its_function_names(self):
        sys.path.insert(0, str(pathlib.Path(__file__).parent))
        import cgdiff
        names = {'Luau::Parser::parseNameExpr(char const*)': (100, 160), 'Luau::Compiler::compileExpr(x)': (50, 80),
                 'Luau::detail::DenseHashTable<A, B>::find(x) const': (10, 20),
                 'luaH_getp(LuaTable*, void*, int)': (10, 40), 'mainposition(LuaTable const*, lua_TValue const*)': (5, 15),
                 'void luau_execute<false>(lua_State*)': (1000, 1003), 'lua_rawgeti(lua_State*, int, int)': (7, 9),
                 'propagatemark(global_State*)': (40, 90), 'luaV_gettable(x)': (20, 25),
                 'zgx::scene::Scene::flush': (5, 500)}
        with tempfile.TemporaryDirectory() as out:
            for side, at in (('base', 0), ('head', 1)):
                # As cachegrind has them: the collector's marking with no file
                # it knows ('other', apart), zgx's in its own.
                file = lambda n: '???' if n.startswith('propagatemark') else 'src/scene.rs' if 'zgx' in n else 'luau/VM/src/x.cpp'
                lines = ['events: Ir'] + [f'fl={file(n)}\nfn={n}\n1 {v[at]}' for n, v in names.items()]
                (pathlib.Path(out) / side).write_text('\n'.join(lines) + '\n')
            parts = cgdiff.luau_parts(pathlib.Path(out) / 'base', pathlib.Path(out) / 'head')
        self.assertEqual(parts, {'parse': 100, 'tables': 40, 'interpreter': 3, 'api': 2, 'rest': 5})

    def test_one_percent_more_compares_at_the_same_cost_of_zgx_is_the_changes(self):
        self.assertIn('memcmp +3,000 calls', self.told(head={'memcmp': (303_000, 2_020_000)}))

    def test_the_same_calls_copying_one_percent_longer_is_the_changes(self):
        # A struct that grew: no call more, each a little longer.
        why = self.told(head={'memcpy': (10_000, 505_000)})
        self.assertIn('memcpy +0 calls +5,000 bytes', why)

    def test_a_call_or_two_as_it_starts_is_not_work(self):
        self.assertTrue(self.told(head={'memcmp': (300_001, 2_000_012)}).startswith('layout:'))

    def test_the_changes_own_allocations_are_printed_to_be_explained(self):
        why = self.told(head={'malloc': (20_600, 900_000)})
        self.assertTrue(why.startswith("layout, given the change's own malloc +600 (+100,000 bytes)"), why)

    def test_past_the_ceiling_a_person_reads_it(self):
        self.assertIn('read it', self.told(delta=0.31, shifts=(0, 90_000_000)))

    def test_a_range_is_no_wider_than_the_ceiling(self):
        # A range past 0.3% reads as 0.3%; a rise past it is a person's to read
        # first, whatever the range.
        self.assertIn('(300,000, 0.3000%; the rise 290,000)', self.told(delta=0.29, total=290_000, shifts=(0, 500_000)))
        self.assertIn('read it', self.told(delta=0.30001, total=300_010, shifts=(0, 500_000)))

    def test_the_widest_cells_are_shifted_and_the_rest_say_so(self):
        rows = [(n, 100_000_000, 0, d, [d]) for n, d in (('small', 0.05), ('wide', 0.2))]
        found = {n: ({'zgx': 0, 'luau': 0, 'libc': 1, 'rust': 0, 'linker': 0, 'other': 0}, 50_000) for n in ('small', 'wide')}
        scenes = {n: (n, 'zgx', 'bench', {}) for n in found}
        with patch.object(gate, 'LAYOUT_MAX', 1):
            told = gate.layout_rows(rows, found, scenes, None, clock=lambda: 0.0,
                                    counted=lambda *a, shift=None, **k: 100_000_000 + (0 if shift == 16 else 250_000),
                                    asked=lambda *a: self.ASKED)
        self.assertEqual([t[0] for t in told], ['wide', 'small'])
        self.assertTrue(told[0][2].startswith('layout'), told)
        self.assertEqual(told[1][2], 'not shifted')

    def test_it_stops_at_its_budget(self):
        self.assertIn('not shifted', self.told(clock=iter([0.0, 10_000.0]).__next__))

    def test_a_cell_that_shrank_or_is_new_is_not_asked(self):
        rows = [('shrank', 100_000_000, 0, -0.2, [-0.2]), ('new', None, 1, None, [])]
        self.assertEqual(gate.layout_rows(rows, {'shrank': ({'zgx': 0}, -5)}, {}, None, clock=lambda: 0.0), [])

    def test_a_shift_preloads_one_block_of_four_digits(self):
        seen = []
        with tempfile.TemporaryDirectory() as work, \
                patch.object(gate, 'WORK', pathlib.Path(work)), patch.object(gate, 'WARM', set()), \
                patch.object(gate, 'same_run', return_value=pathlib.Path('same_run.so')), \
                patch.object(gate, 'heap_shift', return_value='heap_shift.so'):
            def run(command, env=None, **_):
                seen.append(env)
                return COUNTED
            with patch.object(gate.subprocess, 'run', side_effect=run):
                gate.count(pathlib.Path(work) / 'bench', {}, shift=16)
                gate.count(pathlib.Path(work) / 'bench', {}, shift=3600)
                gate.count(pathlib.Path(work) / 'bench', {})
        # (The first of each pair is the warm-up, uncounted.)
        shifted = [env for env in seen if env and 'ZGX_GATE_SHIFT' in env]
        self.assertEqual([env['ZGX_GATE_SHIFT'] for env in shifted], ['0016', '3600'])
        self.assertTrue(all(env['LD_PRELOAD'].startswith('heap_shift.so same_run.so') for env in shifted), shifted)


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

    def test_an_app_scene_naming_a_setting_runs_only_where_apps_reads_it(self):
        with tempfile.TemporaryDirectory() as root:
            checkout = pathlib.Path(root)
            examples = checkout / 'crates' / 'zgx-script' / 'examples'
            (examples / 'support').mkdir(parents=True)
            (examples / 'list_state_bench.rs').write_text('apps::run()')
            apps = examples / 'support' / 'apps.rs'
            env = {'APP': 'stream', 'CLOCK': 'frames', 'READS': 'CLOCK'}
            apps.write_text('("stream", stream)')
            self.assertFalse(gate.runs_scene(checkout, env, 'zgx-script', 'list_state_bench'))
            apps.write_text('("stream", stream) std::env::var("CLOCK")')
            self.assertTrue(gate.runs_scene(checkout, env, 'zgx-script', 'list_state_bench'))

    def test_the_streaming_cells_are_new_to_a_bench_without_its_clock(self):
        for cell in ('md-stream-fade', 'md-stream-fade-between'):
            name, package, example, env = next(s for s in gate.SCENES if s[0] == cell)
            self.assertEqual((env['CLOCK'], env['READS']), ('frames', 'CLOCK'))
            self.assertTrue(gate.runs_scene(gate.ROOT, env, package, example))

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



class OutputTests(unittest.TestCase):
    """The output table: the same paint and damage, or the first flush
    where paint or damage parts, and what a scene without a fingerprint
    says."""

    def test_the_same_output_reads_same(self):
        frames = [('a1', 'd1'), ('a2', 'd2')]
        self.assertEqual(gate.output_of('x', frames, list(frames)), 'same output: paint and damage (2 flushes)')

    def test_paint_parting_is_reported_at_its_flush(self):
        before = [('a1', 'd1'), ('a2', 'd2'), ('a3', 'd3')]
        after = [('a1', 'd1'), ('a2', 'd2'), ('b3', 'd3')]
        self.assertEqual(gate.output_of('x', before, after), 'output differs from flush 2 (of 3)')

    def test_same_paint_with_other_damage_says_so(self):
        before = [('a1', 'd1'), ('a2', 'd2')]
        after = [('a1', 'd1'), ('a2', 'e2')]
        self.assertEqual(gate.output_of('x', before, after), 'same paint; damage differs from flush 1 (of 2)')

    def test_a_harness_flush_compares_paint_alone(self):
        before = [('a1', '-'), ('a2', '-')]
        self.assertEqual(gate.output_of('x', before, list(before)), 'same output: paint (2 flushes)')

    def test_unhooked_failed_and_new_scenes_say_so(self):
        with patch.dict(gate.UNPRINTED, {'unhooked': 'not hooked: drawn some other way'}):
            self.assertEqual(gate.output_of('unhooked', [], []), 'not hooked: drawn some other way')
        self.assertIn('not hooked', gate.output_of('some-scene', [], []))
        self.assertIn('no adapter draws', gate.output_of('3d-textured', [], []))
        self.assertEqual(gate.output_of('x', 'failed', [('a', 'd')]), 'the run failed')
        self.assertEqual(gate.output_of('x', [('a', 'd')], None), 'the run failed')
        self.assertEqual(gate.output_of('x', None, [('a', 'd')]), '(new) 1 flushes')
        self.assertEqual(gate.output_of('x', [('a', 'd')], [('a', 'd'), ('b', 'e')]), 'flush count differs (1 vs 2)')


if __name__ == '__main__':
    unittest.main()
