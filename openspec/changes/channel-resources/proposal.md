# Channels as resources

## Why

The door serves one channel, named in its configuration, with one list of allowed people and
one inbox. A second conversation with a different audience, such as a `#fixme` channel where
only David reports problems in Mago (`mago#199`), would need a second door or code that knows
about it. In resource-oriented terms a channel is just a resource: the host should declare its
channels, and the door should serve the same family of names for each one, with who may speak
and what each caller may do set per channel.

Planned with Michael in a Pax session, 2026-10-21 (pax-explore on `mago#199`; research
`Research/Mago fixme button` in Michael's graph).

## What Changes

- **The host declares the channels.** The configuration names each channel the door serves,
  with its Slack id and the Slack users who may speak there. No channel is built in, and the
  single `SLACK_CHANNEL`, `SLACK_ALLOWED_USERS` and `ROC_INBOX_SPACE` settings go.
- **Every name sits under its channel**:
  - `urn:slack:channel:{name}` takes a write as a new top-level message.
  - `urn:slack:channel:{name}:thread:{key}` is a thread, as `urn:slack:thread:{key}` is today.
  - `urn:slack:channel:{name}:thread:{key}:reaction:{reaction}` is a reaction on it.
  - `urn:slack:channel:{name}:inbox` is what that channel's speakers said.
- **A speaker's top-level message opens a thread.** The door mints its key, and the inbox tuple
  carries that key, so a caller can reply under the person's own message by writing to the
  thread. Today such a message lands with no key and can't be answered in place.
- **Capabilities are per channel**: `urn:cap:slack:channel:{name}:read`, `…:post` and `…:react`
  replace `urn:cap:slack:read`, `…:post` and `…:react`.
- **Threads opened before the change keep working** under the channel they were opened in.
- **Breaking.** The old names and capabilities are removed, with no aliases (see Variances).

## Impact

- Artifacts: `slack`, `door`
- Affected specs: `configuration`, `threads`, `reactions`, `inbox`.
- **Dax is the one client.** Its kickoff names `urn:sd:slack:thread:<key>` and
  `urn:sd:space:dax-inbox` (`~/gonk-da/.gonk/dax-kickoff.md` on Tamariz). They change to
  `urn:sd:slack:channel:gonk-chat:thread:<key>` and `urn:sd:slack:channel:gonk-chat:inbox` at
  the moment the new door is deployed. Task 5.2 does both together.
- **The host's `door.env` is rewritten** for the channel keys, by Michael, at deploy.
- Followed by `channel-files` (file intake for a channel), which builds on these names.

## Variances

- **A breaking rename with no aliases.** Michael, 2026-10-21: move fast. Dax is the only
  client, aliases tend to stay forever, and a separate resolution space mapping the old names
  could be added later if a second client ever needs them. Scope: this change only.

## Out of scope

- Files in a channel (`channel-files`).
- Public channels, direct messages, and channels the door creates itself. The host creates each
  channel and invites the bot.
- Anything about `#fixme` itself: its channel and its speakers are host configuration.
