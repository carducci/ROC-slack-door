"""`tools/harness.py check`: every rule AGENTS.md states is named by a guide in harness.ttl."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

spec = importlib.util.spec_from_file_location("harness", ROOT / "tools" / "harness.py")
harness = importlib.util.module_from_spec(spec)
sys.modules["harness"] = harness
spec.loader.exec_module(harness)


class HarnessCheckTest(unittest.TestCase):
    def test_check_finds_nothing_wrong(self) -> None:
        problems = harness.check(ROOT)
        self.assertEqual([], problems, "\n".join(problems))


if __name__ == "__main__":
    unittest.main()
