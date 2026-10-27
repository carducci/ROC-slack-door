"""`tools/harness.py check`: every rule AGENTS.md states is named by a guide in harness.ttl,
and a guide and the AGENTS.md section it cites agree on the words they quote."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import re
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
        self.assertEqual(
            [
                "urn:iki:harness:starter:guide:ask-dax-dont-guess: statedIn 'NOWHERE.md', "
                "which is not in this repository"
            ],
            problems,
        )


GUIDES = "urn:iki:harness:starter:guide:"


class GuideTextTest(unittest.TestCase):
    """Each guide quotes its section, and a quote is held to the section as a reader sees it,
    in the file the guide's statedIn names, inside this repository."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.outside = Path(self.tmp.name)
        self.repo = self.outside / "repo"
        shutil.copytree(ROOT, self.repo, ignore=shutil.ignore_patterns(".git", "target"))
        self.assertEqual([], harness.check(self.repo), "the copy agrees before it is changed")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def edit(self, name: str, pattern: str, new: str) -> None:
        path = self.repo / name
        text, n = re.subn(pattern, lambda _: new, path.read_text(encoding="utf-8"), flags=re.M)
        self.assertEqual(1, n, f"{pattern!r} in {name}")
        path.write_text(text, encoding="utf-8")

    def guide(self, guide: str, comment: str | None = None, stated_in: str | None = None) -> None:
        """Give one guide in the copy's harness.ttl a new rdfs:comment or statedIn."""
        path = self.repo / ".gonk/harness/harness.ttl"
        text = path.read_text(encoding="utf-8")
        start = text.index(f"guide:{guide} a harness:Guide ;\n")
        end = text.index(" .\n", start)
        block = text[start:end]
        if comment is not None:
            block, n = re.subn(r'rdfs:comment "[^"\n]*"', f'rdfs:comment "{comment}"', block)
            self.assertEqual(1, n)
        if stated_in is not None:
            self.assertIn('harness:statedIn "AGENTS.md"', block)
            block = block.replace('harness:statedIn "AGENTS.md"', f'harness:statedIn "{stated_in}"')
        path.write_text(text[:start] + block + text[end:], encoding="utf-8")

    def restore(self) -> None:
        shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")

    @staticmethod
    def comments() -> dict[str, str]:
        text = (ROOT / ".gonk/harness/harness.ttl").read_text(encoding="utf-8")
        pattern = r'^guide:(\S+) a harness:Guide ;\n.*\n\s+rdfs:comment "([^"\n]*)"'
        return dict(re.findall(pattern, text, re.M))

    def problems(self, guide: str) -> list[str]:
        return [p for p in harness.check(self.repo) if p.startswith(GUIDES + guide + ":")]

    # A guide that quotes nothing of its section is not compared with it, so it is refused.

    def test_a_guide_whose_text_drifts_from_its_section_fails(self) -> None:
        # p2 of ikigai-slack#186: the guide now says the opposite of its section.
        self.guide("ask-dax-dont-guess", "pick the likeliest option and carry on without asking.")
        self.assertEqual(
            [
                f"{GUIDES}ask-dax-dont-guess: quotes nothing of AGENTS.md "
                "`## Ask Dax, don't guess`; quote, in backticks, a phrase the section says"
            ],
            self.problems("ask-dax-dont-guess"),
        )

    def test_a_guide_that_loses_its_only_quote_fails(self) -> None:
        self.guide("gates-before-push", "Run the checks locally before pushing (just gates).")
        self.assertEqual(
            [
                f"{GUIDES}gates-before-push: quotes nothing of AGENTS.md "
                "`## 4. Every change through a pull request with green CI`; "
                "quote, in backticks, a phrase the section says"
            ],
            self.problems("gates-before-push"),
        )

    def test_a_section_that_drops_a_quoted_rule_fails(self) -> None:
        # p2b of ikigai-slack#186: AGENTS.md's copy of a rule says the opposite.
        self.edit(
            "AGENTS.md",
            r"Never pipe a command whose result you are trusting\.",
            "Piping a command whose result you trust is fine.",
        )
        self.assertEqual(1, len(self.problems("last-command-exit-status")))

    def test_every_guide_quotes_its_section(self) -> None:
        unquoted = [g for g, c in self.comments().items() if not re.search(r"`[^`\n]+`", c)]
        self.assertEqual([], unquoted)

    # A quote must be in the section the guide cites, not merely somewhere in the file.

    def test_a_quote_from_another_section_fails(self) -> None:
        # `just gates` is in AGENTS.md, in section 4: m1/M3 and M5 of ikigai-slack#190.
        self.guide("no-bare-numbers", "Write `PR #N` for a pull request, and run `just gates`.")
        self.assertEqual(
            [
                f"{GUIDES}no-bare-numbers: quotes `just gates`, which AGENTS.md "
                "`` ## 6. Never a bare `#N` `` does not"
            ],
            self.problems("no-bare-numbers"),
        )

    # The section as a reader sees it.

    def test_a_code_span_wrapped_across_lines_still_matches(self) -> None:
        # P2 of ikigai-slack#188: CommonMark reads the line ending as a space.
        self.edit("AGENTS.md", r"`Supersedes PR #N`\.$", "`Supersedes PR\n   #N`.")
        self.assertEqual([], harness.check(self.repo))

    def test_the_headings_words_are_in_its_section(self) -> None:
        # P3 of ikigai-slack#188.
        self.guide("no-bare-numbers", "`Never a bare #N`: write `PR #N` for a pull request.")
        self.assertEqual([], harness.check(self.repo))

    def test_a_fenced_line_is_not_a_heading(self) -> None:
        # p8 of ikigai-slack#188.
        self.edit(
            "AGENTS.md",
            r"^(?=1\. Open your pull request as a draft)",
            "```sh\n## not a heading\n```\n\n",
        )
        self.assertEqual([], harness.check(self.repo))

    def test_emphasis_and_links_read_as_their_words(self) -> None:
        self.edit(
            "AGENTS.md",
            r"^- \*\*No claim, no write\.\*\*",
            "- **No claim, no write.** Read [the loop](#the-loop).",
        )
        self.guide("claim-before-write", "`No claim, no write. Read the loop.`")
        self.assertEqual([], harness.check(self.repo))

    # statedIn is a file inside this repository, compared by what it resolves to.

    def test_a_stated_in_path_outside_the_repository_fails(self) -> None:
        # P1 and P1b of ikigai-slack#187.
        (self.outside / "x").mkdir()
        shutil.copy(self.repo / "AGENTS.md", self.outside / "x" / "RULES.md")
        (self.repo / "out").symlink_to(self.outside / "x" / "RULES.md")
        for name in (str(self.outside / "x" / "RULES.md"), "../x/RULES.md", "out"):
            with self.subTest(name=name):
                self.restore()
                self.guide("ask-dax-continue-draft", stated_in=name)
                self.assertEqual(
                    [
                        f"{GUIDES}ask-dax-continue-draft: statedIn {name!r}, "
                        "which is outside this repository"
                    ],
                    self.problems("ask-dax-continue-draft"),
                )

    def test_another_spelling_of_agents_md_is_agents_md(self) -> None:
        # P8 of ikigai-slack#187: the guide still cites `## 7. …` of AGENTS.md.
        self.guide("read-accessibility-first", stated_in="./AGENTS.md")
        self.assertEqual([], harness.check(self.repo))

    def test_a_stated_in_file_that_is_not_utf8_is_a_sentence(self) -> None:
        # p6 and P6 of ikigai-slack#189.
        for body in (b"\xff\xfe\n", b"caf\xe9\n"):
            with self.subTest(body=body):
                self.restore()
                (self.repo / "BIN.md").write_bytes(b"## Ask Dax, don't guess\n" + body)
                self.guide("ask-dax-continue-draft", stated_in="BIN.md")
                self.assertEqual(
                    [f"{GUIDES}ask-dax-continue-draft: statedIn 'BIN.md', which is not UTF-8 text"],
                    self.problems("ask-dax-continue-draft"),
                )
                out = io.StringIO()
                with contextlib.redirect_stdout(out):
                    self.assertEqual(1, harness.main(["check", "--repo", str(self.repo)]))
                self.assertIn("which is not UTF-8 text", out.getvalue())


if __name__ == "__main__":
    unittest.main()
