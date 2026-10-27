"""No instruction in AGENTS.md or the harness guides writes a bare `#N` (rule 6).

Ledger items and pull requests share numbers, so a `#N` is qualified: `PR #N` for a pull
request, `<ledger>#N` or `owner/repo#N` (the qualifier touches the `#`) for an item or
another repository. Rule 6 names the bare form it forbids, as "bare `#N`"; that mention is
the one exception.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

NAMING = ["AGENTS.md", ".gonk/harness/harness.ttl"]
# A `#N` or `#12` not touching a qualifier (`default#12`, `owner/repo#N`, `<ledger>#N`).
HASH_NUMBER = re.compile(r"(?<![\w/>])#(?:N|\d+)\b")
QUALIFIED = re.compile(r"(?:PR |bare `?)$")


def bare_item_numbers(root: Path) -> list[str]:
    problems = []
    for name in NAMING:
        for number, line in enumerate((root / name).read_text(encoding="utf-8").splitlines(), 1):
            for hit in HASH_NUMBER.finditer(line):
                if not QUALIFIED.search(line[: hit.start()]):
                    problems.append(f"{name}:{number} writes a bare {hit.group()}: {line.strip()}")
    return problems


class BareItemNumbersTest(unittest.TestCase):
    def test_no_instruction_writes_a_bare_item_number(self) -> None:
        problems = bare_item_numbers(ROOT)
        self.assertEqual([], problems, "\n".join(problems))


if __name__ == "__main__":
    unittest.main()
