"""A change whose vectors task is done has a vector for every scenario it adds or modifies.

`openspec/changes/channel-resources/tasks.md` 5.1 says: update `openspec/vectors.json` for every
changed and new scenario. Once that box is checked, every `#### Scenario:` under the change's
`## ADDED` and `## MODIFIED Requirements` has one entry in the vectors, so archiving the change
(5.3) leaves no scenario in `openspec/specs/` without one.
"""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path

from test_specs_and_vectors import OPENSPEC, REQUIREMENT, SCENARIO, gonk

DONE_VECTORS_TASK = re.compile(
    r"^- \[x\] \S+ Update `openspec/vectors\.json` for every changed and new scenario", re.MULTILINE
)


def added_or_modified(delta: Path, capability: str) -> list[str]:
    found, keep, requirement = [], False, None
    for line in delta.read_text(encoding="utf-8").splitlines():
        if line.startswith("## "):
            keep = line.strip() in ("## ADDED Requirements", "## MODIFIED Requirements")
        m = REQUIREMENT.match(line)
        if m:
            requirement = m.group(1)
            continue
        m = SCENARIO.match(line)
        if m and keep and requirement is not None:
            iri = gonk.requirement_iri(capability, requirement)
            found.append(f"{iri}:scenario:{gonk.slug(m.group(1))}")
    return found


class ChangeVectors(unittest.TestCase):
    def test_a_change_whose_vectors_task_is_done_has_a_vector_for_each_new_or_changed_scenario(self):
        vectors = json.loads((OPENSPEC / "vectors.json").read_text(encoding="utf-8"))
        ids = {s["id"] for s in vectors["scenarios"]}
        for change in sorted((OPENSPEC / "changes").iterdir()):
            tasks = change / "tasks.md"
            if change.name == "archive" or not tasks.is_file():
                continue
            if not DONE_VECTORS_TASK.search(tasks.read_text(encoding="utf-8")):
                continue
            missing = [
                s
                for delta in sorted(change.glob("specs/*/spec.md"))
                for s in added_or_modified(delta, delta.parent.name)
                if s not in ids
            ]
            with self.subTest(change=change.name):
                self.assertEqual(missing, [], f"{change.name}: scenarios with no vector")


if __name__ == "__main__":
    unittest.main()
