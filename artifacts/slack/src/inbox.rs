//! The inbox: what allowed people say in the configured channel, turned into deliveries for a
//! host to act on. A delivery names the thread it belongs to by the key the caller chose, never
//! by a Slack id, and a message outside any thread the door opened carries no key.

use std::collections::BTreeSet;

use crate::conversation::utc;
use crate::{Event, SlackTransport, Threads};

/// One thing the inbox delivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// A message by an allowed person. `key` is the thread it replies in, when the door opened
    /// that thread; a message outside any such thread has no key.
    Message {
        key: Option<String>,
        user: String,
        /// The time in UTC, as `2023-11-14T22:13:20Z`.
        time: String,
        text: String,
    },
    /// A reaction by an allowed person on the opening message of the thread `key` names.
    Reaction {
        key: String,
        user: String,
        name: String,
    },
}

/// The filter between Slack's events and the inbox: only allowed people, in the configured
/// channel, and never the door's own bot.
#[derive(Debug, Clone)]
pub struct Inbox {
    channel: String,
    bot_user: String,
    allowed: BTreeSet<String>,
}

impl Inbox {
    /// An inbox for `channel`, delivering only what the Slack users in `allowed` say, and never
    /// what `bot_user` posts.
    pub fn new<I, S>(channel: &str, bot_user: &str, allowed: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Inbox {
            channel: channel.to_owned(),
            bot_user: bot_user.to_owned(),
            allowed: allowed.into_iter().map(Into::into).collect(),
        }
    }

    /// Decides what, if anything, an event delivers. `threads` says which thread an event's
    /// timestamp opens, so a reply in a thread the door opened carries its key.
    pub fn deliver<T: SlackTransport>(&self, event: Event, threads: &Threads<T>) -> Option<Delivery> {
        match event {
            Event::Message {
                channel,
                user,
                ts,
                thread,
                text,
            } => {
                if !self.may_speak(&channel, &user) {
                    return None;
                }
                let key = thread.as_deref().and_then(|root| threads.key_for(root));
                Some(Delivery::Message {
                    key,
                    user,
                    time: utc(&ts),
                    text,
                })
            }
            Event::Reaction {
                channel,
                user,
                message,
                name,
            } => {
                if !self.may_speak(&channel, &user) {
                    return None;
                }
                let key = threads.key_for(&message)?;
                Some(Delivery::Reaction { key, user, name })
            }
        }
    }

    /// Whether a person in a channel may be delivered: the configured channel, an allowed
    /// person, and not the door's own bot. A message with no user came from a bot.
    fn may_speak(&self, channel: &str, user: &str) -> bool {
        channel == self.channel
            && !user.is_empty()
            && user != self.bot_user
            && self.allowed.contains(user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FakeSlack, Threads};
    use std::path::PathBuf;

    const CHANNEL: &str = "C1";
    const BOT: &str = "UBOT";

    /// A state directory with one thread, `carducci-54`, opened by the door.
    fn with_thread(name: &str) -> (Threads<FakeSlack>, String) {
        let dir: PathBuf =
            std::env::temp_dir().join(format!("roc-slack-door-inbox-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut threads = Threads::open(&dir, CHANNEL, FakeSlack::new()).unwrap();
        threads
            .write(&crate::Grants::all(), "carducci-54", "Blocked on a token")
            .unwrap();
        let root = threads.transport().posts()[0].ts.clone();
        (threads, root)
    }

    fn inbox() -> Inbox {
        Inbox::new(CHANNEL, BOT, ["U1"])
    }

    fn reply(channel: &str, user: &str, thread: Option<&str>, text: &str) -> Event {
        Event::Message {
            channel: channel.to_owned(),
            user: user.to_owned(),
            ts: "1700000100.000001".to_owned(),
            thread: thread.map(str::to_owned),
            text: text.to_owned(),
        }
    }

    #[test]
    fn an_allowed_reply_in_a_thread_lands_with_its_key() {
        let (threads, root) = with_thread("reply");
        let delivery = inbox()
            .deliver(reply(CHANNEL, "U1", Some(&root), "Use the fine-grained token"), &threads);
        assert_eq!(
            delivery,
            Some(Delivery::Message {
                key: Some("carducci-54".to_owned()),
                user: "U1".to_owned(),
                time: "2023-11-14T22:15:00Z".to_owned(),
                text: "Use the fine-grained token".to_owned(),
            })
        );
    }

    #[test]
    fn a_stranger_is_not_delivered() {
        let (threads, root) = with_thread("stranger");
        assert_eq!(inbox().deliver(reply(CHANNEL, "U9", Some(&root), "Hi"), &threads), None);
    }

    #[test]
    fn the_doors_own_posts_are_not_delivered() {
        let (threads, root) = with_thread("own");
        assert_eq!(inbox().deliver(reply(CHANNEL, BOT, Some(&root), "Hi"), &threads), None);
    }

    #[test]
    fn a_bot_message_with_no_user_is_not_delivered() {
        let (threads, root) = with_thread("bot");
        assert_eq!(inbox().deliver(reply(CHANNEL, "", Some(&root), "Hi"), &threads), None);
    }

    #[test]
    fn a_message_in_another_channel_is_not_delivered() {
        let (threads, root) = with_thread("channel");
        assert_eq!(inbox().deliver(reply("C2", "U1", Some(&root), "Hi"), &threads), None);
    }

    #[test]
    fn a_top_level_message_lands_with_no_key() {
        let (threads, _) = with_thread("top-level");
        let delivery = inbox().deliver(reply(CHANNEL, "U1", None, "What's blocked today?"), &threads);
        assert!(matches!(delivery, Some(Delivery::Message { key: None, .. })));
    }

    #[test]
    fn a_reply_in_a_thread_the_door_did_not_open_lands_with_no_key() {
        let (threads, _) = with_thread("not-ours");
        let delivery = inbox().deliver(reply(CHANNEL, "U1", Some("1700000000.999999"), "Hi"), &threads);
        assert!(matches!(delivery, Some(Delivery::Message { key: None, .. })));
    }

    #[test]
    fn an_allowed_reaction_on_an_opening_message_lands_with_the_key() {
        let (threads, root) = with_thread("reaction");
        let event = Event::Reaction {
            channel: CHANNEL.to_owned(),
            user: "U1".to_owned(),
            message: root,
            name: "+1".to_owned(),
        };
        assert_eq!(
            inbox().deliver(event, &threads),
            Some(Delivery::Reaction {
                key: "carducci-54".to_owned(),
                user: "U1".to_owned(),
                name: "+1".to_owned(),
            })
        );
    }

    #[test]
    fn a_reaction_on_a_message_the_door_did_not_open_is_not_delivered() {
        let (threads, _) = with_thread("reaction-other");
        let event = Event::Reaction {
            channel: CHANNEL.to_owned(),
            user: "U1".to_owned(),
            message: "1700000000.999999".to_owned(),
            name: "+1".to_owned(),
        };
        assert_eq!(inbox().deliver(event, &threads), None);
    }
}
