"""Review 1 of PR 149 (ledger item 46, task 4.1 of `channel-files`): a vector runs its scenario.

`artifacts/slack/tests/vectors.rs` skips a vector marked `"door": true`, which is for "a scenario
the library cannot reach (it reads the door's configuration)". The intake scenarios of
`channel-files` are reached by the library's `Inbox::deliver_to` and `FakeSlack::serve_file`,
but their vectors are marked door-only and hold only the name of a unit test. The unit test for
"A screenshot and a line" checks the file's name and never the text or the inbox: a door that
sends a post with a file to `gonk-chat`'s inbox, with its text dropped, leaves every test green.
"""

from __future__ import annotations

import json
import unittest

from test_specs_and_vectors import OPENSPEC

FILES = "urn:openspec:spec:files:requirement:"
SCREENSHOT = FILES + "a-speaker-s-post-with-files-lands-in-the-inbox:scenario:a-screenshot-and-a-line"
INTAKE = [
    SCREENSHOT,
    FILES + "a-speaker-s-post-with-files-lands-in-the-inbox:scenario:a-file-in-a-channel-that-takes-none",
    FILES + "a-file-is-kept-by-its-content:scenario:the-same-screenshot-twice",
    FILES + "a-file-s-type-comes-from-its-bytes:scenario:a-renamed-executable",
    FILES + "a-file-s-type-comes-from-its-bytes:scenario:too-large",
]


def vector(identifier: str) -> dict:
    vectors = json.loads((OPENSPEC / "vectors.json").read_text(encoding="utf-8"))
    return next(s for s in vectors["scenarios"] if s["id"] == identifier)


class IntakeVectors(unittest.TestCase):
    def test_an_intake_scenario_runs_against_the_fake(self):
        for identifier in INTAKE:
            with self.subTest(scenario=identifier.rsplit(":", 1)[-1]):
                self.assertFalse(
                    vector(identifier).get("door", False),
                    "the library reaches this scenario, but its vector is skipped as door-only",
                )

    def test_a_screenshot_and_a_line_lands_with_its_text_in_fixmes_inbox(self):
        delivered = [d for step in vector(SCREENSHOT)["steps"] for d in step.get("inbox", [])]
        self.assertEqual(
            [(d.get("channel"), d.get("text")) for d in delivered],
            [("fixme", "cosmetic: the save button overlaps the footer")],
            "the vector does not check the tuple's text in fixme's inbox",
        )


if __name__ == "__main__":
    unittest.main()
