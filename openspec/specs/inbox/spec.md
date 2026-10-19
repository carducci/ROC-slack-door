## Purpose

Artifacts: `slack`, `door`

What allowed people say in the configured channel that the door delivers, and at most once.

## Requirements

### Requirement: An allowed person's reply lands in the inbox

A message posted by an allowed person as a reply in a thread the door opened SHALL be written to
the configured inbox space as one tuple carrying the thread's key, the author's Slack user id,
the time in UTC and the text. Reading the thread afterwards SHALL include the message: the
thread's resource is cut when the message lands, so a cached read is never served stale.

#### Scenario: A reply arrives

- **WHEN** an allowed person replies "Use the fine-grained token" in the thread for `carducci-54`
- **THEN** one tuple with key `carducci-54`, that person and that text is in the inbox

### Requirement: Only allowed people, in the configured channel, are delivered

The door MUST NOT deliver messages from people outside the allowed list, from any bot
(including itself), or from any channel other than the configured one.

#### Scenario: A stranger's reply is ignored

- **WHEN** a person who is not allowed replies in the thread for `carducci-54`
- **THEN** nothing is written to the inbox

#### Scenario: The door's own posts are not delivered

- **WHEN** the door posts a reply in a thread
- **THEN** nothing is written to the inbox

### Requirement: A message outside any thread lands without a key

A top-level message by an allowed person in the configured channel, or a reply in a thread the
door did not open, SHALL be written to the inbox as a tuple with no key, so a host can start a
conversation from it.

#### Scenario: A person starts a conversation

- **WHEN** an allowed person posts "What's blocked today?" as a new message in the channel
- **THEN** one tuple with no key and that text is in the inbox

### Requirement: An allowed person's reaction lands in the inbox

A reaction added by an allowed person to a thread's opening message SHALL be written to the
inbox as a tuple carrying the key, the person and the reaction's name.

#### Scenario: A thumbs-up arrives

- **WHEN** an allowed person adds `+1` to the opening message of the thread for `carducci-54`
- **THEN** one tuple with key `carducci-54`, that person and reaction `+1` is in the inbox

### Requirement: Each Slack event is delivered at most once

Slack redelivers an event it thinks was not acknowledged. The door SHALL write each event to the
inbox at most once, by the event's id, across restarts.

#### Scenario: A redelivered event

- **WHEN** Slack delivers the same reply event twice
- **THEN** one tuple is in the inbox
