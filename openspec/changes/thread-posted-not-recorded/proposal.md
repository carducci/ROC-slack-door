# A write that posted but was not recorded

## Why

When `chat.postMessage` succeeds but a later step fails (the answer carries no `ts`, the `ts`
is malformed or refused, or the save of `threads.tsv` fails), `write_at` answers a plain
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

- **One error for every failure after the post.** "Posted, not recorded" names the key and the
  `ts` when there is one. It covers an answer with no `ts`, a malformed `ts`, a refused `ts`
  and a failed save of `threads.tsv`.
- **It is not a retryable error.** It is not `Unavailable`: a caller must not retry it, since
  the text is already in Slack.
- **Before or during the post nothing changes.** A failed post keeps today's answers.

## Impact

- Artifacts: `slack`, `door`
- Modified spec: `threads` (a new requirement beside "Writing to a new key opens the thread").
- No new dependencies. No configuration.

## Resource map

Not needed: an error answer on an existing endpoint.
