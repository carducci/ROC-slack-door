# Design: files in a channel

## Context

`artifacts/door/src/socket.rs` drops every `message` event that has a `subtype`, so that edits
and deletions aren't heard. A post with an upload arrives as `subtype: "file_share"` with a
`files` array: each entry has an id, size, a claimed `mimetype` and a `url_private` that needs
the bot token and the `files:read` scope. This is from Slack's Events API documentation, to be
confirmed against a recorded event in task 1.1.

ikigai already has the parts (crate sources read on Tamariz, 2026-10-21):

- `ikigai-core` 0.1.108 `ContentId`: a BLAKE3 digest rendered as `b3:<hex>`, with
  `ContentId::of(bytes)`. Every `Representation` is a type and raw bytes.
- `ikigai-fs` 0.1.9: a byte store jailed to a root, default-deny path ACLs. Paths resolve
  through `openat` on directory handles (Unix only; elsewhere it refuses every request).
  `Source … as=application/octet-stream` returns raw bytes, and it won't relabel.
- `ikigai-sniff` 0.2.6: classifies `application/octet-stream` bytes into a media type, PNG
  included, from the bytes alone.

There is no content-addressed object store crate on crates.io (`cargo search ikigai`,
2026-10-21), so this change composes one inside the door.

## Decisions

### D1. Content-addressed, git-style storage

An object lives at `objects/9f/2c/<remaining 60 hex>` under the state directory: two levels
of fan-out, as git does, so no directory grows past a few thousand entries. The address is
`ContentId::of(bytes)` of the raw bytes. It isn't `Representation::content_id`, which mixes in
the type, because the name must not depend on the sniffed type. Writing is to a temporary
name in the same directory, then a rename, so a crash never leaves a partial object under a
real name. An existing object is never rewritten.

- Alternative rejected: a git repository (`git hash-object`). That needs `ikigai-repo`'s exec
  seam and a git binary for what is one hash and two directory levels, and git's object
  format adds a header, so the name is no longer the plain content hash.
- Alternative rejected: Azure Blob or another hosted store. It adds a sub-processor for
  screenshots while the Mago processor terms are open.

### D2. Names per channel, bytes shared

Each channel keeps an index of the addresses it has received (`files/{channel}.tsv`: address,
sniffed type, size, first-seen time). A read checks the channel's index, then the object. A
delete removes the index line, then removes the object if no other channel's index names it.
Access follows the channel the file arrived in, while the bytes are stored once.

### D3. The bytes decide the type

The claimed `mimetype` and the filename are never stored or used. `ikigai-sniff` runs on the
downloaded bytes before anything is written, and only a type on the channel's list is kept.
The type is recorded in the index, so reads don't sniff again. This is the safety property:
the door stores opaque bytes and serves them only as one of a few image types it confirmed
itself.

### D4. Download in the event path, with the size checked twice

Slack's reported size is checked before downloading. The download is then capped at the limit
plus one byte, so a lie about the size can't fill the disk. A download that fails is noted as
`file-unavailable` on the tuple, and the message still lands. There is no retry queue: the
reporter can post again. The download happens before the event is acknowledged as delivered,
so a crash in the middle means Slack redelivers, and dedup by event id still holds.

### D5. Capabilities

`urn:cap:slack:channel:{name}:file:read` and `…:file:delete` stand apart from `read`, `post`
and `react`. The triage worker gets only `file:read`, and the retention chore gets only
`file:delete`.

## Fitness functions

- The spec-and-vector agreement test covers every scenario here.
- A test feeds a `file_share` event whose `mimetype` says `image/png` but whose bytes are an
  ELF or PE header, and asserts nothing is stored. That's the sensor for D3, and it fails if
  someone starts trusting Slack's type.
- A test reads every object under `objects/` and checks its BLAKE3 equals its path (an
  integrity check, also usable as a chore later).

## Open

- **The `files:read` scope and the reinstall are Michael's** (task 4.2, human-only). Whether the
  bot token survives a scope reinstall is to be checked then (inferred yes).
- **The `#fixme` channel and David's Slack user id** are host configuration (task 4.2).
- **Retention** is a chore outside this repo. It reads closed items on the ledgers and deletes
  file names 30 days after the item closes (Michael, 2026-10-21). It needs only `file:delete`.
