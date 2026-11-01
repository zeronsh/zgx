"""semver_defaults.py's tests: semver-checks output over a crate written to a temporary directory."""
import contextlib
import importlib.util
import io
from pathlib import Path
import tempfile
import textwrap
import unittest

spec = importlib.util.spec_from_file_location("semver_defaults", Path(__file__).with_name("semver_defaults.py"))
semver_defaults = importlib.util.module_from_spec(spec)
spec.loader.exec_module(semver_defaults)

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


def log(path, *flags, lint=semver_defaults.LINT):
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


class NewFieldsNeedDefault(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        (self.root / "Cargo.toml").write_text('[package]\nname = "fixture"\n')
        (self.root / "src").mkdir()
        self.lib = self.root / "src" / "lib.rs"
        self.lib.write_text(SOURCE)
        (self.root / "src" / "impls.rs").write_text(IMPLS)

    def run_check(self, text):
        path = self.root / "semver.log"
        path.write_text(text)
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = semver_defaults.main([str(path)])
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

    def test_other_lints_are_left_to_the_rulings(self):
        code, out, err = self.run_check(log(self.lib, ("Snapshot", "added"), lint="method_parameter_count_changed"))
        self.assertEqual((code, out, err), (0, "", ""))

    def test_a_clean_run_passes(self):
        code, _, _ = self.run_check("     Summary no semver update required\n")
        self.assertEqual(code, 0)


if __name__ == "__main__":
    unittest.main()
