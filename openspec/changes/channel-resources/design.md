# Design: channels as resources

## Context

`slack-resources` gave the door one channel (`SLACK_CHANNEL`), one allowed list
(`SLACK_ALLOWED_USERS`) and one inbox space (`ROC_INBOX_SPACE`), with global capabilities
`urn:cap:slack:{read,post,react}`. The thread map (`threads.tsv`) already stores the channel of
each thread (`key TAB channel TAB ts`), and `Inbox::may_speak` checks one channel and one set.

## Decisions

### D1. A channel is a resource, and the host's configuration is the list

Every name sits under `urn:slack:channel:{name}`, where `{name}` is the host's name for the
channel, not Slack's id. The door serves exactly the channels the host declares, and no code
names one.

- Alternative rejected: one door per channel. Two Socket Mode connections on one Slack app
  split events between them (Slack sends each event to one open connection; from Slack's
  Socket Mode documentation, inferred to apply here and not tested), so each door would miss
  events meant for the other, or a second Slack app would be needed per channel.
- Alternative rejected: one inbox for every channel, with a channel field. It's simpler for
  today's one client. But it puts every channel behind one capability, so a caller that may
  read `fixme` could read `gonk-chat`.

### D2. The thread resource is the outbox

Writing to a known key replies in its thread, and writing to a new key opens one. That is
already what a caller needs. Michael asked about an outbox that posts to a thread when one is
known and to the channel otherwise. That would be routing logic hidden behind one name, so
instead the caller names what it means: `…:thread:{key}` when it has a key, and
`urn:slack:channel:{name}` when it doesn't.

### D3. A speaker's top-level message opens a thread with a minted key

To answer a person under their own message, the caller needs a key for it. The door mints
`m-<seconds>-<micros>` from the message's timestamp and records it in the thread map, as if
the door had opened it. Minted keys can't collide with a caller's keys unless a caller chooses
`m-` keys, so callers SHOULD NOT. This is documented, not refused.

- Alternative rejected: an explicit "adopt this message as thread X" write. It needs an opaque
  message handle in the inbox tuple and one more verb, for the same result.

### D4. Capabilities are scoped by channel

`urn:cap:slack:channel:{name}:{read,post,react}`. The inbox is read with `…:read`. The door
checks the capability for the channel the request names, before Slack is called. Whether the
hub can grant a templated family or needs one string per channel doesn't matter here: the
door checks exact strings.

### D5. Configuration stays a key-value file

The host's `door.env` keeps its format, so systemd's `EnvironmentFile=` and the door's own
reader stay as they are:

```
SLACK_CHANNELS=gonk-chat,fixme
SLACK_CHANNEL_GONK_CHAT_ID=C…
SLACK_CHANNEL_GONK_CHAT_SPEAKERS=U…
SLACK_CHANNEL_FIXME_ID=C…
SLACK_CHANNEL_FIXME_SPEAKERS=U…
```

A channel's keys are its name uppercased, with `-` changed to `_`. `SLACK_CHANNEL`,
`SLACK_ALLOWED_USERS` and `ROC_INBOX_SPACE` are refused at start with a message naming their
replacement, so a stale `door.env` fails loudly instead of serving nothing.

- Alternative rejected: a TOML file for channels. It's nicer to read, but it's a second format
  beside the tokens' env file, for the two channels there are.

### D6. Upgrade in place

On start, any line in `threads.tsv` whose channel id matches a declared channel is served
under that channel's name. The file's format doesn't change. Lines for undeclared channels are
kept.

## Fitness functions

- The spec-and-vector agreement test (already in CI) fails if any scenario here has no vector.
- Two tests, written by task 4.1, so the old names can't come back: one over the source and
  one over what is served, because the first alone is a denylist of bytes that a string escape
  (`"urn:slack:\x74hread:{key}"`), a `format!` of the pieces, a template with a wider variable
  (`urn:slack:{space}:{key}`) or an endpoint name with no trailing colon gets past while the
  program still serves the same names.
  - In `tests/`, run by `python3 scripts/test.py`: fails if any file under `artifacts/`
    contains `urn:slack:thread:` (the old thread and reaction names, bound without a channel)
    or one of the global capabilities `urn:cap:slack:read`, `urn:cap:slack:post`,
    `urn:cap:slack:react`. The scoped `urn:cap:slack:channel:{name}:read` has `channel:`
    between and never matches.
  - In the door's and the library's own `cargo test`, also run by `python3 scripts/test.py`,
    through the kernel: `Exists` and `Sink` on `urn:slack:thread:k` and on
    `urn:slack:thread:k:reaction:x` answer that nothing is bound there (not a thread's
    `no-thread` or a channel's `no-channel`, which an endpoint answers), and every capability
    in `descriptions()` starts with `urn:cap:slack:channel:`. These check what is served,
    however it is spelled.

## Open

- **Deploying is Michael's.** It needs a rewrite of `door.env`, a restart of `roc-slack-door`,
  and Dax's kickoff switched to the new names at the same time (task 5.2, human-only).
- **Dax's grant for the door** has to hold `urn:cap:slack:channel:gonk-chat:{read,post,react}`
  wherever the door's capability set is configured for Dax's mount. It isn't read here, so the
  deploy checks it.
