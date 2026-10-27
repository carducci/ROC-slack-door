"""The channel-resources proposal keeps its `## Resource map` and its `**Not needed:**` line.

AGENTS.md's resource-first pass puts the map under `## Resource map` in a change's
`proposal.md`, with the names a `roc` design leaves out on a `Not needed:` line. Scoped to
the one proposal that has its map: `channel-files` has none yet.
"""

from __future__ import annotations

import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

PROPOSAL = "openspec/changes/channel-resources/proposal.md"


def missing_map_parts(text: str) -> list[str]:
    lines = text.splitlines()
    problems = []
    if "## Resource map" not in lines:
        problems.append(f"{PROPOSAL} has no `## Resource map` heading")
    if not any(line.startswith("**Not needed:**") for line in lines):
        problems.append(f"{PROPOSAL} has no `**Not needed:**` line")
    return problems


class ResourceMapTest(unittest.TestCase):
    def test_the_channel_resources_proposal_has_its_map_and_not_needed_line(self) -> None:
        problems = missing_map_parts((ROOT / PROPOSAL).read_text(encoding="utf-8"))
        self.assertEqual([], problems, "\n".join(problems))


if __name__ == "__main__":
    unittest.main()
