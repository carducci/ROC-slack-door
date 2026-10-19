//! Slack conversations as resources for ikigai.
//!
//! A thread is `urn:slack:thread:{key}`, named by a key its caller chooses; a reaction on it is
//! `urn:slack:thread:{key}:reaction:{name}`; replies from allowed people land in an inbox space.
//! Everything Slack-specific sits behind a `SlackTransport`, so every scenario in
//! `openspec/` runs against a fake with no network. What each resource does is specified in
//! `openspec/changes/slack-resources/` until that change is archived into `openspec/specs/`.
//!
//! The transport and its in-memory fake are in place; the resources that use them are not yet.

use std::collections::BTreeSet;

mod threads;

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

/// Something a person did in the channel that the door hears about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A message. `thread` is the timestamp of the message it replies to, if any.
    Message {
        channel: String,
        user: String,
        ts: String,
        thread: Option<String>,
        text: String,
    },
    /// A reaction added to the message at `message`.
    Reaction {
        channel: String,
        user: String,
        message: String,
        name: String,
    },
}

/// The longest thread key the caller may choose.
pub const MAX_KEY_LEN: usize = 80;

/// Why a resource refused a request, named as the specs name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A thread key is empty, longer than `MAX_KEY_LEN`, or has a character outside
    /// ASCII letters, digits, `.`, `_` and `-`.
    BadKey,
}

impl Refusal {
    /// The name the specs give this refusal, as a caller sees it.
    pub fn code(&self) -> &'static str {
        match self {
            Refusal::BadKey => "bad-key",
        }
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

/// A message the fake has recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Post {
    pub ts: String,
    pub channel: String,
    pub thread: Option<String>,
    pub text: String,
}

/// An in-memory `SlackTransport`: posts and reactions are recorded, events are fed by the test.
#[derive(Debug, Default)]
pub struct FakeSlack {
    posts: Vec<Post>,
    reactions: BTreeSet<(String, String)>,
    events: Vec<Event>,
}

impl FakeSlack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every message posted so far, oldest first.
    pub fn posts(&self) -> &[Post] {
        &self.posts
    }

    /// Every reaction present, as (message timestamp, reaction name).
    pub fn reactions(&self) -> &BTreeSet<(String, String)> {
        &self.reactions
    }

    /// Queues an event for the next `take_events`.
    pub fn feed(&mut self, event: Event) {
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
        let ts = format!("1700000000.{:06}", self.posts.len() + 1);
        self.posts.push(Post {
            ts: ts.clone(),
            channel: channel.to_owned(),
            thread: thread.map(str::to_owned),
            text: text.to_owned(),
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
        if !self.posts.iter().any(|post| post.ts == message) {
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

    #[test]
    fn fed_events_are_taken_once_in_order() {
        let mut slack = FakeSlack::new();
        let reaction = Event::Reaction {
            channel: "C1".to_owned(),
            user: "U1".to_owned(),
            message: "1700000000.000001".to_owned(),
            name: "tada".to_owned(),
        };
        slack.feed(reaction.clone());
        assert_eq!(slack.take_events(), vec![reaction]);
        assert!(slack.take_events().is_empty());
    }
}
