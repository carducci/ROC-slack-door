"""The commands and placeholders AGENTS.md prescribes say what they claim (rules 4 and 6).

Agents read AGENTS.md raw, so a command it tells them to copy has to be copyable and true:
a code span sits on one source line, a merge subject it prescribes carries the merge
intention `m` that rule 4 lists, and one step never uses one placeholder for two values
(`N` for both a pull request and a ledger item, or two names for one branch).
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
AGENTS = ROOT / "AGENTS.md"

MERGE_COMMAND = re.compile(r'git merge\b[^`]*?-m "([^"`]*)"')
INTENTION_BULLET = re.compile(r"- Intention, exactly one per commit\.(.*?)(?=\n  - |\n\n)", re.DOTALL)
PLACEHOLDER = re.compile(r"<([a-z][a-z ]*)>")
COMMAND = re.compile(r"`(?:git|gh|python3|just) ")
ITEM_START = re.compile(r"^\s*(?:\d+\.|-)\s")


def text() -> str:
    return AGENTS.read_text(encoding="utf-8")


def lines_outside_fences() -> list[tuple[int, str]]:
    kept, fenced = [], False
    for number, line in enumerate(text().splitlines(), 1):
        if line.lstrip().startswith("```"):
            fenced = not fenced
            continue
        if not fenced:
            kept.append((number, line))
    return kept


def items() -> list[tuple[int, str]]:
    """AGENTS.md as list items and paragraphs: a block starts at a list marker or after a blank."""
    blocks: list[tuple[int, list[str]]] = []
    for number, line in lines_outside_fences():
        if not line.strip():
            blocks.append((number + 1, []))
        elif ITEM_START.match(line) or not blocks:
            blocks.append((number, [line]))
        else:
            blocks[-1][1].append(line)
    return [(start, "\n".join(body)) for start, body in blocks if body]


class PrescribedCommandsTest(unittest.TestCase):
    def test_rule_4_lists_the_merge_intention(self) -> None:
        bullet = INTENTION_BULLET.search(text())
        self.assertIsNotNone(bullet, "rule 4's intention bullet is missing")
        self.assertIn("`m`", bullet.group(1), "rule 4 lists no intention for a merge commit")

    def test_a_prescribed_merge_subject_uses_the_merge_intention(self) -> None:
        subjects = MERGE_COMMAND.findall(text())
        self.assertTrue(subjects, "AGENTS.md prescribes no `git merge -m` command")
        for subject in subjects:
            risk, intention = subject.split(" ", 2)[:2]
            self.assertIn(risk, [".", "^", "!", "@", "<risk>"], subject)
            self.assertEqual("m", intention, f"a prescribed merge subject is not a merge: {subject!r}")

    def test_every_code_span_sits_on_one_source_line(self) -> None:
        split = [f"AGENTS.md:{n} opens a code span it does not close: {line.strip()}"
                 for n, line in lines_outside_fences() if line.count("`") % 2]
        self.assertEqual([], split, "\n".join(split))

    def test_one_placeholder_per_value_in_a_step(self) -> None:
        problems = []
        for start, body in items():
            if not COMMAND.search(body):
                continue  # rule 6 defines both forms; a command's placeholders are values to fill in
            if "PR #N" in body and "ledger item N" in body:
                problems.append(f"AGENTS.md:{start} uses N for a pull request and for a ledger item")
            names = sorted(set(PLACEHOLDER.findall(body)))
            for short in names:
                for long in names:
                    if long != short and long.endswith(" " + short):
                        problems.append(f"AGENTS.md:{start} names one value <{short}> and <{long}>")
        self.assertEqual([], problems, "\n".join(problems))


if __name__ == "__main__":
    unittest.main()
