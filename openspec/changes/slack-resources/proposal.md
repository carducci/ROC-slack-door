# Slack as ikigai resources

## Why

A program that wants to hold a conversation with a person in Slack should not have to speak
Slack. It should name the conversation, write to it, read it back, and find the person's replies
waiting for it, through the same five verbs it uses for everything else. Slack's channel ids,
message timestamps, Socket Mode envelopes, retries and rate limits stay behind the names, so a
caller written against these resources can move to another chat system by swapping the door,
not by rewriting itself.

This repository started as the starter kit's library-lending sample. That sample is retired by
this change.

## What Changes

- **Threads named by the caller.** `urn:slack:thread:{key}` is a conversation the caller names
  with a key of its own choosing. Writing to a thread that does not exist yet opens it as a new
  message in the configured channel; writing to one that does replies in it. Reading it returns
  the conversation. The door keeps the key-to-thread mapping, durably; callers never see a
  Slack id.
- **Reactions.** `urn:slack:thread:{key}:reaction:{name}` adds (sink) or removes (delete) a
  reaction on a thread's opening message, so a caller can mark a conversation, e.g. done.
- **An inbox.** Messages and reactions from allowed people, in the configured channel, land as
  tuples in an ikigai space (`ikigai-intray`), each carrying the thread's key when it belongs
  to a thread. A host decides what reacts to them.
- **Configuration from the host, never the repository.** Tokens, the channel, the allowed
  people, the state directory and the inbox's name are given by whoever runs the door, the way
  `ikigai-email` is given its mail server. Nothing in this repository names a workspace, a
  channel or a person.
- **Capabilities** `urn:cap:slack:read`, `urn:cap:slack:post` and `urn:cap:slack:react` gate the
  verbs, so a host can let one caller read and another post.
- **Retire the sample**: its specs, vectors, artifacts and CLI go, in the first tasks.

## Impact

- Artifacts: `slack`, `door`
- `slack` is the library crate, `ikigai-slack`: the space, its endpoints and descriptions, over
  a `SlackTransport` trait so every scenario runs against a fake with no network.
- `door` is the binary: it holds the Socket Mode connection, reads the host's configuration,
  writes the inbox and serves the space over a socket or QUIC for another kernel to mount.
- Affected specs: `threads`, `reactions`, `inbox`, `configuration` (all new).
- Retired: `catalog`, `loans`, `members`, `overdue`, `renewals`, and the two changes in flight
  (`add-holds`, `cap-late-fees`).
