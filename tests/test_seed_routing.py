"""`gonk.py seed` routes a task's `[model repo:name]` hint to `agent:` and `repo:` labels.

The requirement is 'The spec chooses the model' (dax-routing). The task line is read by
`open_changes`, which the spec test also uses, so a bracket hint must not hide the task's
trailing `(requirement)` from it.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))

import gonk  # noqa: E402  (the path above)

LAYOUT = {"spec": [{"format": "openspec", "root": "openspec"}]}
DELTA = "## ADDED Requirements\n\n### Requirement: The spec chooses the model\n"


def tasks_in(lines: str, origin: str | None = "https://example.org/acme/widget.git"):
    """The tasks of a one-change repository whose tasks.md is `lines` (bytes-exact)."""
    with tempfile.TemporaryDirectory() as tmp:
        repo = Path(tmp) / "folder-name"
        change = repo / "openspec" / "changes" / "demo"
        (change / "specs" / "cap").mkdir(parents=True)
        (change / "specs" / "cap" / "spec.md").write_text(DELTA, encoding="utf-8")
        (change / "tasks.md").write_bytes(lines.encode("utf-8"))
        subprocess.run(["git", "init", "-q", str(repo)], check=True)
        if origin:
            subprocess.run(["git", "-C", str(repo), "remote", "add", "origin", origin], check=True)
        return gonk.open_changes(repo, LAYOUT)[0].tasks


def one(text: str, **kw):
    return tasks_in(f"- [ ] 1.1 {text}\n", **kw)[0]


class SeedRouting(unittest.TestCase):
    def test_a_hint_routes_to_agent_and_repo_labels(self):
        change = gonk.Change("demo", "Demo", [], [])
        task = one("[sonnet repo:other] Do it (The spec chooses the model)")
        words = gonk.seed_request(change, task)[0]
        self.assertIn("agent:sonnet,repo:other", words)

    def test_a_bracket_hint_keeps_the_last_parenthesis_citing_the_requirement(self):
        for text in (
            "[sonnet repo:x] Do it (The spec chooses the model)",
            "Do it [sonnet repo:x] (The spec chooses the model)",
            "Do it (The spec chooses the model) [sonnet repo:x]",
        ):
            task = one(text)
            self.assertEqual(task.cites, "The spec chooses the model", text)
            self.assertIsNotNone(task.requirement, text)
            self.assertNotIn("[", task.title, text)

    def test_no_hint_no_routing_labels(self):
        change = gonk.Change("demo", "Demo", [], [])
        words = gonk.seed_request(change, one("Do it"))[0]
        self.assertNotIn("agent:", words)
        self.assertNotIn("repo:", words)

    def test_a_human_note_anywhere_stops_the_agent_label(self):
        for note in ("(human: ask)", "[human: ask]", "(human： ask)", "(Human: ask)"):
            for text in (f"[opus] {note} Do it", f"[opus] Do it {note}", f"Do {note} it [opus]"):
                self.assertIsNone(one(text).model, text)

    def test_the_default_repo_is_the_origin_last_segment_on_any_host(self):
        for origin, name in (
            ("https://github.com/acme/widget.git", "widget"),
            ("git@gitlab.example.org:team/sub/tool.git", "tool"),
            ("ssh://host/srv/git/thing/", "thing"),
            ("/srv/git/local-bare", "local-bare"),
        ):
            self.assertEqual(one("[opus] Do it", origin=origin).repo, name, origin)

    def test_the_folder_name_is_the_default_repo_only_without_an_origin(self):
        self.assertEqual(one("[opus] Do it", origin=None).repo, "folder-name")

    def test_a_known_model_bracket_followed_by_a_parenthesis_is_refused_naming_the_line(self):
        with self.assertRaises(gonk.ConfigError) as caught:
            tasks_in("- [ ] 1.1 Fine\n- [ ] 1.2 [opus](The spec chooses the model)\n")
        self.assertIn("tasks.md:2", str(caught.exception))

    def test_refusal_line_numbers_count_only_newlines(self):
        # A lone carriage return, a form feed and U+2028 are splitlines breaks, not line ends.
        text = "- [ ] 1.1 a\rb\x0cc d\n- [ ] 1.2 [opus](R)\n"
        with self.assertRaises(gonk.ConfigError) as caught:
            tasks_in(text)
        self.assertIn("tasks.md:2", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
