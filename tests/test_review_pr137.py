"""Review of PR #137 (ledger item 417): a defect in `seed`'s model-hint refusal, as a failing test."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tests"))
sys.path.insert(0, str(ROOT / "tools"))

import gonk  # noqa: E402  (the path above)
from test_seed_routing import tasks_in  # noqa: E402  (the path above)


class ReviewPr137(unittest.TestCase):
    def test_a_second_known_model_bracket_followed_by_a_parenthesis_is_refused(self):
        # Only the first `[model]` on the line is checked, so `[sonnet](R)` after `[opus]` slips
        # past the refusal and stays in the title.
        with self.assertRaises(gonk.ConfigError) as caught:
            tasks_in("- [ ] 1.1 Fine\n- [ ] 1.2 [opus] Do [sonnet](The spec chooses the model)\n")
        self.assertIn("tasks.md:2", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
