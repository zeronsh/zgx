#!/usr/bin/env python3
"""The mail example with its soak in, for `soak.py`: a copy, as the example
itself stays what a gate counts (every module of a folder compiles ahead, and
the bench runs this one).

    python3 scripts/ios/mail_soak.py target/ios-soak/mail
    python3 scripts/ios/sim.py run -p zgx-run --example ios_app --release \\
        --name soak_mail --resource target/ios-soak/mail --install-only
    python3 scripts/ios/soak.py dev.zgx.ios.soak-mail --minutes 60
"""

import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent / "mail_soak"


def main():
    out = Path(sys.argv[1])
    shutil.rmtree(out, ignore_errors=True)
    shutil.copytree(ROOT / "crates/zgx-script/examples/mail", out, symlinks=True)
    shutil.copy2(HERE / "soak.luau", out / "soak.luau")
    for name, old, new in json.loads((HERE / "patches.json").read_text()):
        path = out / name
        text = path.read_text()
        if text.count(old) != 1:
            sys.exit(f"{name}: the soak's patch no longer applies")
        path.write_text(text.replace(old, new))
    print(out)


if __name__ == "__main__":
    main()
