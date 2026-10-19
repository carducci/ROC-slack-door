#!/usr/bin/env python3
"""Every test in this repository, the way CI runs them: `python3 scripts/test.py`.

The check that the specs and the scenario vectors agree (`tests/`, standard-library Python),
then one `cargo test` over the workspace, so a failure in either fails this. Cargo does the
building, so this script stays the same whatever the workspace grows. A workspace with no
members yet gives cargo nothing to run, and is skipped.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def run(argv: list[str]) -> int:
    print("-- " + " ".join(argv), flush=True)
    return subprocess.run(argv, cwd=ROOT).returncode


def has_members(manifest: str) -> bool:
    found = re.search(r"^members\s*=\s*\[(.*?)\]", manifest, re.MULTILINE | re.DOTALL)
    return bool(found and found.group(1).strip())


def main() -> int:
    code = run([sys.executable, "-m", "unittest", "discover", "-s", "tests"])
    if not has_members((ROOT / "Cargo.toml").read_text(encoding="utf-8")):
        print("-- cargo test skipped: the workspace has no members yet", flush=True)
        return code
    return run(["cargo", "test", "--workspace"]) or code


if __name__ == "__main__":
    sys.exit(main())
