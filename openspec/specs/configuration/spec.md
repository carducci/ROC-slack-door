## Purpose

Artifacts: `slack`, `door`

What the door reads from its host, what it refuses to do without, and which capability each verb needs.

## Requirements

### Requirement: The host configures the door

The door SHALL take its bot token, its app-level token, the channel, the allowed people, its
state directory and the inbox space's name from the host (a configuration file the host names,
with environment variables overriding it), and MUST refuse to start, naming what is missing,
when any of them is absent. The library crate MUST NOT read the environment or any file for
them: the door passes them in.

#### Scenario: A missing token stops the start

- **WHEN** the door starts with no app-level token configured
- **THEN** it refuses to start, naming the app-level token

### Requirement: Secrets never leave the door

The tokens MUST NOT appear in any representation, description (`Meta`), log line or error the
door produces.

#### Scenario: Describing a thread

- **WHEN** a caller asks for the description of `urn:slack:thread:carducci-54`
- **THEN** the answer contains neither token

### Requirement: Capabilities gate the verbs

Reading a thread SHALL require `urn:cap:slack:read`, posting `urn:cap:slack:post`, and adding or
removing a reaction `urn:cap:slack:react`. A request without the capability MUST be refused
before Slack is called.

#### Scenario: Posting without the capability

- **WHEN** a caller holding only `urn:cap:slack:read` writes to an open thread
- **THEN** it is refused for lack of `urn:cap:slack:post`, and nothing is posted

### Requirement: Slack's transient failures are unavailable, not errors

A rate limit or a Slack outage SHALL be answered as unavailable, carrying Slack's retry-after
when it gives one, so a caller's retry and circuit-breaker overlays apply; an invalid token or a
missing permission SHALL be answered as an error naming it.

#### Scenario: Rate limited

- **WHEN** Slack answers a post with a rate limit and a retry-after of 30 seconds
- **THEN** the write is answered as unavailable with a retry-after of 30 seconds
