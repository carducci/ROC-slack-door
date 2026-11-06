## ADDED Requirements

### Requirement: A write that posted but was not recorded says so

When the post to Slack succeeds and a later step fails, the door MUST answer one error,
`posted-not-recorded`, that a caller can tell apart from a failed post. The later steps are
reading the `ts` from Slack's answer and saving the key's thread to `threads.tsv`; the failures
are an answer with no `ts`, a malformed `ts`, a refused `ts` and a failed save. The error MUST
name the key, and the `ts` when Slack gave a usable one. It MUST NOT be classed as unavailable
or otherwise invite a retry. The door SHALL NOT post a second time for one write. A failure
before or during the post keeps its own answers.

#### Scenario: The save fails after the post

- **WHEN** a caller writes "Build is blocked on a token" to `urn:slack:thread:carducci-54`, a key with no thread, Slack accepts the post, and `threads.tsv` cannot be saved
- **THEN** it is refused as `posted-not-recorded`, naming `carducci-54` and the message's `ts`, and one message was posted

#### Scenario: A retry does not post again

- **WHEN** the write above is refused as `posted-not-recorded`
- **THEN** the door does not post that text again by itself, and the refusal is not one a caller retries

#### Scenario: Slack's answer has no usable ts

- **WHEN** Slack accepts the post but its answer has no `ts`, or a `ts` that is malformed
- **THEN** it is refused as `posted-not-recorded`, naming the key, and it is not classed as unavailable

#### Scenario: A failed post is still a failed post

- **WHEN** Slack refuses the post, or cannot be reached
- **THEN** the answer is the post's own error, not `posted-not-recorded`
