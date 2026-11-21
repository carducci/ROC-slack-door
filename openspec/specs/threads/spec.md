## Purpose

Artifacts: `slack`, `door`

Conversations named by a key the caller chooses: opening, replying, reading, and existence.

## Requirements

### Requirement: A thread is named by a key the caller chooses

A thread SHALL be the resource `urn:slack:channel:{name}:thread:{key}`, where the key is 1 to
80 characters of letters, digits, `.`, `_` and `-`, unique within its channel. A request whose
key is empty, longer or has any other character MUST be refused as `bad-key`.

#### Scenario: A key with a space is refused

- **WHEN** a caller writes to `urn:slack:channel:gonk-chat:thread:item 54`
- **THEN** it is refused as `bad-key`

#### Scenario: The same key in two channels

- **WHEN** a caller writes to `urn:slack:channel:gonk-chat:thread:x` and then to `urn:slack:channel:fixme:thread:x`
- **THEN** two threads are opened, one in each channel

### Requirement: Writing to a new key opens the thread

Sinking text to a key the door has no thread for in that channel SHALL post it as a new
top-level message in that channel and record that message as the key's thread, durably, so the
key names the same thread after the door restarts.

#### Scenario: Opening a thread

- **WHEN** a caller writes "Build is blocked on a token" to `urn:slack:channel:gonk-chat:thread:carducci-54`, a key with no thread
- **THEN** one top-level message with that text is posted in `gonk-chat`, and the key names its thread

#### Scenario: The thread survives a restart

- **WHEN** the door restarts after opening the thread for `carducci-54` in `gonk-chat`, and a caller writes to it again
- **THEN** the text is posted as a reply in the same thread, not as a new top-level message

### Requirement: Writing to an open thread replies in it

Sinking text to a key that names a thread SHALL post it as a reply in that thread.

#### Scenario: Replying

- **WHEN** a caller writes "Here is the command" to `urn:slack:channel:gonk-chat:thread:carducci-54`, which is open
- **THEN** the text is posted as a reply in that thread

### Requirement: A message has text within Slack's limit

The door MUST refuse to post empty or whitespace-only text as `empty-message`, and text longer
than 40000 characters as `message-too-long`, without calling Slack.

#### Scenario: Empty text is refused

- **WHEN** a caller writes "" to an open thread
- **THEN** it is refused as `empty-message`, and nothing is posted

### Requirement: Reading a thread returns the conversation

Sourcing a thread SHALL return its messages, the opening message first and the replies in the
order Slack holds them, as `application/json`: for each message its author (the Slack user id,
and whether it was the door's own bot), its time in UTC and its text. Slack ids of the channel
and messages MUST NOT appear.

#### Scenario: Reading a conversation

- **WHEN** a thread was opened with "Blocked on a token" and a person replied "Which token?"
- **THEN** reading it returns two messages in that order, the first by the bot and the second by that person

### Requirement: An unknown key is not a thread

`Exists` on a key with no thread in that channel SHALL answer false, and sourcing it MUST be
refused as `no-thread`, without calling Slack.

#### Scenario: Reading a key with no thread

- **WHEN** a caller reads `urn:slack:channel:gonk-chat:thread:never-opened`
- **THEN** it is refused as `no-thread`

### Requirement: Writing to a channel posts a new message

Sinking text to `urn:slack:channel:{name}` SHALL post it as a new top-level message in that
channel, belonging to no thread, under the same text limits as a thread.

#### Scenario: An announcement

- **WHEN** a caller writes "Door restarted" to `urn:slack:channel:gonk-chat`
- **THEN** one top-level message with that text is posted in `gonk-chat`, and no key names it

### Requirement: Threads opened before channels keep their keys

On its first start with channels declared, the door SHALL file each thread it opened before
under the declared channel whose Slack id it was opened in, under the same key. A thread whose
channel is no longer declared SHALL be kept and served by no name.

#### Scenario: An old thread after the upgrade

- **WHEN** the door had opened `carducci-54` in the channel now declared as `gonk-chat`, and is upgraded
- **THEN** writing to `urn:slack:channel:gonk-chat:thread:carducci-54` replies in that same thread
