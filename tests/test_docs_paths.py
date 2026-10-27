"""Every backticked `docs/...` path AGENTS.md or the harness guides name is a file here.

Scoped to `docs/`: AGENTS.md also names per-machine files, such as `.gonk/worker.toml`, that
are not in the tree.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

NAMING = ["AGENTS.md", ".gonk/harness/harness.ttl"]
DOCS_PATH = re.compile(r"`(docs/[^`\s]+)`")


def missing_docs_paths(root: Path) -> list[str]:
    problems = []
    for name in NAMING:
        text = (root / name).read_text(encoding="utf-8")
        for path in sorted(set(DOCS_PATH.findall(text))):
            if not (root / path).is_file():
                problems.append(f"{name} names {path}, which is missing")
    return problems


class DocsPathsTest(unittest.TestCase):
    def test_every_named_docs_path_exists(self) -> None:
        problems = missing_docs_paths(ROOT)
        self.assertEqual([], problems, "\n".join(problems))


if __name__ == "__main__":
    unittest.main()
