"""sizes.py: symbols read from llvm-nm, names kept across builds, what
counts as moved, which binaries it builds, and that it fails on any move.

    python3 scripts/perf/test_sizes.py -v
"""
import pathlib
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import sizes  # noqa: E402

NM = """\
0000000100001000 T main
0000000100001040 t zgx::scene::Scene::walk::h0123456789abcdef
0000000100001100 t core::ptr::drop_in_place<zgx::Node>::hfedcba9876543210
0000000100001100 t alias_at_same_address
0000000100001180 T _last_text_symbol
0000000100008000 D DATA_A
0000000100008010 d data_b.llvm.12345
0000000100008030 D DATA_END
garbage line
"""


class SymbolTests(unittest.TestCase):
    def test_names_drop_rustc_hashes_and_llvm_suffixes(self):
        self.assertEqual(sizes.normalize('a::b::h0123456789abcdef'), 'a::b')
        self.assertEqual(sizes.normalize('data.llvm.42'), 'data')
        self.assertEqual(sizes.normalize('plain'), 'plain')

    def test_sizes_are_distances_to_the_next_symbol_of_the_kind(self):
        got = sizes.symbol_sizes(NM)
        self.assertEqual(got['main'], 0x40)
        self.assertEqual(got['zgx::scene::Scene::walk'], 0xc0)
        # Two names at one address (aliases): the first by name is 0 bytes
        # and left out, the other holds the size, the same in every build.
        self.assertNotIn('alias_at_same_address', got)
        self.assertEqual(got['core::ptr::drop_in_place<zgx::Node>'], 0x80)
        # The last of a kind has no known end.
        self.assertNotIn('_last_text_symbol', got)
        self.assertEqual(got['DATA_A'], 0x10)
        self.assertEqual(got['data_b'], 0x20)
        self.assertNotIn('DATA_END', got)

    def test_names_numbered_by_emission_order_are_left_out(self):
        nm = """\
0000000100001000 T main
0000000100001040 s _anon.8180f6d836fdfbc2d50caef93d99bffd.3733
0000000100001100 s GCC_except_table1192
0000000100001140 s ltmp12
0000000100001180 t zgx::walk
00000001000011c0 T _end_of_text
"""
        got = sizes.symbol_sizes(nm)
        self.assertEqual(set(got), {'main', 'zgx::walk'})

    def test_sections_are_read_by_segment_from_mach_o_and_by_name_from_elf(self):
        mach_o = """Segment __PAGEZERO: 4294967296
Segment __TEXT: 5472256
\tSection __text: 2746344
\tSection __const: 2329544
\ttotal 5461110
Segment __DATA_CONST: 81920
\tSection __const: 68072
"""
        self.assertEqual(sizes.section_sizes(mach_o, ''),
                         {'__TEXT,__text': 2746344, '__TEXT,__const': 2329544, '__DATA_CONST,__const': 68072})
        sysv = """app  :
section          size      addr
.text         1048576     65536
.rodata        262144   1114112
.data.rel.ro    4096    1376256
Total         1314816
"""
        self.assertEqual(sizes.section_sizes('', sysv),
                         {'.text': 1048576, '.rodata': 262144, '.data.rel.ro': 4096})

    def test_strings_are_compared_without_their_padding(self):
        a = sizes.strings_in(b'panic\0\0\0\0abc\0')
        b = sizes.strings_in(b'abc\0panic\0\0\0\0\0\0')
        self.assertEqual(a, b)
        self.assertNotEqual(a, sizes.strings_in(b'abc\0panicked\0'))

    def test_paths_of_different_lengths_stop_it(self):
        sizes.same_lengths('/tmp/abc', '/tmp/xyz')
        with self.assertRaises(SystemExit):
            sizes.same_lengths('/tmp/abc', '/tmp/abcd')

    def test_a_note_follows_its_section(self):
        text = '\n'.join(sizes.report('app', 1, 1, [], section_rows=[('__TEXT,__cstring', 16, 32)],
                                       notes={'__TEXT,__cstring': 'the same strings: padding'}))
        self.assertIn('section __TEXT,__cstring: 16 -> 32 (+16), the same strings: padding', text)

    def test_moved_lists_changes_additions_and_removals_largest_first(self):
        before = {'same': 10, 'grew': 10, 'gone': 5}
        after = {'same': 10, 'grew': 30, 'new': 100}
        rows = sizes.moved(before, after)
        self.assertEqual([r[0] for r in rows], ['new', 'grew', 'gone'])
        self.assertEqual(rows[1], ('grew', 10, 30))
        self.assertEqual(sizes.moved(before, dict(before)), [])

    def test_report_says_added_and_removed(self):
        text = '\n'.join(sizes.report('app', 100, 120, [('n', None, 8), ('o', 4, None)]))
        self.assertIn('stripped 100 -> 120 bytes (+20)', text)
        self.assertIn('added', text)
        self.assertIn('removed', text)


class BuildTests(unittest.TestCase):
    def test_ui_scenes_leave_out_3d_and_video_and_repeats(self):
        scenes = [('a', 'zgx-desktop', 'heavy_headless', {}), ('b', 'zgx-desktop', 'heavy_headless', {}),
                  ('c', 'zgx-3d', 'world_bench', {}), ('d', 'zgx-video', 'bench', {}),
                  ('e', 'zgx-script', 'mount_bench', {})]
        with patch.object(sizes.gate, 'SCENES', scenes), \
                patch.object(sizes.gate, 'has_example', return_value=True):
            got = sizes.ui_scenes(pathlib.Path('.'))
        self.assertEqual(got, [('zgx-desktop', 'heavy_headless'), ('zgx-script', 'mount_bench')])

    def test_zgx_run_builds_without_3d(self):
        with tempfile.TemporaryDirectory() as work:
            checkout = pathlib.Path(work)
            crate = checkout / 'crates' / 'zgx-run'
            crate.mkdir(parents=True)
            (crate / 'Cargo.toml').write_text(
                '[features]\ndefault = ["mimalloc", "scene3d", "i18n", "gltf", "terminal"]\n')
            with patch.object(sizes.gate, 'package_in', return_value='zgx-run'):
                self.assertEqual(sizes.run_features(checkout), ['mimalloc', 'i18n', 'terminal'])


class VerdictTests(unittest.TestCase):
    def run_main(self, base, head):
        """main() with builds, strip and nm replaced: `base` and `head` map
        each binary to (stripped size, symbols)."""
        measured = {}

        def fake_build(checkout, target, into):
            side = 'head' if checkout == sizes.ROOT else 'base'
            for name in (head if side == 'head' else base):
                measured[str(into / name)] = (head if side == 'head' else base)[name]
            return list(head if side == 'head' else base)

        with tempfile.TemporaryDirectory() as work, \
                patch.object(sizes, 'WORK', pathlib.Path(work)), \
                patch.object(sizes, 'llvm_nm', return_value='llvm-nm'), \
                patch.object(sizes, 'llvm_tool', return_value='llvm-size'), \
                patch.object(sizes, 'base_dir', return_value=pathlib.Path(work) / 'base'), \
                patch.object(sizes, 'same_lengths'), \
                patch.object(sizes.subprocess, 'run'), \
                patch.object(sizes, 'build', side_effect=fake_build), \
                patch.object(sizes, 'measure', side_effect=lambda binary, scratch, nm, size: measured[str(binary)]), \
                patch.object(sys, 'argv', ['sizes.py', 'main']), \
                patch('builtins.print'):
            with self.assertRaises(SystemExit) as ended:
                sizes.main()
        return ended.exception.code

    def test_nothing_moved_passes(self):
        same = {'heavy_headless': (1000, {'f': 10}, {'.text': 64}), 'zgx-run': (2000, {'g': 20}, {'.text': 64})}
        self.assertEqual(self.run_main(same, dict(same)), 0)

    def test_a_symbol_that_moved_fails_though_the_size_did_not(self):
        base = {'zgx-run': (2000, {'g': 20, 'h': 8}, {'.text': 64})}
        head = {'zgx-run': (2000, {'g': 24, 'h': 4}, {'.text': 64})}
        self.assertEqual(self.run_main(base, head), 1)

    def test_a_stripped_size_that_moved_fails(self):
        self.assertEqual(self.run_main({'zgx-run': (2000, {}, {})}, {'zgx-run': (2016, {}, {})}), 1)

    def test_a_section_that_moved_fails_though_nothing_else_did(self):
        # A page-aligned binary under a step: anonymous constants grew, no
        # named symbol and no stripped size moved.
        base = {'zgx-run': (2000, {'g': 20}, {'__TEXT,__const': 4096})}
        head = {'zgx-run': (2000, {'g': 20}, {'__TEXT,__const': 4160})}
        self.assertEqual(self.run_main(base, head), 1)


if __name__ == '__main__':
    unittest.main()
