"""prune_caches.py: which Actions cache entries ci-caches.yml deletes."""

import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import prune_caches  # noqa: E402


class PruneTests(unittest.TestCase):
    def entry(self, id_, key, created, ref="refs/heads/main"):
        return {"id": id_, "key": key, "ref": ref, "createdAt": created}

    def test_prune(self):
        entries = [
            self.entry(1, "v0-rust-linux-Linux-x64-43996dc4-2db137b3", "2026-11-11T18:41:50Z"),
            self.entry(2, "v0-rust-linux-Linux-x64-43996dc4-fc4dc2da", "2026-11-11T20:57:10Z"),
            # A compiler update (new environment hash) supersedes too.
            self.entry(3, "v0-rust-linux-Linux-x64-11111111-fc4dc2da", "2026-11-01T10:00:00Z"),
            self.entry(4, "v0-rust-perf-gate-perf-Linux-x64-43996dc4-fc4dc2da", "2026-11-11T21:05:42Z"),
            self.entry(5, "v0-rust-windows-Windows_NT-x64-19d81eeb-ce0b94eb", "2026-11-11T21:15:23Z"),
            self.entry(6, "v0-rust-windows-Windows_NT-x64-19d81eeb-fc4dc2da", "2026-11-11T19:43:18Z"),
            self.entry(7, "apt-perf-X64-2026-11", "2026-11-11T20:37:09Z", "refs/pull/1882/merge"),
            self.entry(8, "v0-rust-dry-run-Linux-x64-43996dc4-fc4dc2da", "2026-11-10T00:00:00Z", "refs/tags/v0.1.10"),
            self.entry(9, "apt-perf-X64-2026-11", "2026-11-04T16:18:36Z"),
            self.entry(10, "gate-bin-0123abcd", "2026-11-11T21:00:00Z"),
            self.entry(11, "gate-bin-4567ef01", "2026-11-11T20:00:00Z"),
        ]
        deleted = dict(prune_caches.to_delete(entries))
        self.assertEqual(sorted(deleted), [1, 3, 6, 7, 8])
        self.assertIn("43996dc4-fc4dc2da", deleted[1])
        self.assertIn("refs/pull/1882/merge", deleted[7])

    def test_newest_perf_binaries_kept(self):
        entries = [self.entry(i, f"gate-bin-{i:040x}", f"2026-11-11T{i:02d}:00:00Z") for i in range(12)]
        entries.append(self.entry(99, "gate-bin-abc", "2026-11-11T23:00:00Z", "refs/pull/5/merge"))
        deleted = dict(prune_caches.to_delete(entries))
        self.assertEqual(sorted(deleted), [0, 1, 2, 3, 99])  # the newest 8 of main's stay

    def test_family(self):
        self.assertEqual(prune_caches.family("v0-rust-macos-Darwin-arm64-1d331f2e-fc4dc2da"),
                         "v0-rust-macos-Darwin-arm64")
        self.assertIsNone(prune_caches.family("apt-X64-2026-11-dc6eae39"))
        self.assertIsNone(prune_caches.family("luau-tools-stylua-2.6.0-luau-lsp-1.70.1-linux-x86_64"))


if __name__ == "__main__":
    unittest.main()
