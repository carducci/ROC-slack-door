"""Review 1 of PR 153 (ledger item 46, task 4.1 of `channel-files`): the vectors and the design's tests.

A vector marked `"door": true` is skipped by `artifacts/slack/tests/vectors.rs`, and names the
door's own test that runs its scenario instead. "Retention removes a screenshot" says the chore,
which holds only `urn:cap:slack:channel:{name}:file:delete` (design D5), deletes the file and
reading it is then refused as `no-file`. Its vector names `deleting_the_only_name_removes_the_object`,
a unit test of the library's `Files::delete` in `artifacts/slack/src/files.rs`: no door, no
capability, no read. A door whose file endpoint refused every Delete would leave it green.

The design's type-spoofing fitness function feeds "a `file_share` event whose `mimetype` says
`image/png` but whose bytes are an ELF or PE header", so it "fails if someone starts trusting
Slack's type". The executable tests feed a `SlackFile`, which carries no claimed type at all, and
the tests that parse a `file_share` event never serve executable bytes: no test holds a claimed
`image/png` and an executable together, so a door that trusted `mimetype` stays green.
"""

from __future__ import annotations

import json
import re
import unittest

from test_specs_and_vectors import OPENSPEC, ROOT

FILES = "urn:openspec:spec:files:requirement:"
CONFIGURATION = "urn:openspec:spec:configuration:requirement:"
DOOR_ONLY = [
    CONFIGURATION + "a-channel-lists-the-files-it-accepts:scenario:images-only",
    FILES + "reading-a-file-returns-its-bytes:scenario:reading-the-screenshot",
    FILES + "reading-a-file-returns-its-bytes:scenario:the-same-bytes-another-channel",
    FILES + "deleting-a-file-removes-its-name:scenario:retention-removes-a-screenshot",
    FILES + "file-capabilities-are-separate:scenario:a-triage-worker-looks-and-can-t-post",
]
TEST_FN = re.compile(r"#\[test\]\s*(?:#\[[^\]]*\]\s*)*fn\s+(\w+)\s*\(\)\s*\{")


def vector(identifier: str) -> dict:
    vectors = json.loads((OPENSPEC / "vectors.json").read_text(encoding="utf-8"))
    return next(s for s in vectors["scenarios"] if s["id"] == identifier)


def rust_tests(under: str) -> dict[str, str]:
    """Every `#[test]` function under `under`, by name: its body."""
    found = {}
    for path in sorted((ROOT / under).rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for m in TEST_FN.finditer(text):
            depth, end = 1, m.end()
            while depth and end < len(text):
                depth += {"{": 1, "}": -1}.get(text[end], 0)
                end += 1
            found[m.group(1)] = text[m.end() : end]
    return found


class ChannelFilesVectors(unittest.TestCase):
    def test_a_door_only_vector_names_a_test_of_the_door(self):
        door = rust_tests("artifacts/door")
        for identifier in DOOR_ONLY:
            named = [step["door"] for step in vector(identifier)["steps"] if "door" in step]
            with self.subTest(scenario=identifier.rsplit(":", 1)[-1]):
                self.assertTrue(named, "the vector names no door test")
                for name in named:
                    self.assertTrue(name in door, f"{name} is not one of the door's tests")

    def test_an_executable_claimed_as_a_png_is_fed_and_refused(self):
        claims_png = re.compile(r'"mimetype"\s*:\s*"image/png"|file_share\.json')
        executable = re.compile(r"MZ\\x90|\\x7fELF|\bEXE\b|\bELF\b")
        spoofed = [
            name
            for name, body in rust_tests("artifacts").items()
            if claims_png.search(body) and executable.search(body)
        ]
        self.assertNotEqual(
            spoofed, [], "no test feeds a file_share whose mimetype says image/png with executable bytes"
        )


if __name__ == "__main__":
    unittest.main()
