//! Slack conversations as resources for ikigai.
//!
//! A thread is `urn:slack:channel:{name}:thread:{key}`, named by a key its caller chooses; a
//! reaction on it is `urn:slack:channel:{name}:thread:{key}:reaction:{reaction}`; replies and
//! new messages from a channel's speakers land in `urn:slack:channel:{name}:inbox`, a new message
//! under a key the door mints.
//! Everything Slack-specific sits behind a `SlackTransport`, so every scenario in
//! `openspec/` runs against a fake with no network. What each resource does is specified in
//! `openspec/changes/channel-resources/specs/` while that change is in flight, and in
//! `openspec/specs/` once it is archived (task 5.3 of the change).
//!
//! The transport, its in-memory fake, the thread and reaction resources and the inbox are in
//! place.

use std::collections::{BTreeSet, VecDeque};

use ikigai_core::Capability;

mod conversation;
mod inbox;
mod threads;

pub use conversation::{to_json, Entry};
pub use inbox::{Delivery, Inbox};
pub use threads::{is_slack_channel_id, Thread, ThreadError, Threads};

/// What the door asks of Slack. The real implementation speaks the Web API and Socket Mode;
/// the fake in this crate records what it is asked and answers from memory.
pub trait SlackTransport {
    /// Posts `text` in `channel`, as a reply in `thread` when one is given. Returns the new
    /// message's timestamp, which is Slack's id for it.
    fn post(&mut self, channel: &str, thread: Option<&str>, text: &str)
        -> Result<String, TransportError>;

    /// Adds a reaction to `message`, or removes it when `add` is false. Doing what is already
    /// done succeeds and changes nothing.
    fn react(
        &mut self,
        channel: &str,
        message: &str,
        name: &str,
        add: bool,
    ) -> Result<(), TransportError>;

    /// The messages of the thread that opens at `root`: the opening message first, then the
    /// replies, in the order Slack holds them.
    fn replies(&mut self, channel: &str, root: &str) -> Result<Vec<Message>, TransportError>;

    /// The events Slack has delivered since the last call, oldest first.
    fn take_events(&mut self) -> Vec<Event>;
}

/// Why Slack would not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// Slack is throttling the door; try again after this many seconds.
    RateLimited { retry_after_secs: u64 },
    /// Slack cannot be reached or failed for its own reasons.
    Unavailable(String),
    /// Slack refused the request, with the error it gave.
    Rejected(String),
}

/// A message in a channel, as Slack reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Slack's id for the message: its timestamp.
    pub ts: String,
    /// The Slack user id of the author.
    pub user: String,
    /// Whether the author is the door's own bot.
    pub bot: bool,
    pub text: String,
}

/// Something a person did in the channel that the door hears about. `id` is Slack's id for the
/// event; Slack repeats it when it redelivers the event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A message. `thread` is the timestamp of the message it replies to, if any.
    Message {
        id: String,
        channel: String,
        user: String,
        ts: String,
        thread: Option<String>,
        text: String,
    },
    /// A reaction added to the message at `message`.
    Reaction {
        id: String,
        channel: String,
        user: String,
        message: String,
        name: String,
    },
}

impl Event {
    /// Slack's id for the event.
    pub fn id(&self) -> &str {
        match self {
            Event::Message { id, .. } | Event::Reaction { id, .. } => id,
        }
    }
}

/// The longest thread key the caller may choose.
pub const MAX_KEY_LEN: usize = 80;

/// The longest text a message may carry, in characters.
pub const MAX_MESSAGE_LEN: usize = 40_000;

/// Why a resource refused a request, named as the specs name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A thread key is empty, longer than `MAX_KEY_LEN`, or has a character outside
    /// ASCII letters, digits, `.`, `_` and `-`.
    BadKey,
    /// The text is empty or only whitespace.
    EmptyMessage,
    /// The text is longer than `MAX_MESSAGE_LEN` characters.
    MessageTooLong,
    /// The key names no thread the door has opened.
    NoThread,
    /// The reaction's name is not a Slack emoji name.
    BadReaction,
    /// The caller does not hold the capability the verb needs.
    NotGranted(String),
    /// The channel name is not one the host declares.
    NoChannel(String),
}

impl Refusal {
    /// The name the specs give this refusal, as a caller sees it.
    pub fn code(&self) -> &'static str {
        match self {
            Refusal::BadKey => "bad-key",
            Refusal::EmptyMessage => "empty-message",
            Refusal::MessageTooLong => "message-too-long",
            Refusal::NoThread => "no-thread",
            Refusal::BadReaction => "bad-reaction",
            Refusal::NotGranted(_) => "not-granted",
            Refusal::NoChannel(_) => "no-channel",
        }
    }
}

/// The action that reads a thread or the inbox.
pub const READ: &str = "read";
/// The action that posts to a channel or a thread.
pub const POST: &str = "post";
/// The action that adds or removes a reaction on a thread.
pub const REACT: &str = "react";

/// The capability that lets a caller `action` (`READ`, `POST` or `REACT`) in the channel `name`:
/// `urn:cap:slack:channel:{name}:{action}`.
pub fn scope(name: &str, action: &str) -> String {
    format!("urn:cap:slack:channel:{name}:{action}")
}

/// Accepts the caller only when its capability allows `action` in the channel `name`; otherwise
/// refuses naming the scope it lacks.
pub fn require(capability: &Capability, name: &str, action: &str) -> Result<(), Refusal> {
    let needed = scope(name, action);
    if capability.allows(&needed) {
        Ok(())
    } else {
        Err(Refusal::NotGranted(needed))
    }
}

/// Accepts a channel name only when the host declares it; otherwise refuses naming it.
pub fn check_channel<'a>(declared: &[String], name: &'a str) -> Result<&'a str, Refusal> {
    if declared.iter().any(|d| d == name) {
        Ok(name)
    } else {
        Err(Refusal::NoChannel(name.to_owned()))
    }
}

/// Checks a reaction's name, a Slack emoji name, and returns it unchanged when it is valid.
pub fn check_reaction(name: &str) -> Result<&str, Refusal> {
    let valid_char =
        |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '+');
    if !name.is_empty() && name.chars().all(valid_char) {
        Ok(name)
    } else {
        Err(Refusal::BadReaction)
    }
}

/// Checks a message's text and returns it unchanged when it may be posted.
pub fn check_text(text: &str) -> Result<&str, Refusal> {
    if text.trim().is_empty() {
        Err(Refusal::EmptyMessage)
    } else if text.chars().count() > MAX_MESSAGE_LEN {
        Err(Refusal::MessageTooLong)
    } else {
        Ok(text)
    }
}

/// Checks a caller's thread key and returns it unchanged when it is valid.
pub fn check_key(key: &str) -> Result<&str, Refusal> {
    let valid_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    if (1..=MAX_KEY_LEN).contains(&key.len()) && key.chars().all(valid_char) {
        Ok(key)
    } else {
        Err(Refusal::BadKey)
    }
}

/// Whether `ts` has the shape Slack's timestamps have: digits, one dot, digits.
pub(crate) fn is_slack_ts(ts: &str) -> bool {
    ts.split_once('.').is_some_and(|(seconds, micros)| {
        [seconds, micros]
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// What a resource is, for a caller that asks. A description holds no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    /// The resource's name, with its placeholders in braces.
    pub resource: &'static str,
    pub description: &'static str,
    /// Each verb the resource answers, with the capability it needs.
    pub verbs: Vec<(&'static str, &'static str)>,
}

/// The resources the door serves, described.
pub fn descriptions() -> Vec<Meta> {
    vec![Meta {
        resource: "urn:slack:channel:{name}:thread:{key}",
        description: "A conversation in the declared Slack channel `name`, named by a key you \
            choose. Writing to a key with no thread opens one; writing to an open thread \
            replies in it; reading returns its messages in order, in UTC.",
        verbs: vec![
            ("source", "urn:cap:slack:channel:{name}:read"),
            ("exists", "urn:cap:slack:channel:{name}:read"),
            ("sink", "urn:cap:slack:channel:{name}:post"),
        ],
    }]
}

/// A message the door's bot posted, as the fake records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Post {
    pub ts: String,
    pub channel: String,
    pub thread: Option<String>,
    pub text: String,
}

/// A message in the fake's history, with the thread it is in.
#[derive(Debug, Clone)]
struct Stored {
    channel: String,
    thread: Option<String>,
    message: Message,
}

/// An in-memory `SlackTransport`: posts and reactions are recorded, events are fed by the test.
#[derive(Debug, Default)]
pub struct FakeSlack {
    history: Vec<Stored>,
    reactions: BTreeSet<(String, String)>,
    events: Vec<Event>,
    failures: VecDeque<TransportError>,
    answers: VecDeque<String>,
}

impl FakeSlack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes the next call to the transport fail with `error`, as Slack would answer it.
    pub fn fail_next(&mut self, error: TransportError) {
        self.failures.push_back(error);
    }

    /// Makes the next post answer `ts` as the new message's timestamp, whatever its shape, as a
    /// Slack that answered something other than digits, a dot, digits would.
    pub fn answer_next_post(&mut self, ts: &str) {
        self.answers.push_back(ts.to_owned());
    }

    fn next_failure(&mut self) -> Result<(), TransportError> {
        match self.failures.pop_front() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Every message the door's bot has posted so far, oldest first.
    pub fn posts(&self) -> Vec<Post> {
        self.history
            .iter()
            .filter(|stored| stored.message.bot)
            .map(|stored| Post {
                ts: stored.message.ts.clone(),
                channel: stored.channel.clone(),
                thread: stored.thread.clone(),
                text: stored.message.text.clone(),
            })
            .collect()
    }

    /// Every reaction present, as (message timestamp, reaction name).
    pub fn reactions(&self) -> &BTreeSet<(String, String)> {
        &self.reactions
    }

    /// Queues an event for the next `take_events`. A message also joins the channel's history,
    /// so `replies` sees it.
    pub fn feed(&mut self, event: Event) {
        if let Event::Message {
            channel,
            user,
            ts,
            thread,
            text,
            ..
        } = &event
        {
            self.history.push(Stored {
                channel: channel.clone(),
                thread: thread.clone(),
                message: Message {
                    ts: ts.clone(),
                    user: user.clone(),
                    bot: false,
                    text: text.clone(),
                },
            });
        }
        self.events.push(event);
    }
}

impl SlackTransport for FakeSlack {
    fn post(
        &mut self,
        channel: &str,
        thread: Option<&str>,
        text: &str,
    ) -> Result<String, TransportError> {
        self.next_failure()?;
        let ts = self
            .answers
            .pop_front()
            .unwrap_or_else(|| format!("1700000000.{:06}", self.history.len() + 1));
        self.history.push(Stored {
            channel: channel.to_owned(),
            thread: thread.map(str::to_owned),
            message: Message {
                ts: ts.clone(),
                user: "UBOT".to_owned(),
                bot: true,
                text: text.to_owned(),
            },
        });
        Ok(ts)
    }

    fn react(
        &mut self,
        _channel: &str,
        message: &str,
        name: &str,
        add: bool,
    ) -> Result<(), TransportError> {
        self.next_failure()?;
        if !self.history.iter().any(|stored| stored.message.ts == message) {
            return Err(TransportError::Rejected("message_not_found".to_owned()));
        }
        let key = (message.to_owned(), name.to_owned());
        if add {
            self.reactions.insert(key);
        } else {
            self.reactions.remove(&key);
        }
        Ok(())
    }

    fn replies(&mut self, channel: &str, root: &str) -> Result<Vec<Message>, TransportError> {
        self.next_failure()?;
        Ok(self
            .history
            .iter()
            .filter(|stored| {
                stored.channel == channel
                    && (stored.message.ts == root || stored.thread.as_deref() == Some(root))
            })
            .map(|stored| stored.message.clone())
            .collect())
    }

    fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_capability_described_is_scoped_to_a_channel() {
        let verbs: Vec<_> = descriptions().into_iter().flat_map(|meta| meta.verbs).collect();
        assert!(!verbs.is_empty());
        for (verb, capability) in verbs {
            assert!(capability.starts_with("urn:cap:slack:channel:"), "{verb}: {capability}");
        }
    }

    #[test]
    fn a_scope_for_one_channel_and_action_admits_only_that() {
        let held = Capability::scoped([scope("gonk-chat", READ)]);
        assert_eq!(require(&held, "gonk-chat", READ), Ok(()));
        assert_eq!(
            require(&held, "gonk-chat", POST),
            Err(Refusal::NotGranted("urn:cap:slack:channel:gonk-chat:post".to_owned()))
        );
        assert!(require(&held, "fixme", READ).is_err());
    }

    #[test]
    fn a_post_is_recorded_and_returns_its_timestamp() {
        let mut slack = FakeSlack::new();
        let ts = slack.post("C1", None, "Blocked on a token").unwrap();
        assert_eq!(slack.posts().len(), 1);
        assert_eq!(slack.posts()[0].ts, ts);
        assert_eq!(slack.posts()[0].thread, None);
    }

    #[test]
    fn a_post_answers_the_timestamp_the_test_chose_then_mints_its_own_again() {
        let mut slack = FakeSlack::new();
        slack.answer_next_post("not-a-timestamp");
        assert_eq!(slack.post("C1", None, "Opening").unwrap(), "not-a-timestamp");
        assert_eq!(slack.posts()[0].ts, "not-a-timestamp");
        assert!(is_slack_ts(&slack.post("C1", None, "Again").unwrap()));
    }

    #[test]
    fn a_reply_names_the_message_it_is_in() {
        let mut slack = FakeSlack::new();
        let root = slack.post("C1", None, "Opening").unwrap();
        slack.post("C1", Some(&root), "Reply").unwrap();
        assert_eq!(slack.posts()[1].thread.as_deref(), Some(root.as_str()));
    }

    #[test]
    fn a_reaction_is_added_once_and_removed() {
        let mut slack = FakeSlack::new();
        let ts = slack.post("C1", None, "Opening").unwrap();
        slack.react("C1", &ts, "white_check_mark", true).unwrap();
        slack.react("C1", &ts, "white_check_mark", true).unwrap();
        assert_eq!(slack.reactions().len(), 1);
        slack.react("C1", &ts, "white_check_mark", false).unwrap();
        slack.react("C1", &ts, "white_check_mark", false).unwrap();
        assert!(slack.reactions().is_empty());
    }

    #[test]
    fn a_reaction_on_an_unknown_message_is_rejected() {
        let mut slack = FakeSlack::new();
        let err = slack.react("C1", "1700000000.999999", "tada", true).unwrap_err();
        assert_eq!(err, TransportError::Rejected("message_not_found".to_owned()));
    }

    #[test]
    fn replies_are_the_opening_message_then_the_fed_replies() {
        let mut slack = FakeSlack::new();
        let root = slack.post("C1", None, "Blocked on a token").unwrap();
        slack.feed(Event::Message {
            id: "Ev1".to_owned(),
            channel: "C1".to_owned(),
            user: "U1".to_owned(),
            ts: "1700000100.000001".to_owned(),
            thread: Some(root.clone()),
            text: "Which token?".to_owned(),
        });
        let messages = slack.replies("C1", &root).unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages[0].bot && messages[0].text == "Blocked on a token");
        assert!(!messages[1].bot && messages[1].user == "U1");
    }

    #[test]
    fn fed_events_are_taken_once_in_order() {
        let mut slack = FakeSlack::new();
        let reaction = Event::Reaction {
            id: "Ev2".to_owned(),
            channel: "C1".to_owned(),
            user: "U1".to_owned(),
            message: "1700000000.000001".to_owned(),
            name: "tada".to_owned(),
        };
        slack.feed(reaction.clone());
        assert_eq!(slack.take_events(), vec![reaction]);
        assert!(slack.take_events().is_empty());
    }

    #[test]
    fn every_resource_is_described_with_its_capabilities() {
        let described: Vec<&str> = descriptions().iter().map(|meta| meta.resource).collect();
        assert_eq!(described, ["urn:slack:channel:{name}:thread:{key}"]);
        for meta in descriptions() {
            assert!(!meta.description.is_empty());
        }
    }

    #[test]
    fn no_description_carries_a_token() {
        for meta in descriptions() {
            assert!(!meta.description.contains("xoxb-"));
            assert!(!meta.description.contains("xapp-"));
        }
    }

    #[test]
    fn an_undeclared_channel_is_refused_as_no_channel_naming_it() {
        let declared = ["gonk-chat".to_owned()];
        assert_eq!(check_channel(&declared, "gonk-chat"), Ok("gonk-chat"));
        let err = check_channel(&declared, "random").unwrap_err();
        assert_eq!(err.code(), "no-channel");
        assert_eq!(err, Refusal::NoChannel("random".to_owned()));
    }

    #[test]
    fn a_key_with_a_space_is_refused() {
        let err = check_key("item 54").unwrap_err();
        assert_eq!(err.code(), "bad-key");
    }

    #[test]
    fn a_key_of_letters_digits_dots_underscores_and_dashes_is_accepted() {
        assert_eq!(check_key("carducci-54.v2_final"), Ok("carducci-54.v2_final"));
    }

    #[test]
    fn an_empty_key_is_refused() {
        assert_eq!(check_key(""), Err(Refusal::BadKey));
    }

    #[test]
    fn a_key_of_exactly_the_longest_length_is_accepted_and_one_longer_refused() {
        let longest = "k".repeat(MAX_KEY_LEN);
        assert_eq!(check_key(&longest), Ok(longest.as_str()));
        assert_eq!(check_key(&"k".repeat(MAX_KEY_LEN + 1)), Err(Refusal::BadKey));
    }

    #[test]
    fn a_key_with_a_non_ascii_letter_is_refused() {
        assert_eq!(check_key("café"), Err(Refusal::BadKey));
    }
}
