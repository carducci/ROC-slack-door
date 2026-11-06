## ADDED Requirements

### Requirement: A write that posted but was not recorded says so

When a write opens a thread (a key with no thread) and the post to Slack succeeds, but a later
step fails, the door MUST answer one error, `posted-not-recorded`, that a caller can tell apart
from a failed post. The later steps are reading the `ts` from Slack's answer and saving the
key's thread to `threads.tsv`; the failures are an answer with no `ts`, a malformed `ts` and a
failed save. A usable `ts` is one `is_slack_ts` accepts. The error MUST be an `Error::Endpoint`
whose message begins `posted-not-recorded: ` and then names the key and the `ts`, escaped: a
usable `ts` as Slack gave it, and for a malformed one what Slack answered, quoted. It MUST NOT
be an `Unavailable` or a `Timeout`, and MUST NOT otherwise invite a retry. The door SHALL NOT
post a second time for one write.

When the save fails after Slack gave a usable `ts`, the door MUST keep the (channel, key) → `ts`
slot in memory, so a later write to that key replies in that thread, and the next save that
succeeds records it. A restart before such a save loses the slot, which the error told the
caller (it names the `ts`).

A failure before or during the post keeps its own answers. A reply to an existing thread, and an
announcement, have nothing to record: when Slack answered `ok` the write succeeds, whether or not
the answer holds a usable `ts`.

#### Scenario: The save fails after the post

- **WHEN** a caller writes "Build is blocked on a token" to `urn:slack:thread:carducci-54`, a key with no thread, Slack accepts the post, and `threads.tsv` cannot be saved
- **THEN** it is refused as `posted-not-recorded`, naming `carducci-54` and the message's `ts`, and one message was posted

#### Scenario: The refusal carries its code in the message

- **WHEN** a write is refused as `posted-not-recorded`
- **THEN** the door's answer is an `Error::Endpoint`, not an `Unavailable` or a `Timeout`, and the first word of its message is `posted-not-recorded:`

#### Scenario: A retry after posted-not-recorded replies in the posted thread

- **WHEN** the write above is refused as `posted-not-recorded`, and the caller writes to `urn:slack:thread:carducci-54` again
- **THEN** the second write is a reply in the thread of the first message, so one top-level message and one reply were posted

#### Scenario: The slot is recorded by the next save that succeeds

- **WHEN** a write was refused as `posted-not-recorded` because the save failed, and a later save of `threads.tsv` succeeds
- **THEN** `threads.tsv` holds the slot for `carducci-54` with the `ts` the error named

#### Scenario: Slack's answer has no ts

- **WHEN** a caller writes to a key with no thread, and Slack accepts the post but its answer has no `ts`
- **THEN** it is refused as `posted-not-recorded`, naming the key, and it is not classed as unavailable

#### Scenario: Slack's answer has a malformed ts

- **WHEN** a caller writes to a key with no thread, and Slack accepts the post but its answer has a `ts` that `is_slack_ts` does not accept
- **THEN** it is refused as `posted-not-recorded`, naming the key and quoting, escaped, what Slack answered

#### Scenario: A reply whose answer has no ts succeeds

- **WHEN** a caller writes to a key that has a thread, and Slack answers `ok` to the reply with no usable `ts`
- **THEN** the write succeeds, and the door did not post the reply twice

#### Scenario: An announcement whose answer has no ts succeeds

- **WHEN** the door announces, and Slack answers `ok` with no usable `ts`
- **THEN** the announcement succeeds, and it was posted once

#### Scenario: A failed post is still a failed post

- **WHEN** Slack refuses the post, or cannot be reached
- **THEN** the answer is the post's own error, not `posted-not-recorded`
