## ADDED Requirements

### Requirement: A speaker's post with files lands in the inbox

A message by one of a channel's speakers that carries files (Slack's `file_share`) SHALL be
written to the channel's inbox as one tuple with the key, person, time and text, as any message,
plus one entry per file: either its resource name, or the reason it was refused. A message with
files in a channel that accepts none SHALL land with each file refused as `files-not-accepted`.
Edits and deletions MUST still not be delivered.

#### Scenario: A screenshot and a line

- **WHEN** David, a speaker of `fixme`, posts "cosmetic: the save button overlaps the footer" with one PNG screenshot
- **THEN** one tuple with that text and one file named `urn:slack:channel:fixme:file:b3:` followed by the PNG's BLAKE3 digest is in `urn:slack:channel:fixme:inbox`

#### Scenario: A file in a channel that takes none

- **WHEN** a speaker of `gonk-chat`, which lists no file types, posts a message with a PNG
- **THEN** one tuple with the text is in the inbox, and its one file is refused as `files-not-accepted`

### Requirement: A file is kept by its content

The door SHALL download each accepted file with its bot token and store the bytes once, under
their BLAKE3 content address (`b3:` and 64 hex digits), at
`objects/{first two hex}/{next two hex}/{rest}` under the state directory, through a
capability-gated, jailed file store. A file whose content is already stored MUST NOT be
written again. Stored bytes MUST NOT change.

#### Scenario: The same screenshot twice

- **WHEN** David posts the same PNG in two messages
- **THEN** both tuples name the same file, and one object is stored

### Requirement: A file's type comes from its bytes

The door SHALL decide a file's media type by sniffing its bytes, and MUST ignore the filename
and the type Slack reports. A file whose sniffed type the channel doesn't accept MUST be
refused as `file-type-refused`, and one larger than the channel's limit as `file-too-large`,
before it is stored. A file Slack can't serve SHALL be noted as `file-unavailable`, and the
message still lands.

#### Scenario: A renamed executable

- **WHEN** David posts a file named `screenshot.png` whose bytes are a Windows executable
- **THEN** the file is refused as `file-type-refused`, and nothing is stored

#### Scenario: Too large

- **WHEN** a 12 MB PNG is posted in `fixme`, whose limit is 10 MB
- **THEN** the file is refused as `file-too-large`, and nothing is stored

### Requirement: Reading a file returns its bytes

Sourcing `urn:slack:channel:{name}:file:b3:{hex}` SHALL return the stored bytes with their
sniffed media type, and SHALL be cacheable without expiry, because the content can't change.
A name the channel never received MUST be refused as `no-file`, even when another channel holds
the same bytes. A malformed address MUST be refused as `bad-file`.

#### Scenario: Reading the screenshot

- **WHEN** a caller holding `urn:cap:slack:channel:fixme:file:read` reads the file from David's post
- **THEN** it gets the PNG's bytes as `image/png`

#### Scenario: The same bytes, another channel

- **WHEN** a caller reads `urn:slack:channel:gonk-chat:file:b3:<digest>` for bytes that only `fixme` received
- **THEN** it is refused as `no-file`

### Requirement: Deleting a file removes its name

Deleting `urn:slack:channel:{name}:file:b3:{hex}` SHALL remove that channel's name for the
bytes, and SHALL remove the stored object once no channel names it. Deleting a name that isn't
there SHALL succeed and change nothing.

#### Scenario: Retention removes a screenshot

- **WHEN** the retention chore deletes the file from David's post, and no other channel received those bytes
- **THEN** reading it is refused as `no-file`, and its object is gone

### Requirement: File capabilities are separate

Reading a file SHALL require `urn:cap:slack:channel:{name}:file:read`, and deleting one
`urn:cap:slack:channel:{name}:file:delete`. Neither MUST admit posting, reacting or reading the
inbox, and `…:read` MUST NOT admit reading files.

#### Scenario: A triage worker looks and can't post

- **WHEN** a caller holding only `urn:cap:slack:channel:fixme:file:read` writes to a thread in `fixme`
- **THEN** it is refused for lack of `urn:cap:slack:channel:fixme:post`
