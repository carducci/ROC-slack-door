# A post whose outcome is unknown is not retried

## Why

A post whose outcome is unknown is answered `Unavailable`, which is transient, so a caller's
retry or the kernel's retry overlay may post the text twice. Two cases in `web_api.rs`: an
HTTP 200 from `chat.postMessage` whose body can't be read, and a timeout after the request was
sent. Reproduced against the real `WebApi::post` and a loopback server answering a truncated
`{"ok":true,"ts":"1700000000.000001"`: `Err(Unavailable("unreadable answer: Failed to read JSON: ..."))`,
at main a6db263. ikigai-core notes that a non-idempotent `Sink` needs an idempotency key even
when the error is transient; `chat.postMessage` has none.

Raised in round 1 review of PR 84 (ledger item ikigai-slack#300) as pre-existing; ledger item 324.

## What Changes

- A post whose outcome is unknown is answered as a non-transient `Error::Endpoint` whose message
  begins `post-outcome-unknown: `, so no overlay retries it and a caller can tell it from a post
  that failed before sending.
- A post that failed before it was sent (connection refused, DNS failure, a rate limit, a 5xx
  answer) stays unavailable.
- Reads and reactions are unchanged: repeating them is safe.

## Impact

- Artifacts: `door`, `slack`
- Modified spec: `threads` (a new requirement). `configuration`'s "transient failures are
  unavailable" stays true: it names only a rate limit and an outage, both known not to have posted.

## Resource map

Not needed: an error answer on an existing endpoint.

## Variances

None.

## Out of scope

- An idempotency key or a read-back to find out whether the post landed.
- Telling a failed connect from a failed send beyond what `ureq`'s transport error exposes.
