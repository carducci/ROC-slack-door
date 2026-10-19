//! Slack conversations as resources for ikigai.
//!
//! A thread is `urn:slack:thread:{key}`, named by a key its caller chooses; a reaction on it is
//! `urn:slack:thread:{key}:reaction:{name}`; replies from allowed people land in an inbox space.
//! Everything Slack-specific sits behind a `SlackTransport`, so every scenario in
//! `openspec/` runs against a fake with no network. What each resource does is specified in
//! `openspec/changes/slack-resources/` until that change is archived into `openspec/specs/`.
//!
//! The transport, its in-memory fake and the thread resource are in place; reactions and the
//! inbox are not yet.

use std::collections::{BTreeSet, VecDeque};

mod conversation;
mod inbox;
mod threads;

pub use conversation::{to_json, Entry};
pub use inbox::{Delivery, Inbox};
pub use threads::{Thread, ThreadError, Threads};

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
    NotGranted(&'static str),
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
        }
    }
}

/// The capability that reads a thread.
pub const READ: &str = "urn:cap:slack:read";
/// The capability that posts to a thread.
pub const POST: &str = "urn:cap:slack:post";
/// The capability that adds or removes a reaction on a thread.
pub const REACT: &str = "urn:cap:slack:react";

/// The capabilities one caller holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grants(BTreeSet<String>);

impl Grants {
    pub fn new<I, S>(capabilities: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Grants(capabilities.into_iter().map(Into::into).collect())
    }

    /// A caller holding every capability the door has.
    pub fn all() -> Self {
        Self::new([READ, POST, REACT])
    }

    /// Accepts the caller only when it holds `capability`; otherwise refuses naming it.
    pub fn require(&self, capability: &'static str) -> Result<(), Refusal> {
        if self.0.contains(capability) {
            Ok(())
        } else {
            Err(Refusal::NotGranted(capability))
        }
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
    vec![
        Meta {
            resource: "urn:slack:thread:{key}",
            description: "A conversation in the configured Slack channel, named by a key you \
                choose. Writing to a key with no thread opens one; writing to an open thread \
                replies in it; reading returns its messages in order, in UTC.",
            verbs: vec![("source", READ), ("sink", POST)],
        },
        Meta {
            resource: "urn:slack:thread:{key}:reaction:{name}",
            description: "A reaction on a thread's opening message, such as white_check_mark \
                to mark it done. Adding one twice, or removing one that is not there, changes \
                nothing.",
            verbs: vec![("sink", REACT), ("delete", REACT)],
        },
    ]
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
}

impl FakeSlack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes the next call to the transport fail with `error`, as Slack would answer it.
    pub fn fail_next(&mut self, error: TransportError) {
        self.failures.push_back(error);
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
        let ts = format!("1700000000.{:06}", self.history.len() + 1);
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
    fn a_post_is_recorded_and_returns_its_timestamp() {
        let mut slack = FakeSlack::new();
        let ts = slack.post("C1", None, "Blocked on a token").unwrap();
        assert_eq!(slack.posts().len(), 1);
        assert_eq!(slack.posts()[0].ts, ts);
        assert_eq!(slack.posts()[0].thread, None);
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
        assert_eq!(
            described,
            ["urn:slack:thread:{key}", "urn:slack:thread:{key}:reaction:{name}"]
        );
        for meta in descriptions() {
            assert!(!meta.description.is_empty());
            assert!(meta.verbs.iter().all(|(_, capability)| capability.starts_with("urn:cap:slack:")));
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
