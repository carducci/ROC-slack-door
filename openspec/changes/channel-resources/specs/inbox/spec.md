## MODIFIED Requirements

### Requirement: An allowed person's reply lands in the inbox

A message posted by one of a channel's speakers as a reply in a thread the door knows in that
channel SHALL be written to `urn:slack:channel:{name}:inbox` as one tuple carrying the thread's
key, the author's Slack user id, the time in UTC and the text. Reading the thread afterwards
SHALL include the message: the thread's resource is cut when the message lands, so a cached
read is never served stale.

#### Scenario: A reply arrives

- **WHEN** a speaker of `gonk-chat` replies "Use the fine-grained token" in its thread for `carducci-54`
- **THEN** one tuple with key `carducci-54`, that person and that text is in `urn:slack:channel:gonk-chat:inbox`

### Requirement: Only allowed people, in the configured channel, are delivered

The door MUST NOT deliver a message from anyone who is not a speaker of the channel it was
posted in, from any bot (including itself), or from any channel the host did not declare. A
speaker of one channel MUST NOT be delivered from another channel where they are not a speaker.

#### Scenario: A stranger's reply is ignored

- **WHEN** a person who is not a speaker of `gonk-chat` replies in its thread for `carducci-54`
- **THEN** nothing is written to any inbox

#### Scenario: The door's own posts are not delivered

- **WHEN** the door posts a reply in a thread
- **THEN** nothing is written to any inbox

#### Scenario: A speaker of one channel in another

- **WHEN** `U2`, a speaker of `fixme` only, posts in `gonk-chat`
- **THEN** nothing is written to any inbox

### Requirement: An allowed person's reaction lands in the inbox

A reaction added by one of a channel's speakers to the opening message of a thread the door
knows in that channel SHALL be written to that channel's inbox as a tuple carrying the key, the
person and the reaction's name.

#### Scenario: A thumbs-up arrives

- **WHEN** a speaker of `gonk-chat` adds `+1` to the opening message of its thread for `carducci-54`
- **THEN** one tuple with key `carducci-54`, that person and reaction `+1` is in `urn:slack:channel:gonk-chat:inbox`

## REMOVED Requirements

### Requirement: A message outside any thread lands without a key

Replaced by "A speaker's top-level message opens a thread": a top-level message now gets a key
the door mints, so a caller can reply in place.

## ADDED Requirements

### Requirement: A speaker's top-level message opens a thread

A top-level message by one of a channel's speakers SHALL be recorded as a thread under a key the
door mints (`m-` followed by the message's time, e.g. `m-1700000000-000001`), durably, and
written to the channel's inbox carrying that key. A reply in a thread the door does not know,
started by someone who is not a speaker, SHALL be written with no key.

#### Scenario: David starts a conversation

- **WHEN** `U2`, a speaker of `fixme`, posts "bug: the save button overlaps the footer" as a new message
- **THEN** one tuple with a minted key, that person and that text is in `urn:slack:channel:fixme:inbox`, and writing to that key in `fixme` replies under David's message

### Requirement: Reading an inbox needs the channel's capability

Sourcing `urn:slack:channel:{name}:inbox` SHALL list that channel's undelivered tuples, and
deleting one SHALL take it, both requiring `urn:cap:slack:channel:{name}:read`.

#### Scenario: Reading another channel's inbox

- **WHEN** a caller holding only `urn:cap:slack:channel:fixme:read` reads `urn:slack:channel:gonk-chat:inbox`
- **THEN** it is refused for lack of `urn:cap:slack:channel:gonk-chat:read`
