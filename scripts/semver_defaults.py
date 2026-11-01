"""New public fields only on structs with Default, from cargo semver-checks' output:

    cargo semver-checks -p zgx --baseline-version 0.1.6 > semver.log 2>&1
    python3 scripts/semver_defaults.py semver.log [more.log ...]

`constructible_struct_adds_field` flags a pub field added to a struct an app
can build with a literal. A patch release takes that only when the struct
has Default (derived or implemented): the app's literal can end with
`..Default::default()`, the migrate note the changelog gives. Without it the
app has no way to keep compiling, as with zgx-script's `driver::Line` and
`Window` at 0.1.7's first freeze (#1272 put back their 0.1.6 fields).

Exits 1 naming each flagged struct with no Default, or one this can't find
(a struct a macro writes, say): check it by hand. Other lints are left to
the release's semver rulings (docs/releasing.md).
"""
import re
import sys
from pathlib import Path

LINT = "constructible_struct_adds_field"
FIELD = re.compile(r"^\s*field (\w+)\.(\w+) in (.+?):(\d+)\s*$")


def flagged(text):
    """(struct, field, path) for each field the lint flags, once each."""
    found, inside = [], False
    for line in text.splitlines():
        if line.startswith("--- "):
            inside = LINT in line
            continue
        match = FIELD.match(line) if inside else None
        if match and match.group(1, 2, 3) not in found:
            found.append(match.group(1, 2, 3))
    return found


def crate_root(path):
    """The directory holding the crate's Cargo.toml, at or above path."""
    for parent in Path(path).resolve().parents:
        if (parent / "Cargo.toml").is_file():
            return parent
    return Path(path).resolve().parent


def attributes_above(lines, index):
    """The attribute and doc lines written above line index, joined."""
    above = []
    for line in reversed(lines[:index]):
        stripped = line.strip()
        # A blank line, or the end of the item before (a brace or a
        # semicolon), ends them; a derive may run over several lines.
        if not stripped or (stripped.endswith(("}", ";", "{")) and not stripped.startswith(("#", "//"))):
            break
        above.append(stripped)
    return " ".join(reversed(above))


def has_default(struct, path):
    """True or False when the struct is found in path; None when it isn't."""
    try:
        lines = Path(path).read_text(encoding="utf-8").splitlines()
    except OSError:
        return None
    declared = re.compile(rf"^\s*pub(\([^)]*\))?\s+struct\s+{struct}\b")
    index = next((i for i, line in enumerate(lines) if declared.match(line)), None)
    if index is None:
        return None
    if re.search(r"derive\s*\([^)]*\bDefault\b", attributes_above(lines, index)):
        return True
    implemented = re.compile(rf"impl\s*(<[^>]*>)?\s*((::)?(std|core)::default::)?Default\s+for\s+(\w+::)*{struct}\b")
    root = crate_root(path)
    return any(implemented.search(source.read_text(encoding="utf-8", errors="replace"))
               for source in (root / "src").rglob("*.rs")) if (root / "src").is_dir() else False


def main(paths):
    problems = []
    for log in paths:
        for struct, field, path in flagged(Path(log).read_text(encoding="utf-8", errors="replace")):
            default = has_default(struct, path)
            if default:
                print(f"{struct}.{field}: {struct} has Default (a migrate note: `..Default::default()`)")
            elif default is None:
                problems.append(f"{struct}.{field}: no `pub struct {struct}` found in {path}: check it by hand")
            else:
                problems.append(f"{struct}.{field}: {struct} has no Default, so a 0.x patch can't add it "
                                f"(an app's literal of {struct} stops compiling with no way out): "
                                "keep the field off the struct (a side accessor) or wait for a minor release")
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    sys.exit(main(sys.argv[1:]))
