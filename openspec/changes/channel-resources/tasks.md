# Tasks

## 1. Configuration (`door`, `slack`)

- [x] 1.1 Parse `SLACK_CHANNELS` and each channel's id and speakers, refusing a start with none, a bad or repeated name, a missing id or no speakers, and refusing the retired `SLACK_CHANNEL`, `SLACK_ALLOWED_USERS` and `ROC_INBOX_SPACE` by name (The host declares the channels)
- [x] 1.2 Refuse an undeclared channel name as `no-channel` on every resource, without calling Slack (The host declares the channels)

## 2. Threads and reactions (`slack`)

- [x] 2.1 Serve threads as `urn:slack:channel:{name}:thread:{key}`, with keys unique per channel in the existing map (A thread is named by a key the caller chooses)
- [x] 2.2 Open, reply, read and refuse unknown keys per channel (Writing to a new key opens the thread)
- [x] 2.3 Post a top-level message on a write to `urn:slack:channel:{name}` (Writing to a channel posts a new message)
- [x] 2.4 Serve reactions as `urn:slack:channel:{name}:thread:{key}:reaction:{reaction}` (A reaction marks a thread's opening message)
- [x] 2.5 Serve existing `threads.tsv` lines under the declared channel whose id they carry (Threads opened before channels keep their keys)

## 3. Inbox (`slack`, `door`)

- [x] 3.1 One inbox per channel at `urn:slack:channel:{name}:inbox`, filtered by that channel's speakers (Only allowed people, in the configured channel, are delivered)
- [x] 3.2 Mint an `m-` key for a speaker's top-level message and record it as a thread (A speaker's top-level message opens a thread)

## 4. Capabilities (`slack`)

- [ ] 4.1 Require `urn:cap:slack:channel:{name}:{read,post,react}` on every verb, including the inbox; remove the global capabilities `urn:cap:slack:{read,post,react}` (`artifacts/slack/src/lib.rs`) and the old bindings `urn:slack:thread:{key}` and `urn:slack:thread:{key}:reaction:{name}` (`artifacts/door/src/serve.rs`), with their `Meta` entries and tests, and in the same pull request every reader of them, so CI is green at its head: add the `Meta` entry for `urn:slack:channel:{name}:thread:{key}` (this task's, not 5.1's), point the `describe` vector of "Describing a thread" in `openspec/vectors.json` at it, grant the vector runner's `as` capabilities per channel (`artifacts/slack/tests/vectors.rs`), and rewrite README.md's capabilities paragraph for the scoped names and its line saying the older `urn:slack:thread:{key}` names are still bound; and write design.md's second fitness function as a test in `tests/`, run by `python3 scripts/test.py`, that fails if any file under `artifacts/` contains `urn:slack:thread:` or one of `urn:cap:slack:read`, `urn:cap:slack:post`, `urn:cap:slack:react` (Capabilities gate the verbs)

## 5. Vectors, docs and deploy

- [ ] 5.1 Update `openspec/vectors.json` for every changed and new scenario (the `describe` vector of "Describing a thread" is already 4.1's), the README runbook (two channels, the new `door.env` keys, mounting `urn:slack:channel:` for a thread, a reaction and the inbox), and descriptions (`Meta`) for every new resource 4.1 did not add: `urn:slack:channel:{name}`, `urn:slack:channel:{name}:thread:{key}:reaction:{reaction}` and `urn:slack:channel:{name}:inbox`
- [ ] 5.2 Human only, Michael on Tamariz, after this change's PRs merge: in one sitting, rewrite `~/.config/ikigai-slack/door.env` for `gonk-chat`, restart `roc-slack-door`, and switch Dax's kickoff (`~/gonk-da/.gonk/dax-kickoff.md`) to `urn:sd:slack:channel:gonk-chat:thread:<key>` and `urn:sd:slack:channel:gonk-chat:inbox`; check that one thread reply round-trips
- [ ] 5.3 Archive the change into `openspec/specs/`
