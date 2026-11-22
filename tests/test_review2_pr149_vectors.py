"""Review 2 of PR 149 (ledger item 46, task 4.1 of `channel-files`): a door vector names its scenario.

A vector marked `"door": true` is skipped by `artifacts/slack/tests/vectors.rs` and holds only
the name of the test that runs its scenario instead. Spec `files`, "Retention removes a
screenshot": WHEN the retention chore deletes the file from David's post, THEN reading it is
refused as `no-file`, and its object is gone. The chore holds only `…:file:delete` (design D5)
and reaches the door through its kernel. The vector names `deleting_the_only_name_removes_the_object`,
a unit test of the library's `Files::delete` that issues no request, checks no capability and
reads nothing back: the door could stop serving Delete, or serve it under `file:read`, and the
test the vector names would stay green.
"""

from __future__ import annotations

import json
import re
import unittest

from test_specs_and_vectors import OPENSPEC

ROOT = OPENSPEC.parent
RETENTION = (
    "urn:openspec:spec:files:requirement:deleting-a-file-removes-its-name:"
    "scenario:retention-removes-a-screenshot"
)


def door_test(identifier: str) -> str:
    vectors = json.loads((OPENSPEC / "vectors.json").read_text(encoding="utf-8"))
    vector = next(s for s in vectors["scenarios"] if s["id"] == identifier)
    return next(step["door"] for step in vector["steps"] if "door" in step)


class DoorVectors(unittest.TestCase):
    def test_the_retention_vector_names_a_test_that_deletes_through_the_door(self):
        name = door_test(RETENTION)
        defined = re.compile(rf"\bfn {re.escape(name)}\s*\(")
        found = [
            path
            for path in sorted((ROOT / "artifacts" / "door").rglob("*.rs"))
            if defined.search(path.read_text(encoding="utf-8"))
        ]
        self.assertTrue(
            found,
            f"the vector names {name!r}, which is not a test of the door's kernel: "
            "it never deletes with only file:delete and reads the file back as no-file",
        )


if __name__ == "__main__":
    unittest.main()
