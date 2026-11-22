## MODIFIED Requirements

### Requirement: A reaction marks a thread's opening message

Sinking to `urn:slack:channel:{name}:thread:{key}:reaction:{reaction}` SHALL add the reaction
(a Slack emoji name of lowercase letters, digits, `_`, `-` and `+`) to the opening message of
the key's thread in that channel, and deleting it SHALL remove it. Adding a reaction already
there, or removing one that is not, SHALL succeed and change nothing.

#### Scenario: Marking a thread done

- **WHEN** a caller writes to `urn:slack:channel:gonk-chat:thread:carducci-54:reaction:white_check_mark`
- **THEN** the thread's opening message carries that reaction

#### Scenario: Adding a reaction twice

- **WHEN** a caller adds `white_check_mark` to a thread in `gonk-chat` that already carries it
- **THEN** it succeeds, and the message carries the reaction once

### Requirement: A reaction needs an open thread

A reaction on a key with no thread in that channel MUST be refused as `no-thread`, and a name
outside the emoji-name characters as `bad-reaction`, without calling Slack.

#### Scenario: Reacting to a key with no thread

- **WHEN** a caller writes to `urn:slack:channel:gonk-chat:thread:never-opened:reaction:white_check_mark`
- **THEN** it is refused as `no-thread`
