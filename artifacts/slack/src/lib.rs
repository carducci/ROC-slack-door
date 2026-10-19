//! Slack conversations as resources for ikigai.
//!
//! A thread is `urn:slack:thread:{key}`, named by a key its caller chooses; a reaction on it is
//! `urn:slack:thread:{key}:reaction:{name}`; replies from allowed people land in an inbox space.
//! Everything Slack-specific sits behind a `SlackTransport`, so every scenario in
//! `openspec/` runs against a fake with no network. What each resource does is specified in
//! `openspec/changes/slack-resources/` until that change is archived into `openspec/specs/`.
//!
//! Nothing is implemented yet: this crate is the skeleton slack-resources 2.x fills in.
