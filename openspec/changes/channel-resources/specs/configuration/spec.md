## MODIFIED Requirements

### Requirement: The host configures the door

The door SHALL take its bot token, its app-level token, its state directory and its channels
from the host (a configuration file the host names, with environment variables overriding it),
and MUST refuse to start, naming what is missing, when any of them is absent. The library crate
MUST NOT read the environment or any file for them: the door passes them in.

#### Scenario: A missing token stops the start

- **WHEN** the door starts with no app-level token configured
- **THEN** it refuses to start, naming the app-level token

### Requirement: Secrets never leave the door

The tokens MUST NOT appear in any representation, description (`Meta`), log line or error the
door produces.

#### Scenario: Describing a thread

- **WHEN** a caller asks for the description of `urn:slack:channel:gonk-chat:thread:carducci-54`
- **THEN** the answer contains neither token

### Requirement: Capabilities gate the verbs

Each verb SHALL require a capability scoped to the channel it acts in:
`urn:cap:slack:channel:{name}:read` to read a thread or the inbox, `…:post` to write to the
channel or a thread, and `…:react` to add or remove a reaction. A request without the
capability MUST be refused before Slack is called. A capability for one channel MUST NOT
admit a request in another.

#### Scenario: Posting without the capability

- **WHEN** a caller holding only `urn:cap:slack:channel:gonk-chat:read` writes to an open thread in `gonk-chat`
- **THEN** it is refused for lack of `urn:cap:slack:channel:gonk-chat:post`, and nothing is posted

#### Scenario: A capability for another channel

- **WHEN** a caller holding only `urn:cap:slack:channel:gonk-chat:post` writes to `urn:slack:channel:fixme:thread:fixme-1`
- **THEN** it is refused for lack of `urn:cap:slack:channel:fixme:post`, and nothing is posted

## ADDED Requirements

### Requirement: The host declares the channels

The configuration SHALL list the channels the door serves, each by a name of its own (1 to 40
lowercase letters, digits and `-`) with its Slack channel id and the Slack user ids of its
speakers. The door MUST refuse to start when no channel is declared, when a name is invalid or
declared twice, or when a channel has no id or no speakers, naming the channel. A name that is
not declared SHALL be refused as `no-channel`, without calling Slack.

#### Scenario: Two channels

- **WHEN** the host declares `gonk-chat` with speaker `U1` and `fixme` with speaker `U2`
- **THEN** the door serves `urn:slack:channel:gonk-chat` and `urn:slack:channel:fixme`, and no other channel

#### Scenario: A channel with no speakers stops the start

- **WHEN** the host declares `fixme` with an id and no speakers
- **THEN** the door refuses to start, naming `fixme`

#### Scenario: An undeclared channel

- **WHEN** a caller writes to `urn:slack:channel:random:thread:x`
- **THEN** it is refused as `no-channel`, and nothing is posted
