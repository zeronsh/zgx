"""A release's semver verdict from cargo semver-checks' output (the release check's step 5):

    cargo semver-checks -p zgx --baseline-version 0.1.7 > zgx.log 2>&1
    python3 scripts/semver_verdict.py --release 0.1.8 zgx.log [more.log ...]

Every failure semver-checks reports (each "--- failure <lint>" and the items
it names) fails the release unless one of two things accepts it:

- `constructible_struct_adds_field` on a struct with Default, derived or
  implemented: an app's literal of it can end with `..Default::default()`,
  the migrate note the changelog gives. Without Default the app has no way
  to keep compiling (zgx-script's `driver::Line` and `Window` at 0.1.7's
  first freeze; #1272 put back their 0.1.6 fields). A struct this can't
  find (one a macro writes) fails, to check by hand.
- An entry in scripts/semver_allowed.txt for this release, crate and lint
  whose item text is in the item's line: a ruling made for that release,
  with its reason, reviewed like any other change.

So a break no one ruled on fails the check rather than waiting for someone
to read the log (0.1.8's pre-flight found zgx-gpu's `TextCache` no longer
`Send` by reading it). A log is one crate's, named `<crate>.log`, or holds
several after `== <crate>` lines. Exits 1 naming each failure not accepted.
"""
import re
import sys
from pathlib import Path

LINT = "constructible_struct_adds_field"
FIELD = re.compile(r"^\s*field (\w+)\.(\w+) in (.+?):(\d+)\s*$")
ALLOWED = Path(__file__).with_name("semver_allowed.txt")


def failures(text, crate):
    """(crate, lint, item line) for each item each failure names, once each."""
    found, lint, items = [], None, False
    for line in text.splitlines():
        header = re.match(r"^== (\S+)", line)
        if header:
            crate, lint, items = header.group(1), None, False
            continue
        failure = re.match(r"^--- failure (\w+):", line)
        if failure:
            lint, items = failure.group(1), False
            continue
        if line.startswith("--- "):
            lint, items = None, False
            continue
        if lint and line.strip() == "Failed in:":
            items = True
            continue
        if items:
            # Items are indented two spaces; the summary after them, more.
            if not re.match(r"^  \S", line):
                items = False
                continue
            entry = (crate, lint, line.strip())
            if entry not in found:
                found.append(entry)
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


def allowed(path):
    """The allow-list's entries: (release, crate, lint, item, reason)."""
    entries = []
    for number, line in enumerate(Path(path).read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        fields = [field.strip() for field in line.split("|")]
        if len(fields) != 5 or not all(fields):
            raise SystemExit(f"{Path(path).name}:{number}: want `release | crate | lint | item | reason`, got: {line}")
        entries.append(tuple(fields))
    return entries


def main(argv, allow_list=ALLOWED):
    release, logs = None, []
    arguments = iter(argv)
    for argument in arguments:
        if argument == "--release":
            release = next(arguments, None)
        else:
            logs.append(argument)
    if not release or not logs:
        sys.exit(__doc__)
    entries = [entry for entry in allowed(allow_list) if entry[0] == release]
    problems = []
    for log in logs:
        text = Path(log).read_text(encoding="utf-8", errors="replace")
        for crate, lint, item in failures(text, Path(log).stem):
            field = FIELD.match(item) if lint == LINT else None
            if field:
                struct, name, path = field.group(1, 2, 3)
                default = has_default(struct, path)
                if default:
                    print(f"{crate}: {struct}.{name}: {struct} has Default (a migrate note: `..Default::default()`)")
                    continue
                if default is None:
                    problems.append(f"{crate}: {struct}.{name}: no `pub struct {struct}` found in {path}: check it by hand")
                    continue
            ruling = next((entry for entry in entries
                           if entry[1] == crate and entry[2] == lint and entry[3] in item), None)
            if ruling:
                print(f"{crate}: {lint}: {item}: allowed for {release}: {ruling[4]}")
            elif field:
                problems.append(f"{crate}: {struct}.{name}: {struct} has no Default, so a 0.x patch can't add it "
                                f"(an app's literal of {struct} stops compiling with no way out): "
                                "keep the field off the struct (a side accessor) or wait for a minor release")
            else:
                problems.append(f"{crate}: {lint}: {item}: no ruling in {Path(allow_list).name} for {release}: "
                                "fix it, or add `release | crate | lint | item | reason` once the release rules it acceptable")
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
