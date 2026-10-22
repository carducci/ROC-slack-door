# Files in a channel

## Why

A screenshot is the clearest way to report a problem in a UI. Today the door ignores any
message with a subtype, and a message carrying an uploaded file is one (`file_share`), so a
post with a screenshot never reaches the inbox at all. The `#fixme` channel (`mago#199`), where
David reports problems in Mago with a screenshot and a line, needs those posts to land, and it
needs the image somewhere an agent can read it without signing in to Slack.

Planned with Michael in a Pax session, 2026-10-21 (pax-explore on `mago#199`). Builds on
`channel-resources`.

## What Changes

- **A channel can accept files.** The host lists the media types a channel accepts and a size
  limit. A channel that lists none takes no files, as today.
- **A speaker's post with files lands.** The door hears `file_share` messages (edits and
  deletions are still ignored). It downloads each file, keeps it, and writes one inbox tuple
  carrying the text and each file's name, or why a file was refused.
- **Files are kept by their content.** The bytes are stored once, under their BLAKE3 content
  address, in git-style fan-out directories (`objects/9f/2c/…`), through `ikigai-fs`'s jail.
  They're written once and never changed, and the same image posted twice is one object.
- **The type comes from the bytes, never from Slack.** `ikigai-sniff` decides what a file is.
  Slack's filename and claimed type are ignored, and anything the channel doesn't accept is
  refused.
- **Files are resources under their channel.** `urn:slack:channel:{name}:file:b3:{hex}` returns
  the bytes with their sniffed type, and deleting it removes the channel's name for them. The
  capabilities are `…:file:read` and `…:file:delete`, so a caller can be allowed to look at
  screenshots and nothing else.

## Impact

- Artifacts: `slack`, `door`
- New spec: `files`. Modified: `configuration` (a channel's file settings).
- New dependencies: `ikigai-fs`, `ikigai-sniff` and `blake3` (through `ikigai-core`'s
  `ContentId`).
- **The Slack app needs `files:read`.** Michael adds the scope and reinstalls the app. The
  tokens are expected to stay the same (inferred; checked at the reinstall).
- **Disk on Tamariz.** Screenshots under the door's state directory. A retention chore, outside
  this repository, deletes them 30 days after the item that names them closes.
- **Personal data.** Screenshots from `#fixme` use test data or are redacted by the reporter
  (Michael, 2026-10-21). The door doesn't inspect content.

## Variances

None.

## Out of scope

- Any file type but images in `#fixme` (that's host configuration, so it can widen later).
- Serving files over gonk to workers on other machines. Readers mount the door, as Dax does
  today.
- Retention: a chore elsewhere decides when to delete, and calls this delete.
- Thumbnails, resizing, or reading text out of images.
