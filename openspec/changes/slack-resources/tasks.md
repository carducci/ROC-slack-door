# Tasks

## 1. Retire the sample

- [ ] 1.1 Remove the lending specs, the two sample changes, their vectors and the `service` and `cli` artifacts; keep the spec-and-vector agreement test, with an empty vectors file
- [ ] 1.2 Rewrite README.md and the Cargo workspace for `ikigai-slack` (licence `MIT OR Apache-2.0`), with `artifacts/slack` and `artifacts/door` as members

## 2. Library (`slack`)

- [ ] 2.1 The `SlackTransport` trait and a fake that records posts and reactions and can be fed events, so every scenario runs without a network
- [ ] 2.2 Thread keys and their refusal (A thread is named by a key the caller chooses)
- [ ] 2.3 Open a thread on the first write, with the key-to-thread map in the state directory (Writing to a new key opens the thread)
- [ ] 2.4 Reply in an open thread (Writing to an open thread replies in it)
- [ ] 2.5 Refuse empty and over-long text (A message has text within Slack's limit)
- [ ] 2.6 Read a thread as JSON with no Slack ids (Reading a thread returns the conversation)
- [ ] 2.7 Exists and `no-thread` (An unknown key is not a thread)
- [ ] 2.8 Add and remove reactions (A reaction marks a thread's opening message)
- [ ] 2.9 Refuse reactions without a thread or with a bad name (A reaction needs an open thread)
- [ ] 2.10 Capabilities on every verb (Capabilities gate the verbs)
- [ ] 2.11 Map rate limits and outages to unavailable (Slack's transient failures are unavailable, not errors)
- [ ] 2.12 Descriptions (`Meta`) for every resource, with no secrets (Secrets never leave the door)

## 3. Inbox (`slack`)

- [ ] 3.1 Deliver an allowed reply to the inbox and cut the thread (An allowed person's reply lands in the inbox)
- [ ] 3.2 Filter people, bots and channels (Only allowed people, in the configured channel, are delivered)
- [ ] 3.3 Deliver messages outside any thread with no key (A message outside any thread lands without a key)
- [ ] 3.4 Deliver reactions (An allowed person's reaction lands in the inbox)
- [ ] 3.5 Deduplicate by event id across restarts (Each Slack event is delivered at most once)

## 4. Door (`door`)

- [ ] 4.1 Configuration from a file and the environment, refusing a start with anything missing (The host configures the door)
- [ ] 4.2 The Slack Web API transport over HTTPS, behind `SlackTransport`
- [ ] 4.3 The Socket Mode connection: connect, acknowledge, reconnect with backoff, feed events to the inbox
- [ ] 4.4 Serve the space over a Unix socket or QUIC, so another kernel mounts it
- [ ] 4.5 A runbook in the README: creating the Slack app from a manifest (scopes, events, Socket Mode), the two tokens, a private channel, a systemd user unit, and mounting the door from another kernel, with placeholders only

## 5. Vectors and archive

- [ ] 5.1 Add a vector to openspec/vectors.json for every scenario, run by the library's tests against the fake
- [ ] 5.2 Archive the change into `openspec/specs/`
