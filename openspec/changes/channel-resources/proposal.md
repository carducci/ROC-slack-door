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

## Resource map

The resource-first pass for this change, in the form ikigraph's `docs/resource-design.md`
sets for a `roc` repository. Names first, code last: the door holds almost no state of its
own, because Slack is the record. The door's atoms are the thread map, each channel's inbox
space and the delivered-event ids; everything else is a name composed over them and over
Slack. Written after the change's tasks 1 to 3 merged, from `design.md`, the spec deltas and
the code as built. Where the code does not yet do what a spec delta says, the line says so
and names the task that makes it true, or says that no task does yet.

| set | name | kind | how |
|---|---|---|---|
| the declared channels | `SLACK_CHANNELS`, with `SLACK_CHANNEL_{NAME}_ID` and `…_SPEAKERS` | data | the host's `door.env` (D5). The list is the only authority on which channels exist; no code names one (D1) |
| channel | `urn:slack:channel:{name}` | corridor, and a Sink edge | the scope every name below sits in: its Slack id, its speakers, its inbox and its capabilities. Sinking text posts a top-level message that belongs to no thread (D2). It has no Source |
| every other name under a channel | `urn:slack:channel:{rest}` | code: the catch-all handler (`ChannelEndpoint`) | bound after the thread, reaction and inbox templates, so it answers whatever they do not. It takes the channel's name as the first segment of `rest`, refuses an undeclared one as `no-channel` on every verb before Slack is called, serves the channel's Sink and the reaction's Sink and Delete, refuses `…:thread:` with an empty key as `bad-key`, and answers NotFound "not served yet" for any other name. See the kept-code line |
| thread map line | `key TAB channel TAB ts` in `threads.tsv` | ATOM, internal | the key's channel and opening message, the door's only state about a thread. Not a name of its own: reached only through the thread |
| thread | `urn:slack:channel:{name}:thread:{key}` | code: a handler (`ChannelThreadEndpoint`), with Source, Sink and Exists | Source reads Slack's replies to the map line's opening message through the transport, as JSON that carries each author's Slack user id (a bot's `bot_id` for a bot message) and UTC times, and no channel id or message timestamp. Sink posts a reply, or opens the thread when the key has no line (D2). Exists answers from the map. Not a tiny function: see the kept-code line |
| minted key | `m-<seconds>-<micros>` | mapping rule | a speaker's top-level message becomes a thread under a key computed from its Slack timestamp (D3); the only state is the map line it writes, through `Threads::adopt`, from the inbox filter |
| reaction | `…:thread:{key}:reaction:{reaction}` | Sink and Delete edges | add or remove the reaction on the thread's opening message, found through the map line. Nothing is stored; Slack holds the reaction. No Source, and no body: served by the catch-all |
| inbox | `urn:slack:channel:{name}:inbox` | code: a handler (`InboxEndpoint`) over an ATOM | rewrites the request onto the intray space `urn:space:{name}`, which holds the channel's undelivered tuples, after checking the channel is declared. As built it passes the caller's capability through to the space; task 4.1 makes Source and Delete require `urn:cap:slack:channel:{name}:read` at this name (specs/inbox, "Reading an inbox needs the channel's capability"). Not an alias rule: see the kept-code line |
| inbox tuple | a `slack:Message` or `slack:Reaction` blank node | code: string building (`turtle`) | one Turtle tuple per delivery, built in code from the event and the key the map gives it, with the author's Slack user id as `slack:user`; a host matches it with SPARQL. No template resource exists; see Open |
| delivered events | `delivered.txt` in the state directory | ATOM, internal | the ids of events already delivered, so a redelivery lands once across restarts. Not a name |
| capability | `urn:cap:slack:channel:{name}:read`, `…:post`, `…:react` | names, not resources | exact strings the door will check per request, before Slack is called (D4). Pending task 4.1: as built the door checks the three global strings `urn:cap:slack:read`, `…:post` and `…:react` on every channel |
| old threads (D6) | a line's channel id, matched to a declared channel's name on each request | mapping rule | an old line is served under the channel name its id now has: `threads.tsv` is parsed at start keyed by channel id, and `Threads::id_of` turns the name a request carries into its id against the configuration, per request. Nothing is rewritten, no code runs at start beyond parsing, and a line for an undeclared channel is kept and served by no name |

**Corridors.** A channel is the corridor: `urn:slack:channel:{name}` binds, from the host's
configuration, the Slack id requests go to, the speakers the inbox admits, the intray space
deliveries land in and the capability strings that gate the verbs. Nothing threads a channel
argument through the endpoints; the name carries it. A client's mount
(`urn:sd:` from Dax's kernel, see Impact) is a corridor over the whole space that the door
never sees. There is no temporal space: Slack is the record. The inbox spec requires a
thread's resource to be cut when a speaker's message lands in it, so a cached read is never
stale; nothing under `artifacts/` makes that cut yet, and no task names it. Reads are fresh
today only because nothing caches them. There is no personal space: what a caller may see
is its capability set, not a view.

**Sealed.** The old `urn:slack:thread:{key}`, `urn:slack:thread:{key}:reaction:{name}` and
`urn:cap:slack:{read,post,react}` are to be answered by nothing, with no alias (see
Variances). As built, `serve.rs` still binds both old names and checks the global
capabilities. Task 4.1 removes the global capabilities; no task yet removes the old thread
and reaction bindings or writes the fitness function `design.md` names, which will keep them
out of `artifacts/` once it exists. Slack's channel ids and message timestamps never appear
in a name or a representation: a channel is named by the host's name, a thread by its key,
and a time is UTC. Slack's user ids are carried: the conversation's JSON and the inbox
tuple's `slack:user` both give the author's Slack user id, and a bot message's `bot_id` in
its place, because that id is how a caller tells speakers apart. Any name under a channel
the host did not declare is refused as `no-channel` before Slack is called.

**Faces.** A thread sources as `application/json`, one object per message. The inbox's
tuples are Turtle, the intray space's own face. The channel's and the thread's Sinks take
`text/plain`, as `content` or as `in` when piped. The reaction's Sink takes no body: text
sent with it is ignored. The inbox's Sink takes a Turtle tuple, forwarded to the space.
There are no templates and no views: the door renders nothing for a person, and Slack is
the only UI.

**Errors.** `bad-key` (an empty key included), `bad-reaction` and an empty or over-long
text are InvalidArgument; a Sink to the channel or a thread with no text is
MissingArgument(`content`); `no-thread`, `no-channel` and any other name under a declared
channel ("not served yet") are NotFound; a missing capability is Denied; Slack throttling or
being unreachable is Unavailable, with Slack's retry-after when it gives one. Endpoint
carries the rest: a rejection from Slack such as `not_in_channel` or a bad token, a failure
to save the map or a timestamp from Slack of a shape the map cannot hold, a verb a name does
not take, and a poisoned map lock.

**Code the change keeps**, one line each on why it cannot be a composition, or saying
plainly that it could and is not yet:

- The Slack transport (`SlackTransport`: post, react, replies, events) is the edge to a
  system outside the kernel, over the Web API and Socket Mode. No kernel verb names Slack's
  API, and the fake transport is what lets every scenario run with no network.
- The thread's Source is code because it reads no resource: the replies come from the
  transport, which is the edge, and the kernel serves no name for them. A tiny function has
  nothing sourced to compose over until the transport's replies are a resource, which this
  change does not make them.
- Opening a thread is a store with a side effect: Slack returns the opening message's
  timestamp only after the post, and the map line is written from it. A composition has no
  value to compose until Slack has answered.
- The inbox filter (`Inbox::deliver_to`) is code because its input is not a resource: Slack
  pushes events over Socket Mode. Over an event it is a pure decision (a speaker, in a
  declared channel, not the bot, not yet delivered). It writes two atoms: the delivered-ids
  atom for every delivery, and the map line (through `Threads::adopt`) for a speaker's
  top-level message, which is where the minted key is stored.
- The inbox name is a handler, not an alias rule, although the kernel offers exact aliases
  as data (`AliasTable`, `Kernel::with_aliases`): an alias passes the caller's capability
  through to the space, and the inbox spec requires the check for
  `urn:cap:slack:channel:{name}:read` at the inbox's own name, which the handler is the
  place for (task 4.1). The handler also carries `in` into `content` for a piped Sink.
- The capability check is code in the door because the scope is per channel and the kernel's
  check is per space. Pending task 4.1: it will reduce the caller's capability to the three
  strings for the named channel, for the inbox's Source and Delete as for the Slack verbs;
  as built it reduces to the three global strings. Whether the hub can grant a templated
  family is not this change's (D4).
- The catch-all (`ChannelEndpoint`, `urn:slack:channel:{rest}`) is code because a URI
  template has no notion of a declared set of values; the set itself is data, from the
  configuration. One template has to answer every name under a channel the narrower
  templates do not bind, so the channel check (`no-channel`) and the "not served yet"
  refusal live there, and it splits `rest` on `:` in code to find the channel's name.

**Open.** Compositions the design does not decide, left open rather than invented:

- Whether `urn:space:{name}`, the intray space behind a channel's inbox, stays reachable
  beside `urn:slack:channel:{name}:inbox` or is sealed. The kernel as built serves both.
- Whether a channel's threads become a name (a list of a channel's keys). In this change a
  thread is reached only by its key, and the map line is internal.
- Whether the inbox tuple becomes a template fill. It is built by `turtle` in code; a
  template resource and a fill over the delivery would do the same with no code, and none is
  served yet.

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
