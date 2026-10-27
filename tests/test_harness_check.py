"""`tools/harness.py check`: every rule AGENTS.md states is named by a guide in harness.ttl,
and a guide and the AGENTS.md section it cites agree on the words they quote."""

from __future__ import annotations

import importlib.util
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

spec = importlib.util.spec_from_file_location("harness", ROOT / "tools" / "harness.py")
harness = importlib.util.module_from_spec(spec)
sys.modules["harness"] = harness
spec.loader.exec_module(harness)

GUIDE = "urn:iki:harness:starter:guide:ask-dax-continue-draft"


class HarnessCheckTest(unittest.TestCase):
    def test_check_finds_nothing_wrong(self) -> None:
        problems = harness.check(ROOT)
        self.assertEqual([], problems, "\n".join(problems))


class GuideTextDriftTest(unittest.TestCase):
    """Change one copy of a rule, the guide's or AGENTS.md's, and the check fails."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name) / "repo"
        shutil.copytree(ROOT, self.repo, ignore=shutil.ignore_patterns(".git", "target"))
        self.assertEqual([], harness.check(self.repo), "the copy agrees before it is changed")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def change(self, name: str, old: str, new: str) -> None:
        path = self.repo / name
        text = path.read_text(encoding="utf-8")
        self.assertIn(old, text)
        path.write_text(text.replace(old, new), encoding="utf-8")

    def drift(self) -> list[str]:
        return [p for p in harness.check(self.repo) if p.startswith(GUIDE)]

    def test_a_guide_that_quotes_words_its_section_lacks_fails(self) -> None:
        # The reproduction of ikigai-slack#169: harness.ttl's copy of the rule drifts.
        self.change(
            ".gonk/harness/harness.ttl",
            "begin the new draft's description with `Supersedes PR #N`.",
            "begin the new draft's description with `Replaces pull request N`.",
        )
        problems = self.drift()
        self.assertEqual(1, len(problems), "\n".join(harness.check(self.repo)))
        self.assertIn("`Replaces pull request N`", problems[0])

    def test_a_section_that_loses_words_its_guide_quotes_fails(self) -> None:
        # AGENTS.md's copy drifts the other way.
        self.change(
            "AGENTS.md",
            "description with `Supersedes PR #N`.",
            "description with `Replaces PR #N`.",
        )
        problems = self.drift()
        self.assertEqual(1, len(problems), "\n".join(harness.check(self.repo)))
        self.assertIn("`Supersedes PR #N`", problems[0])

    def test_a_guide_stated_in_a_file_that_is_not_here_fails(self) -> None:
        self.change(
            ".gonk/harness/harness.ttl",
            'harness:statedIn "AGENTS.md" ; harness:section "Ask Dax, don\'t guess" ;\n'
            "    harness:direction harness:feedforward ; harness:execution harness:inferential ;\n"
            "    harness:stage harness:runtime ; harness:category harness:authoritySafety .\n"
            "\nguide:ask-dax-draft-pr",
            'harness:statedIn "NOWHERE.md" ; harness:section "Ask Dax, don\'t guess" ;\n'
            "    harness:direction harness:feedforward ; harness:execution harness:inferential ;\n"
            "    harness:stage harness:runtime ; harness:category harness:authoritySafety .\n"
            "\nguide:ask-dax-draft-pr",
        )
        problems = [
            p
            for p in harness.check(self.repo)
            if p.startswith("urn:iki:harness:starter:guide:ask-dax-dont-guess")
        ]
        self.assertTrue(any("NOWHERE.md" in p for p in problems), problems)


if __name__ == "__main__":
    unittest.main()
