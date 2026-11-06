# A write that posted but was not recorded

## Why

When `chat.postMessage` succeeds but a later step fails (the answer carries no `ts`, the `ts`
is malformed, or the save of `threads.tsv` fails), `write_at` answers a plain
`ThreadError::State`, which the door maps to `Endpoint("the thread map: …")`. An answer with
no `ts` maps to `TransportError::Unavailable`, which invites a retry. Nothing tells the caller
the text is already in Slack, so a caller that retries posts it a second time and the first
message is orphaned: no key names it, and a person's reply under it lands with no key.

Found by round 1 review of PR 46 (ledger item `ikigai-slack#117`, pre-existing, severity low).
Reproduced at the base `1f99b6f`: with a directory on `threads.tsv.tmp`, the first write
fails, the retry posts a second top-level message, and after a restart the key names nothing.
The threads spec says the door SHALL post and record; a failure between the two has no
defined answer.

Decision by Dax, 2026-11-06: after the post succeeds, every later failure answers one error
the caller can tell apart from a failed post, and the door never posts twice for one write.
Merging this change is Michael's acceptance.

## What Changes

- **One error for a write that opened a thread and posted.** "Posted, not recorded" names the
  key and the `ts`. It covers an answer with no `ts`, a malformed `ts` (one `is_slack_ts` does
  not accept) and a failed save of `threads.tsv`. A reply and an announcement have nothing to
  record: when Slack answered `ok` they succeed, whether or not the answer holds a usable `ts`.
- **The carrier is `Error::Endpoint`.** Its message begins `posted-not-recorded: ` and then
  names the key and the `ts`, escaped, the way `no-channel` carries its code. It is never
  `Unavailable` or `Timeout`: a caller must not retry it as if nothing posted.
- **The posted thread is kept.** After a failed save the door keeps the (channel, key) → `ts`
  slot in memory, so a retry replies in the posted thread instead of opening a second one, and
  the next save that succeeds records it. A restart before that loses the slot, which the error
  told the caller. This reverses PR #46's
  `a_thread_whose_save_failed_is_not_kept_and_a_retry_opens_it`, which becomes a test of the
  new scenario.
- **Before or during the post nothing changes.** A failed post keeps today's answers.

Not in this change: a post whose outcome is unknown (an unreadable 200, a timeout after
sending) can't be told from one that never posted, so it is not `posted-not-recorded`; it is
filed separately as `ikigai-slack#324`.

## Impact

- Artifacts: `slack`, `door`
- Modified spec: `threads` (a new requirement beside "Writing to a new key opens the thread").
- No new dependencies. No configuration.

## Resource map

Not needed: an error answer on an existing endpoint.
