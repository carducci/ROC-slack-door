# Tasks

## 1. Telling a posted write apart (`slack`, `door`)

- [ ] 1.1 Add a `ThreadError` variant for a write that posted but was not recorded, carrying the key and an optional `ts`, and map the failed save of `threads.tsv` to it; the test is the finding's reproduction, a directory on `threads.tsv.tmp`, one write, which must answer posted-not-recorded with the ts (A write that posted but was not recorded says so)
- [ ] 1.2 In `artifacts/slack/src/threads.rs`, answer the same variant for a write that opens a thread when Slack's answer has no `ts` or a malformed `ts` (the plain `State` at `threads.rs:170`; usable means `is_slack_ts` accepts it); one test each for no `ts`, a malformed `ts` and a failed save (A write that posted but was not recorded says so)
- [ ] 1.3 Change `SlackTransport::post` or `TransportError` in `artifacts/slack/src/lib.rs` so a post Slack answered `ok` without a usable `ts` is told apart from `TransportError::Unavailable`, change `from_transport` (`threads.rs:40-50`) to match, and have `WebApi::post` in `artifacts/door/src/web_api.rs` answer it (A write that posted but was not recorded says so)
- [ ] 1.4 Give `FakeSlack` in `artifacts/slack/src/lib.rs` a way to record a post and answer no `ts` or a malformed one (`fail_next` fails before recording a post, and `answer_next_post` only supplies a string) (A write that posted but was not recorded says so)
- [ ] 1.5 Map the variant in the door (`artifacts/door/src/serve.rs`) to `Error::Endpoint` whose message begins `posted-not-recorded: ` and then names the key and the `ts`, escaped, never `Unavailable` or `Timeout`; keep a failed post's answers as they are; door tests check the first word of the message, the error's class, and a failed post (A write that posted but was not recorded says so)
- [ ] 1.6 Keep the (channel, key) → `ts` slot in memory when the save fails (change `record`, `threads.rs:184-198`, which rolls it back) and record it in the next save that succeeds; change `a_thread_whose_save_failed_is_not_kept_and_a_retry_opens_it` to the new scenario: a failed save, then a second write, shows one top-level post and one reply, and after the save works the map holds the slot (A write that posted but was not recorded says so)
- [ ] 1.7 Make a reply and `announce` succeed when Slack answered `ok` without a usable `ts`, in `WebApi::post` (`web_api.rs`) and the callers at `threads.rs:125-131` and `serve.rs:252`, so a retry does not post either twice; one test each (A write that posted but was not recorded says so)

## 2. Vectors and archive

- [ ] 2.1 Vectors for the scenarios in `openspec/vectors.json`: add runner steps in `artifacts/slack/tests/vectors.rs`, beside `fail_next` (`vectors.rs:255-265`), that make a save of `threads.tsv` fail ("the next save of threads.tsv fails") and that make a post succeed without a usable `ts` ("the next post is answered ok with no ts"). That the error is an `Endpoint` and not an `Unavailable` is a property of the door's `Error`, which the vectors run against the library cannot see; the door tests of task 1.5 check it (A write that posted but was not recorded says so)
- [ ] 2.2 Archive the change into `openspec/specs/threads/spec.md`
