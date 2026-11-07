"""Review of PR 112 (ledger item 59, task 5.1): a vector runs the scenario it is named for.

`openspec/changes/channel-resources/specs/inbox/spec.md`, "David starts a conversation":
WHEN `U2`, a speaker of `fixme`, posts a new message, THEN its tuple is in
`urn:slack:channel:fixme:inbox`, and writing to that key in `fixme` replies under it. Task 5.1
updates the vectors for every changed and new scenario, but this one still feeds `U1` in
`gonk-chat` and writes in `gonk-chat`, so routing a speaker's message of the second channel to
the wrong inbox, or its reply to the wrong channel, leaves every vector green.
"""

from __future__ import annotations

import json
import unittest

from test_specs_and_vectors import OPENSPEC

DAVID = (
    "urn:openspec:spec:inbox:requirement:a-speaker-s-top-level-message-opens-a-thread"
    ":scenario:david-starts-a-conversation"
)


def vector(identifier: str) -> dict:
    vectors = json.loads((OPENSPEC / "vectors.json").read_text(encoding="utf-8"))
    return next(s for s in vectors["scenarios"] if s["id"] == identifier)


def steps(scenario: dict, kind: str) -> list:
    return [step[kind] for step in scenario["steps"] if kind in step]


class DavidStartsAConversation(unittest.TestCase):
    def setUp(self):
        self.scenario = vector(DAVID)

    def test_the_message_is_u2_posting_in_fixme(self):
        messages = [feed["message"] for feed in steps(self.scenario, "feed") if "message" in feed]
        self.assertTrue(messages, "the vector feeds no message")
        self.assertEqual(
            [(m.get("user"), m.get("channel")) for m in messages],
            [("U2", "fixme")],
            "the vector does not feed U2's message in fixme",
        )

    def test_the_tuple_is_in_fixmes_inbox(self):
        delivered = [d for inbox in steps(self.scenario, "inbox") for d in inbox]
        self.assertTrue(delivered, "the vector expects no delivery")
        self.assertEqual(
            [d.get("channel") for d in delivered],
            ["fixme"],
            "the vector does not say the tuple is in fixme's inbox",
        )

    def test_the_reply_is_written_in_fixme_and_posted_there(self):
        writes = steps(self.scenario, "write")
        self.assertEqual([w.get("channel") for w in writes], ["fixme"], "the reply is not written in fixme")
        posts = [p for listed in steps(self.scenario, "posts") for p in listed]
        self.assertEqual([p.get("channel") for p in posts], ["C2"], "the reply is not posted in fixme's channel")


if __name__ == "__main__":
    unittest.main()
