# Tasks

Waits for `channel-resources`: these names sit under `urn:slack:channel:{name}`.

## 1. Hearing files (`door`, `slack`)

- [ ] 1.1 Hear `file_share` messages, still dropping every other subtype, and carry each file's id, reported size and private URL into the inbox decision; add a recorded `file_share` event as a test fixture (A speaker's post with files lands in the inbox)
- [ ] 1.2 Parse a channel's accepted types and size limit, refusing a start with a list but no limit or with a type `ikigai-sniff` can't detect (A channel lists the files it accepts)

## 2. The object store (`slack`)

- [ ] 2.1 Store bytes at `objects/xx/yy/<rest>` by `ContentId::of`, write-once through a temp name and rename, jailed by `ikigai-fs` (A file is kept by its content)
- [ ] 2.2 Per-channel index (`files/{channel}.tsv`), and deletion that removes the object only when no channel names it (Deleting a file removes its name)

## 3. Intake and serving (`slack`, `door`)

- [ ] 3.1 Download with the bot token, capped at the limit plus one byte; sniff; keep or refuse with `file-type-refused`, `file-too-large`, `files-not-accepted` or `file-unavailable`; write the tuple with each file's name or refusal (A file's type comes from its bytes)
- [ ] 3.2 Serve `urn:slack:channel:{name}:file:b3:{hex}`: bytes with the indexed type, cacheable, `no-file` and `bad-file` refusals, with descriptions (`Meta`) (Reading a file returns its bytes)
- [ ] 3.3 Require `…:file:read` and `…:file:delete`, separate from the other verbs (File capabilities are separate)

## 4. Vectors, docs and deploy

- [ ] 4.1 Vectors for every scenario, the type-spoofing and integrity tests from the design, and the README runbook: `files:read` in the app manifest, a channel's file settings in `door.env`
- [ ] 4.2 Human only, Michael as Slack admin and on Tamariz, after `channel-resources` 5.2: add `files:read` to the Slack app and reinstall; create the private channel `#fixme`, invite the bot and David; declare `fixme` in `door.env` with David's Slack user id as its only speaker, images only, 10 MB; restart `roc-slack-door`; post one test screenshot and read it back
- [ ] 4.3 Archive the change into `openspec/specs/`
