"""`.gonk/audits.toml` is a file ikigraph's scheduler accepts.

ikigraph's artifacts/team/schedule/due.py loads every repository's runs through
artifacts/team/schedule/runs.py (`load_repo`, then `_validate`), which refuses an audit that
does not "name exactly two auditors, each a non-empty string", or that names one auditor
twice. A refused file means no run in it ever comes due. These are that loader's audit
rules, restated here so this repository's CI holds them.
"""

from __future__ import annotations

import re
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
AUDITS = ROOT / ".gonk" / "audits.toml"


@unittest.skipUnless(AUDITS.is_file(), "no .gonk/audits.toml")
class AuditsTomlTest(unittest.TestCase):
    def runs(self) -> list[dict]:
        with open(AUDITS, "rb") as f:
            return tomllib.load(f)["run"]

    def test_each_audit_names_exactly_two_distinct_auditors(self):
        for run in self.runs():
            if run.get("kind") != "audit":
                continue
            auditors = run["auditors"]
            with self.subTest(run=run["id"]):
                self.assertIsInstance(auditors, list)
                self.assertEqual(len(auditors), 2, f"runs.py refuses {auditors!r}: not exactly two auditors")
                self.assertTrue(all(isinstance(a, str) and a for a in auditors))
                self.assertNotEqual(auditors[0], auditors[1])

    def test_each_run_has_the_fields_runs_py_requires(self):
        for run in self.runs():
            with self.subTest(run=run["id"]):
                self.assertRegex(run["id"], re.compile(r"\A[A-Za-z0-9._-]+\Z"))
                self.assertIn(run["kind"], ("audit", "chore"))
                self.assertRegex(run["every"], re.compile(r"\A[1-9][0-9]*d\Z"))
                if run["kind"] == "audit":
                    self.assertTrue(isinstance(run["lens"], str) and run["lens"])
                    self.assertIs(type(run["budget"]), int)
                    self.assertGreater(run["budget"], 0)
                    scope = run["scope"]
                    self.assertTrue(isinstance(scope, list) and scope)
                    for glob in scope:
                        self.assertTrue(list(ROOT.glob(glob)), f"scope glob {glob!r} matches no file")


if __name__ == "__main__":
    unittest.main()
