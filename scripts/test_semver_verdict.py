"""semver_verdict.py's tests: semver-checks output over a crate written to a temporary directory."""
import contextlib
import importlib.util
import io
from pathlib import Path
import tempfile
import textwrap
import unittest

spec = importlib.util.spec_from_file_location("semver_verdict", Path(__file__).with_name("semver_verdict.py"))
semver_verdict = importlib.util.module_from_spec(spec)
spec.loader.exec_module(semver_verdict)

SOURCE = """\
use std::fmt;

/// Derived on one line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Derived {
    pub a: u32,
    pub added: bool,
}

/// Derived over several lines, among other attributes.
#[derive(
    Clone,
    Default,
)]
#[non_exhaustive]
pub struct Spread {
    pub added: bool,
}

/// Implemented by hand, in another file.
pub struct Implemented {
    pub added: bool,
}

/// Generic, implemented by hand.
pub struct Generic<T> {
    pub value: T,
    pub added: bool,
}

/// None: an app's literal of it has no way to keep compiling.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub id: u64,
    pub added: bool,
}

/// Its derive belongs to the struct above it, not this one.
pub struct After {
    pub added: bool,
}
"""

IMPLS = """\
impl Default for crate::Implemented {
    fn default() -> Self { Self { added: false } }
}
impl<T: Default> Default for Generic<T> {
    fn default() -> Self { Self { value: T::default(), added: false } }
}
"""


AUTO_TRAIT = """\
--- failure auto_trait_impl_removed: auto trait no longer implemented ---

Description:
A public type has stopped implementing one or more auto traits.

Failed in:
  type TextCache is no longer Send, in /somewhere/crates/zgx-gpu/src/prepared.rs:877
  type TextCache is no longer Sync, in /somewhere/crates/zgx-gpu/src/prepared.rs:877

     Summary semver requires new major version: 1 major and 0 minor checks failed
"""


def log(path, *flags, lint=semver_verdict.LINT):
    """semver-checks' output for these (struct, field) flags in path, each listed twice as it lists them."""
    fields = "\n".join(f"  field {struct}.{field} in {path}:5" for struct, field in flags)
    return textwrap.dedent(f"""\
        --- failure {lint}: struct exhaustively constructible through public API adds field ---

        Description:
        A pub struct that could be exhaustively constructed with a literal ...

        Failed in:
        {{fields}}
        {{fields}}

             Summary semver requires new major version: 1 major and 0 minor checks failed
        """).format(fields=fields)


class SemverVerdict(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        (self.root / "Cargo.toml").write_text('[package]\nname = "fixture"\n')
        (self.root / "src").mkdir()
        self.lib = self.root / "src" / "lib.rs"
        self.lib.write_text(SOURCE)
        (self.root / "src" / "impls.rs").write_text(IMPLS)

    def run_check(self, text, allowed="", release="0.1.8", name="fixture.log"):
        path = self.root / name
        path.write_text(text)
        allow_list = self.root / "semver_allowed.txt"
        allow_list.write_text("# release | crate | lint | item | reason\n" + allowed)
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = semver_verdict.main(["--release", release, str(path)], allow_list)
        return code, out.getvalue(), err.getvalue()

    def test_structs_with_default_pass(self):
        flags = [("Derived", "added"), ("Spread", "added"), ("Implemented", "added"), ("Generic", "added")]
        code, out, err = self.run_check(log(self.lib, *flags))
        self.assertEqual((code, err), (0, ""))
        self.assertEqual(out.count("has Default"), 4)

    def test_a_struct_without_default_fails(self):
        code, _, err = self.run_check(log(self.lib, ("Derived", "added"), ("Snapshot", "added")))
        self.assertEqual(code, 1)
        self.assertIn("Snapshot.added: Snapshot has no Default", err)
        self.assertNotIn("Derived", err)

    def test_the_derive_of_the_struct_above_is_not_taken(self):
        code, _, err = self.run_check(log(self.lib, ("After", "added")))
        self.assertEqual(code, 1)
        self.assertIn("After has no Default", err)

    def test_a_struct_not_found_fails_to_be_checked_by_hand(self):
        code, _, err = self.run_check(log(self.lib, ("Macro", "added")))
        self.assertEqual(code, 1)
        self.assertIn("no `pub struct Macro` found", err)

    def test_each_field_is_named_once(self):
        _, _, err = self.run_check(log(self.lib, ("Snapshot", "added")))
        self.assertEqual(err.count("Snapshot.added"), 1)

    def test_another_lint_without_a_ruling_fails(self):
        code, _, err = self.run_check(AUTO_TRAIT)
        self.assertEqual(code, 1)
        self.assertIn("fixture: auto_trait_impl_removed: type TextCache is no longer Send", err)
        self.assertIn("no ruling in semver_allowed.txt for 0.1.8", err)

    def test_a_ruling_for_the_release_accepts_it_with_its_reason(self):
        ruling = "0.1.8 | fixture | auto_trait_impl_removed | type TextCache is no longer | held on one thread only\n"
        code, out, err = self.run_check(AUTO_TRAIT, ruling)
        self.assertEqual((code, err), (0, ""))
        self.assertEqual(out.count("allowed for 0.1.8: held on one thread only"), 2)

    def test_a_ruling_counts_only_for_its_release_and_crate(self):
        for ruling in ("0.1.7 | fixture | auto_trait_impl_removed | TextCache | an older release\n",
                       "0.1.8 | zgx-gpu | auto_trait_impl_removed | TextCache | another crate\n",
                       "0.1.8 | fixture | auto_trait_impl_added | TextCache | another lint\n"):
            code, _, _ = self.run_check(AUTO_TRAIT, ruling)
            self.assertEqual(code, 1, ruling)

    def test_a_ruling_can_accept_a_field_on_a_struct_without_default(self):
        ruling = "0.1.8 | fixture | constructible_struct_adds_field | field Snapshot.added | engine-made, never built by apps\n"
        code, out, err = self.run_check(log(self.lib, ("Snapshot", "added")), ruling)
        self.assertEqual((code, err), (0, ""))
        self.assertIn("allowed for 0.1.8: engine-made", out)

    def test_a_log_of_several_crates_names_each_failure_s_crate(self):
        text = "== zgx\n     Summary no semver update required\n== zgx-gpu\n" + AUTO_TRAIT
        _, _, err = self.run_check(text, name="semver.log")
        self.assertIn("zgx-gpu: auto_trait_impl_removed", err)
        self.assertNotIn("zgx: auto_trait", err)

    def test_warnings_are_not_failures(self):
        code, _, _ = self.run_check(AUTO_TRAIT.replace("--- failure", "--- warning"))
        self.assertEqual(code, 0)

    def test_a_malformed_ruling_stops_the_check(self):
        with self.assertRaises(SystemExit) as stopped:
            self.run_check(AUTO_TRAIT, "0.1.8 | fixture | auto_trait_impl_removed | no reason\n")
        self.assertIn("want `release | crate | lint | item | reason`", str(stopped.exception))

    def test_a_clean_run_passes(self):
        code, _, _ = self.run_check("     Summary no semver update required\n")
        self.assertEqual(code, 0)


if __name__ == "__main__":
    unittest.main()
