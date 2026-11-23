"""Review 2 of PR 157 (ledger item 442, a fix round of task 4.1 of `channel-files`).

"Images only" says: the host lists four image types with a 10 MB limit for `fixme`, THEN
`fixme` accepts those four types up to 10 MB, and `gonk-chat` accepts none. What a channel
accepts is what `Config::file_accepts` hands the inbox, and PR 157 adds the door test that
checks it, `a_declared_channel_yields_its_accepts_and_an_undeclared_one_none`. The scenario's
vector still names only `a_channel_lists_its_file_types_and_limit`, which reads the parsed
fields and never calls `file_accepts`: the vector's test stays green if `file_accepts` returns
nothing, so the scenario's map still points at a test that cannot fail for its THEN.
"""

from __future__ import annotations

import unittest

from test_review_pr153_vectors import CONFIGURATION, rust_tests, vector

IMAGES_ONLY = CONFIGURATION + "a-channel-lists-the-files-it-accepts:scenario:images-only"


class ImagesOnlyVector(unittest.TestCase):
    def test_the_images_only_vector_names_a_test_of_what_a_channel_accepts(self):
        door = rust_tests("artifacts/door")
        named = [step["door"] for step in vector(IMAGES_ONLY)["steps"] if "door" in step]
        reaching = [name for name in named if "file_accepts" in door.get(name, "")]
        self.assertNotEqual(
            reaching,
            [],
            f"the vector names {named}, none of which calls Config::file_accepts, "
            "so none checks what fixme and gonk-chat accept",
        )


if __name__ == "__main__":
    unittest.main()
