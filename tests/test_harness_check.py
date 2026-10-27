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

    def test_a_guide_that_quotes_only_its_heading_fails(self) -> None:
        # ikigai-slack#196: the heading's words alone let the rule's body drift unchecked.
        self.guide("reproduce-before-fix", "`Reproduce before you fix`: do whatever the item says.")
        self.edit(
            "AGENTS.md",
            r"A bug report, a review finding, or a measurement someone else made is a \*\*claim\*\*",
            "Just do what the item says. A bug report is a task",
        )
        self.assertEqual(
            [
                f"{GUIDES}reproduce-before-fix: quotes only the heading of AGENTS.md "
                "`## 3. Reproduce before you fix`; quote, in backticks, a phrase the section "
                "says under it"
            ],
            harness.check(self.repo),
        )

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

    def test_a_line_opening_with_a_backtick_code_span_opens_no_fence(self) -> None:
        # ikigai-slack#198: CommonMark 4.5, a backtick fence's info string holds no backtick.
        self.edit(
            "AGENTS.md",
            r"^(?=1\. Open your pull request as a draft)",
            "``` `#N` ``` is how a code span holds backticks.\n\n",
        )
        self.assertEqual([], harness.check(self.repo))

    def test_a_fence_closes_only_on_its_own_character_at_least_as_long(self) -> None:
        # ikigai-slack#198: a shorter run, or the other character, leaves the fence open.
        self.edit(
            "AGENTS.md",
            r"^(?=1\. Open your pull request as a draft)",
            "````\n```\n~~~~\n## not a heading\n`````\n\n",
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

    def test_inline_markdown_reads_as_its_words(self) -> None:
        # ikigai-slack#199: each form AGENTS.md might use, read as a reader sees it.
        for markdown, words in [
            ("*Really* never.", "Really never."),
            ("_Really_ never.", "Really never."),
            ("***Really*** never.", "Really never."),
            ("__Really__ never.", "Really never."),
            ("**a *b* c** d", "a b c d"),
            ("See [the loop](#the-loop \"t\").", "See the loop."),
            ("See [the loop](#the-loop 't').", "See the loop."),
            ("See [the loop](<#the loop> (t)).", "See the loop."),
            ("See ![the loop](loop.png \"t\").", "See the loop."),
            ("2\\*3 never.", "2*3 never."),
            ("\\*not emphasis\\*", "*not emphasis*"),
            ("\\`not code\\`", "`not code`"),
            ("a\\\nb", "a b"),
            ("&amp; &lt;b&gt; &#42;x&#42; &#x2A; &copy;", "& <b> *x* * ©"),
            ("&nosuch; &amp", "&nosuch; &amp"),
            ("<mailto:a@b.c> <https://x.y/z>", "mailto:a@b.c https://x.y/z"),
            ("`a\\*b` `&amp;` `*c*`", "a\\*b &amp; *c*"),
            ("snake_case_name and 2 * 3 and a*b", "snake_case_name and 2 * 3 and a*b"),
            ("*see `x` here*", "see x here"),
        ]:
            with self.subTest(markdown=markdown):
                self.assertEqual(words, harness.flat(harness.inline(markdown)))

    def test_emphasis_titles_escapes_and_entities_in_agents_md_match(self) -> None:
        # The four reproductions of ikigai-slack#199, through the check.
        for added, quote in [
            ("*Really* never.", "No claim, no write. Really never."),
            ("_Really_ never.", "No claim, no write. Really never."),
            ('See [the loop](#the-loop "t").', "No claim, no write. See the loop."),
            ("2\\*3 never.", "No claim, no write. 2*3 never."),
            ("&amp; never.", "No claim, no write. & never."),
        ]:
            with self.subTest(added=added):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.restore()
                self.edit(
                    "AGENTS.md",
                    r"^- \*\*No claim, no write\.\*\*",
                    f"- **No claim, no write.** {added}",
                )
                self.guide("claim-before-write", f"`{quote}`")
                self.assertEqual([], harness.check(self.repo))

    def test_an_autolink_is_its_address_read_as_nothing_else(self) -> None:
        # ikigai-slack#202: no emphasis and no backslash escape inside an autolink (CommonMark 6.5).
        for markdown, words in [
            ("<https://x.y/_a_>", "https://x.y/_a_"),
            ("<https://x.y/*a*>", "https://x.y/*a*"),
            ("<https://x/\\_a\\_>", "https://x/\\_a\\_"),
            ("*a <https://x.y/*b> c*", "a https://x.y/*b c"),
            ("<a_b_@c.d>", "a_b_@c.d"),
        ]:
            with self.subTest(markdown=markdown):
                self.assertEqual(words, harness.flat(harness.inline(markdown)))
        self.edit(
            "AGENTS.md",
            r"^- \*\*No claim, no write\.\*\*",
            "- **No claim, no write.** See <https://x.y/_a_>.",
        )
        self.guide("claim-before-write", "`No claim, no write. See https://x.y/_a_.`")
        self.assertEqual([], harness.check(self.repo))

    def test_a_private_use_character_reads_as_itself(self) -> None:
        # ikigai-slack#203: a plane-15 character neither crashes the check nor reads as code.
        for char in ("\U000f0fff", "\U000f0000"):
            with self.subTest(char=hex(ord(char))):
                self.assertEqual(f"x {char} y c", harness.flat(harness.inline(f"x {char} y `c`")))
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.edit(
                    "AGENTS.md",
                    r"^(?=1\. Open your pull request as a draft)",
                    f"x {char} y\n\n",
                )
                self.assertEqual([], harness.check(self.repo))
                section = harness.read_sections(self.repo / "AGENTS.md")["Ask Dax, don't guess"]
                self.assertIn(f"x {char} y", section.body)

    def test_unequal_delimiter_runs_and_balanced_parentheses_read_as_commonmark(self) -> None:
        # ikigai-slack#204: CommonMark 6.2's delimiter runs, and 6.3's link destinations.
        for markdown, words in [
            ("**Really* never.", "*Really never."),
            ("*Really** never.", "Really* never."),
            ("*foo**bar**baz*", "foobarbaz"),
            ("*foo**bar*", "foo**bar"),
            ("***strong emph***", "strong emph"),
            ("**foo*bar*baz**", "foobarbaz"),
            ("*(*foo*)*", "(foo)"),
            ("foo*bar*", "foobar"),
            ("foo_bar_", "foo_bar_"),
            ("_foo_bar_baz_", "foo_bar_baz"),
            ("a * b * c", "a * b * c"),
            ("*a `*`*", "a *"),
            ("See [the loop](x(y)).", "See the loop."),
            ("[link](foo(and(bar)))", "link"),
            ("[link](foo(and(bar))", "[link](foo(and(bar))"),
            ("[link](foo\\)bar)", "link"),
            ("[link]()", "link"),
            ("[link](<a b> \"t\")", "link"),
            ("[*a*](b)*", "a*"),
            ("[a [b](c) d](e)", "[a b d](e)"),
            ("[a `]` b](c)", "a ] b"),
            ("[a] b", "[a] b"),
            ("*a\n\nb*", "*a b*"),
            ("`a\n\nb`", "`a b`"),
        ]:
            with self.subTest(markdown=markdown):
                self.assertEqual(words, harness.flat(harness.inline(markdown)))
        for added, quote in [
            ("**Really* never.", "No claim, no write. *Really never."),
            ("See [the loop](x(y)).", "No claim, no write. See the loop."),
        ]:
            with self.subTest(added=added):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.restore()
                self.edit(
                    "AGENTS.md",
                    r"^- \*\*No claim, no write\.\*\*",
                    f"- **No claim, no write.** {added}",
                )
                self.guide("claim-before-write", f"`{quote}`")
                self.assertEqual([], harness.check(self.repo))

    def test_entities_are_the_named_or_numbered_character_or_their_text(self) -> None:
        for markdown, words in [
            ("&ampx; &amp;", "&ampx; &"),
            ("&#0; &#x110000;", "� �"),
            ("&#42;a&#42;", "*a*"),
        ]:
            with self.subTest(markdown=markdown):
                self.assertEqual(words, harness.flat(harness.inline(markdown)))

    def test_a_quote_spanning_the_heading_and_the_body_says_so(self) -> None:
        # ikigai-slack#205: a quote no reader sees, refused with a sentence true for it.
        self.guide("reproduce-before-fix", "`Reproduce before you fix A bug report`")
        self.assertEqual(
            [
                f"{GUIDES}reproduce-before-fix: quotes `Reproduce before you fix A bug report`, "
                "which runs from the heading of AGENTS.md `## 3. Reproduce before you fix` "
                "into its body; quote, in backticks, a phrase of one or the other"
            ],
            harness.check(self.repo),
        )

    def test_a_form_the_check_does_not_read_is_named_when_a_quote_is_not_found(self) -> None:
        # Raw HTML, a link reference definition and a setext underline are not read, so a quote
        # the check can't find there is not called one the section does not say.
        for added, clause in [
            ("<b>Really</b> never.", "the section has raw HTML"),
            ("[Really][r] never.\n\n[r]: https://x.y", "the file has a link reference definition"),
            ("Really\n===\nnever.", "the section has a line of only `=` or `-` under a line of text"),
        ]:
            with self.subTest(added=added):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.restore()
                self.edit("AGENTS.md", r"^(?=1\. Open your pull request as a draft)", f"{added}\n\n")
                self.guide("ask-dax-dont-guess", "`Really never.` `Never mark it ready for review.`")
                self.assertEqual(
                    [
                        f"{GUIDES}ask-dax-dont-guess: quotes `Really never.`, which the check "
                        f"can't find in AGENTS.md `## Ask Dax, don't guess`: {clause}, "
                        "which the check does not read"
                    ],
                    self.problems("ask-dax-dont-guess"),
                )

    def test_an_image_may_hold_a_link(self) -> None:
        # ikigai-slack#217: a link deactivates only the `[` openers before it, not `![`
        # (CommonMark 6.3), so an image's text may hold a link.
        for markdown, words in [
            ("![a [b](c)](d)", "a b"),
            ("![[[foo](uri1)](uri2)](uri3)", "[foo](uri2)"),
            ("[a ![b](c) d](e)", "a b d"),
        ]:
            with self.subTest(markdown=markdown):
                self.assertEqual(words, harness.flat(harness.inline(markdown)))
        self.edit(
            "AGENTS.md",
            r"^- \*\*No claim, no write\.\*\*",
            "- **No claim, no write.** See ![a [b](c)](d).",
        )
        self.guide("claim-before-write", "`No claim, no write. See a b.`")
        self.assertEqual([], harness.check(self.repo))

    def test_a_block_form_the_check_does_not_read_is_named(self) -> None:
        # ikigai-slack#218: an indented code block or a block quote is named, not misread.
        for added, quote, clause in [
            ("    *Really* never.", "*Really* never.", "the section has an indented code block"),
            ("\t*Really* never.", "*Really* never.", "the section has an indented code block"),
            ("> Really\n> never.", "Really never.", "the section has a block quote"),
            ("- a\n\n      *Really* never.", "*Really* never.", "the section has an indented code block"),
            ("-     *Really* never.", "*Really* never.", "the section has an indented code block"),
            ("- > Really\n  > never.", "Really never.", "the section has a block quote"),
        ]:
            with self.subTest(added=added):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.restore()
                self.edit("AGENTS.md", r"^(?=1\. Open your pull request as a draft)", f"{added}\n\n")
                self.guide("ask-dax-dont-guess", f"`{quote}` `Never mark it ready for review.`")
                self.assertEqual(
                    [
                        f"{GUIDES}ask-dax-dont-guess: quotes `{quote}`, which the check "
                        f"can't find in AGENTS.md `## Ask Dax, don't guess`: {clause}, "
                        "which the check does not read"
                    ],
                    self.problems("ask-dax-dont-guess"),
                )

    def test_block_forms_the_check_reads_are_read_as_commonmark_does(self) -> None:
        # ikigai-slack#218: every other block form is read, each as a reader sees it.
        for added, words in [
            # A list item ends the paragraph before it; a line indented to an item's text
            # after a blank line is a paragraph of the item, not code; a marker is its text.
            ("- *Really\n- never.*", "- *Really - never.*"),
            ("- a\n\n  *Really* never.", "- a Really never."),
            ("1. a\n2. *Really\n3. never.*", "1. a 2. *Really 3. never.*"),
            # A list starting at 2 does not interrupt a paragraph, so its line continues it.
            ("*Really\n2. never.*", "Really 2. never."),
            # A thematic break ends a paragraph and shows no words.
            ("*Really\n***\nnever.*", "*Really never.*"),
            ("*Really\n- - -\nnever.*", "*Really never.*"),
            # An ATX heading in a section is its words, closing #s dropped, a block of its own.
            ("### *Really ###\nnever.*", "*Really never.*"),
            ("- ## *Really*\n\nnever.", "- Really never."),
            # A fence in a list item closes at the item's indentation.
            ("- ```\n  *Really*\n  ```\n\nnever.", "- *Really* never."),
        ]:
            with self.subTest(added=added):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.edit("AGENTS.md", r"^(?=1\. Open your pull request as a draft)", f"{added}\n\n")
                section = harness.read_sections(self.repo / "AGENTS.md")["Ask Dax, don't guess"]
                self.assertIn(f"this file settles. {words} 1. Open", section.body)
                self.assertEqual((), section.unread)

    def test_a_heading_inside_another_block_opens_no_section(self) -> None:
        # ikigai-slack#218: `## ` opens a section only as a top-level ATX heading, and an
        # HTML block runs to its end, a `## ` line inside it included (CommonMark 4.6).
        for added in ["<div>\n## Really\n</div>", "<!--\n\n## Really\n\n-->", "- a\n\n  ## Really"]:
            with self.subTest(added=added):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.edit("AGENTS.md", r"^(?=1\. Open your pull request as a draft)", f"{added}\n\n")
                self.assertEqual([], harness.check(self.repo))
                self.assertNotIn("Really", harness.read_sections(self.repo / "AGENTS.md"))
        # A top-level one does, however it is written: indented, by a tab, or closed by #s.
        for heading in [" ## Really ##", "##\tReally", "## Really #"]:
            with self.subTest(heading=heading):
                shutil.copy(ROOT / "AGENTS.md", self.repo / "AGENTS.md")
                self.edit("AGENTS.md", r"^(?=1\. Open your pull request as a draft)", f"{heading}\n\n")
                self.assertIn("Really", harness.read_sections(self.repo / "AGENTS.md"))

    # statedIn is a file inside this repository, compared by what it resolves to.

    def test_a_stated_in_path_outside_the_repository_fails(self) -> None:
        # P1 and P1b of ikigai-slack#187.
        (self.outside / "x").mkdir()
        shutil.copy(self.repo / "AGENTS.md", self.outside / "x" / "RULES.md")
        (self.repo / "out").symlink_to(self.outside / "x" / "RULES.md")
        for name in ("../x/RULES.md", "out"):
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

    def test_an_absolute_stated_in_path_is_not_relative_to_the_repository(self) -> None:
        # ikigai-slack#197: refused whether it names a file outside the repository or inside.
        (self.outside / "x").mkdir()
        shutil.copy(self.repo / "AGENTS.md", self.outside / "x" / "RULES.md")
        for name in (str(self.outside / "x" / "RULES.md"), str(self.repo / "AGENTS.md")):
            with self.subTest(name=name):
                self.restore()
                self.guide("ask-dax-continue-draft", stated_in=name)
                self.assertEqual(
                    [
                        f"{GUIDES}ask-dax-continue-draft: statedIn {name!r}, "
                        "which is not a path relative to this repository's root"
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


CONTROLS = "urn:iki:harness:starter:control:"


class ControlCodeTest(unittest.TestCase):
    """A control's enforcedAt names a file inside this repository, compared by what it
    resolves to, as a guide's statedIn is (ikigai-slack#193)."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.outside = Path(self.tmp.name)
        self.repo = self.outside / "repo"
        shutil.copytree(ROOT, self.repo, ignore=shutil.ignore_patterns(".git", "target"))
        self.assertEqual([], harness.check(self.repo), "the copy agrees before it is changed")
        (self.outside / "x").mkdir()
        shutil.copy(self.repo / ".github/workflows/ci.yml", self.outside / "x" / "ci.yml")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def enforced_at(self, where: str) -> list[str]:
        """Point the copy's ctl:ci-checks at `where`, and check: the problems it is named in."""
        path = self.repo / ".gonk/harness/harness.ttl"
        text = path.read_text(encoding="utf-8")
        start = text.index("ctl:ci-checks a harness:Control ;\n")
        end = text.index(" .\n", start)
        block = text[start:end]
        self.assertIn('harness:enforcedAt ".github/workflows/ci.yml"', block)
        block = block.replace('harness:enforcedAt ".github/workflows/ci.yml"', f'harness:enforcedAt "{where}"')
        path.write_text(text[:start] + block + text[end:], encoding="utf-8")
        return [p for p in harness.check(self.repo) if p.startswith(CONTROLS + "ci-checks:")]

    def test_an_enforced_at_path_outside_the_repository_fails(self) -> None:
        where = "../x/ci.yml"
        self.assertEqual(
            [f"{CONTROLS}ci-checks: enforcedAt {where}, and {where} is outside this repository"],
            self.enforced_at(where),
        )

    def test_an_absolute_enforced_at_path_is_not_relative_to_the_repository(self) -> None:
        # Refused whether it names a file outside the repository or inside, as statedIn is.
        for where in (str(self.outside / "x" / "ci.yml"), str(self.repo / ".github/workflows/ci.yml")):
            with self.subTest(where=where):
                shutil.copy(ROOT / ".gonk/harness/harness.ttl", self.repo / ".gonk/harness/harness.ttl")
                self.assertEqual(
                    [
                        f"{CONTROLS}ci-checks: enforcedAt {where}, and {where} "
                        "is not a path relative to this repository's root"
                    ],
                    self.enforced_at(where),
                )

    def test_an_enforced_at_file_that_is_not_here_still_fails(self) -> None:
        where = ".github/workflows/nowhere.yml"
        self.assertEqual(
            [f"{CONTROLS}ci-checks: enforcedAt {where}, and {where} is not in this repository"],
            self.enforced_at(where),
        )


if __name__ == "__main__":
    unittest.main()
