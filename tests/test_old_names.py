"""The names `channel-resources` retired do not come back into the source.

The thread and reaction names bound with no channel, and the three capabilities that were not
scoped to one, are gone (design.md, "Two tests, written by task 4.1"). This is the first of the
two: a scan of the source. The second runs in the door's `cargo test` and asks the kernel, so
a name spelled to get past a scan is still caught. The scoped `urn:cap:slack:channel:{name}:read`
has `channel:` between and never matches.

The scan reads bytes, never text: a file that does not decode as UTF-8 is searched all the same,
for each name as UTF-8 and as UTF-16 in both byte orders.
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

RETIRED = [
    "urn:slack:thread:",
    "urn:cap:slack:read",
    "urn:cap:slack:post",
    "urn:cap:slack:react",
]

# What cargo writes, by exact path: `target/` at the workspace root. A directory named `target`
# anywhere else is source.
BUILD_OUTPUT = ("target",)


def encodings(name: str) -> list[bytes]:
    return [name.encode("utf-8"), name.encode("utf-16-le"), name.encode("utf-16-be")]


def retired_names_in(root: Path) -> list[str]:
    problems = []
    skipped = [root / part for part in BUILD_OUTPUT]
    for path in sorted((root / "artifacts").rglob("*")):
        if not path.is_file() or any(path.is_relative_to(skip) for skip in skipped):
            continue
        data = path.read_bytes()
        for name in RETIRED:
            if any(found in data for found in encodings(name)):
                problems.append(f"{path.relative_to(root).as_posix()} contains {name}")
    return problems


class OldNamesTest(unittest.TestCase):
    def test_no_file_under_artifacts_names_a_retired_name(self) -> None:
        problems = retired_names_in(ROOT)
        self.assertEqual([], problems, "\n".join(problems))

    def test_the_scan_finds_a_retired_name(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "artifacts" / "x").mkdir(parents=True)
            (root / "artifacts" / "x" / "a.rs").write_text('"urn:cap:slack:read"', encoding="utf-8")
            (root / "artifacts" / "x" / "b.rs").write_text('"urn:cap:slack:channel:c:read"', encoding="utf-8")
            self.assertEqual(["artifacts/x/a.rs contains urn:cap:slack:read"], retired_names_in(root))

    def test_the_scan_reads_files_that_do_not_decode_and_directories_named_target(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            src = root / "artifacts" / "slack" / "src"
            (src / "target").mkdir(parents=True)
            (root / "target").mkdir()
            (src / "a.rs").write_bytes(b"caf\xe9 urn:cap:slack:read")
            (src / "b.rs").write_bytes("urn:cap:slack:post".encode("utf-16"))
            (src / "e.rs").write_bytes("urn:cap:slack:react".encode("utf-16-be"))
            (src / "target" / "c.rs").write_text("urn:slack:thread:k", encoding="utf-8")
            (src / "d.rs").write_text("urn:slack:thread:k", encoding="utf-8")
            (root / "target" / "built.rs").write_text("urn:slack:thread:k", encoding="utf-8")
            self.assertEqual(
                [
                    "artifacts/slack/src/a.rs contains urn:cap:slack:read",
                    "artifacts/slack/src/b.rs contains urn:cap:slack:post",
                    "artifacts/slack/src/d.rs contains urn:slack:thread:",
                    "artifacts/slack/src/e.rs contains urn:cap:slack:react",
                    "artifacts/slack/src/target/c.rs contains urn:slack:thread:",
                ],
                retired_names_in(root),
            )

    def test_cargos_own_output_under_the_artifacts_is_skipped_by_exact_path(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "artifacts" / "x" / "target").mkdir(parents=True)
            (root / "artifacts" / "x" / "target" / "c.rs").write_text("urn:cap:slack:read", encoding="utf-8")
            self.assertEqual(["artifacts/x/target/c.rs contains urn:cap:slack:read"], retired_names_in(root))


if __name__ == "__main__":
    unittest.main()
