"""The names `channel-resources` retired do not come back into the source.

The thread and reaction names bound with no channel, and the three capabilities that were not
scoped to one, are gone (design.md, "Two tests, written by task 4.1"). This is the first of the
two: a scan of the source. The second runs in the door's `cargo test` and asks the kernel, so
a name spelled to get past a scan is still caught. The scoped `urn:cap:slack:channel:{name}:read`
has `channel:` between and never matches.
"""

from __future__ import annotations

import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

RETIRED = [
    "urn:slack:thread:",
    "urn:cap:slack:read",
    "urn:cap:slack:post",
    "urn:cap:slack:react",
]

SKIPPED_DIRS = {"target", ".git"}


def retired_names_in(root: Path) -> list[str]:
    problems = []
    for path in sorted((root / "artifacts").rglob("*")):
        if not path.is_file() or SKIPPED_DIRS & set(path.relative_to(root).parts):
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        for name in RETIRED:
            if name in text:
                problems.append(f"{path.relative_to(root)} contains {name}")
    return problems


class OldNamesTest(unittest.TestCase):
    def test_no_file_under_artifacts_names_a_retired_name(self) -> None:
        problems = retired_names_in(ROOT)
        self.assertEqual([], problems, "\n".join(problems))

    def test_the_scan_finds_a_retired_name(self) -> None:
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "artifacts" / "x").mkdir(parents=True)
            (root / "artifacts" / "x" / "a.rs").write_text('"urn:cap:slack:read"', encoding="utf-8")
            (root / "artifacts" / "x" / "b.rs").write_text('"urn:cap:slack:channel:c:read"', encoding="utf-8")
            self.assertEqual(["artifacts/x/a.rs contains urn:cap:slack:read"], retired_names_in(root))


if __name__ == "__main__":
    unittest.main()
