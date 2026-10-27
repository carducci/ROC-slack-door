//! The inbox: what allowed people say in the configured channel, turned into deliveries for a
//! host to act on. A delivery names the thread it belongs to by the key the caller chose, never
//! by a Slack id. A speaker's top-level message opens a thread under a key the door mints, and a
//! reply in a thread the door does not know carries no key.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::conversation::utc;
use crate::{check_key, Event, SlackTransport, Threads};

/// The file in the state directory that lists the ids of events already delivered.
const DELIVERED_FILE: &str = "delivered.txt";

/// One thing the inbox delivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// A message by an allowed person. `key` is the thread it replies in, when the door opened
    /// that thread or minted for a top-level message; a reply in a thread the door does not know
    /// has no key, and neither has a top-level message whose key could not be minted safely (its
    /// timestamp is not Slack-shaped, or the key names another message).
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

/// One declared channel of the inbox: its name, its Slack id and the people who may be delivered.
#[derive(Debug)]
struct InboxChannel {
    name: String,
    id: String,
    speakers: BTreeSet<String>,
}

/// The key the door mints for a message with Slack timestamp `ts` (`1700000000.000001`):
/// `m-1700000000-000001`.
fn mint_key(ts: &str) -> String {
    format!("m-{}", ts.replace('.', "-"))
}

/// Whether `ts` has the shape Slack's timestamps have: digits, one dot, digits.
fn is_slack_ts(ts: &str) -> bool {
    ts.split_once('.').is_some_and(|(seconds, micros)| {
        [seconds, micros]
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// The filter between Slack's events and the inbox: only allowed people, in the configured
/// channel, never the door's own bot, and each event at most once across restarts.
#[derive(Debug)]
pub struct Inbox {
    channels: Vec<InboxChannel>,
    bot_user: String,
    state_file: PathBuf,
    delivered: BTreeSet<String>,
}

impl Inbox {
    /// An inbox for `channel`, delivering only what the Slack users in `allowed` say, and never
    /// what `bot_user` posts. The ids of delivered events are kept in `state_dir`.
    pub fn open<I, S>(state_dir: &Path, channel: &str, bot_user: &str, allowed: I) -> io::Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let allowed = allowed.into_iter().map(Into::into).collect();
        Self::open_channels(state_dir, bot_user, &[(String::new(), channel.to_owned(), allowed)])
    }

    /// An inbox for each declared channel, a (name, Slack id, speakers) triple, delivering from a
    /// channel only what that channel's speakers say. The ids of delivered events are kept in
    /// `state_dir`.
    pub fn open_channels(
        state_dir: &Path,
        bot_user: &str,
        channels: &[(String, String, Vec<String>)],
    ) -> io::Result<Self> {
        fs::create_dir_all(state_dir)?;
        let state_file = state_dir.join(DELIVERED_FILE);
        let delivered = match fs::read_to_string(&state_file) {
            Ok(text) => text.lines().map(str::to_owned).collect(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeSet::new(),
            Err(e) => return Err(e),
        };
        Ok(Inbox {
            channels: channels
                .iter()
                .map(|(name, id, speakers)| InboxChannel {
                    name: name.clone(),
                    id: id.clone(),
                    speakers: speakers.iter().cloned().collect(),
                })
                .collect(),
            bot_user: bot_user.to_owned(),
            state_file,
            delivered,
        })
    }

    /// Decides what, if anything, an event delivers. A redelivery of an event already delivered
    /// delivers nothing. `threads` says which thread an event's timestamp opens, so a reply in a
    /// thread the door opened carries its key; a speaker's top-level message is recorded in it, and
    /// saved, as a new thread.
    ///
    /// The id is recorded before the delivery is returned: a crash after this point loses the
    /// delivery rather than repeating it, which is the at-most-once the spec asks for.
    pub fn deliver<T: SlackTransport>(
        &mut self,
        event: Event,
        threads: &mut Threads<T>,
    ) -> io::Result<Option<Delivery>> {
        Ok(self.deliver_to(event, threads)?.map(|(_, delivery)| delivery))
    }

    /// `deliver`, naming the declared channel whose inbox the delivery belongs to.
    pub fn deliver_to<T: SlackTransport>(
        &mut self,
        event: Event,
        threads: &mut Threads<T>,
    ) -> io::Result<Option<(String, Delivery)>> {
        let event_id = event.id().to_owned();
        if self.delivered.contains(&event_id) {
            return Ok(None);
        }
        let Some(delivery) = self.decide(event, threads)? else {
            return Ok(None);
        };
        self.remember(&event_id)?;
        Ok(Some(delivery))
    }

    fn decide<T: SlackTransport>(
        &self,
        event: Event,
        threads: &mut Threads<T>,
    ) -> io::Result<Option<(String, Delivery)>> {
        Ok(match event {
            Event::Message {
                channel,
                user,
                ts,
                thread,
                text,
                ..
            } => {
                let Some(name) = self.speaker_channel(&channel, &user) else {
                    return Ok(None);
                };
                let key = match thread.as_deref() {
                    Some(root) => threads.key_for(&channel, root),
                    // A speaker's top-level message opens a thread under a key minted from its time.
                    // A timestamp of another shape, or a key a caller already holds for another
                    // message, gets no key: the message is delivered as before, and logged.
                    None => {
                        let key = mint_key(&ts);
                        if !is_slack_ts(&ts) || check_key(&key).is_err() {
                            eprintln!("no key for the message at {ts:?} in {channel}: not a Slack timestamp");
                            None
                        } else if threads.adopt(&channel, &key, &ts)? {
                            Some(key)
                        } else {
                            eprintln!("no key for the message at {ts} in {channel}: {key} names another message");
                            None
                        }
                    }
                };
                Some((
                    name,
                    Delivery::Message {
                        key,
                        user,
                        time: utc(&ts),
                        text,
                    },
                ))
            }
            Event::Reaction {
                channel,
                user,
                message,
                name,
                ..
            } => {
                self.speaker_channel(&channel, &user).and_then(|channel_name| {
                    let key = threads.key_for(&channel, &message)?;
                    Some((channel_name, Delivery::Reaction { key, user, name }))
                })
            }
        })
    }

    /// The name of the declared channel `channel` (a Slack id) when `user` may be delivered from
    /// it: one of that channel's speakers, and not the door's own bot. A message with no user
    /// came from a bot.
    fn speaker_channel(&self, channel: &str, user: &str) -> Option<String> {
        let declared = self.channels.iter().find(|c| c.id == channel)?;
        (!user.is_empty() && user != self.bot_user && declared.speakers.contains(user))
            .then(|| declared.name.clone())
    }

    /// Appends the id to the delivered file, synced, before it is remembered in memory.
    fn remember(&mut self, event_id: &str) -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.state_file)?;
        writeln!(file, "{event_id}")?;
        file.sync_all()?;
        self.delivered.insert(event_id.to_owned());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FakeSlack, Grants};

    const CHANNEL: &str = "C1";
    const BOT: &str = "UBOT";

    /// A state directory, empty, for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("roc-slack-door-inbox-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    /// Threads with one thread, `carducci-54`, opened by the door, and that thread's timestamp.
    fn with_thread(dir: &Path) -> (Threads<FakeSlack>, String) {
        let mut threads = Threads::open(dir, CHANNEL, FakeSlack::new()).unwrap();
        threads
            .write(&Grants::all(), "carducci-54", "Blocked on a token")
            .unwrap();
        let root = threads.transport().posts()[0].ts.clone();
        (threads, root)
    }

    fn inbox(dir: &Path) -> Inbox {
        Inbox::open(dir, CHANNEL, BOT, ["U1"]).unwrap()
    }

    fn reply(channel: &str, user: &str, thread: Option<&str>, text: &str) -> Event {
        Event::Message {
            id: "Ev1".to_owned(),
            channel: channel.to_owned(),
            user: user.to_owned(),
            ts: "1700000100.000001".to_owned(),
            thread: thread.map(str::to_owned),
            text: text.to_owned(),
        }
    }

    #[test]
    fn an_allowed_reply_in_a_thread_lands_with_its_key() {
        let dir = scratch("reply");
        let (mut threads, root) = with_thread(&dir);
        let event = reply(CHANNEL, "U1", Some(&root), "Use the fine-grained token");
        let delivery = inbox(&dir).deliver(event, &mut threads).unwrap();
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
        let dir = scratch("stranger");
        let (mut threads, root) = with_thread(&dir);
        let event = reply(CHANNEL, "U9", Some(&root), "Hi");
        assert_eq!(inbox(&dir).deliver(event, &mut threads).unwrap(), None);
    }

    #[test]
    fn the_doors_own_posts_are_not_delivered() {
        let dir = scratch("own");
        let (mut threads, root) = with_thread(&dir);
        let event = reply(CHANNEL, BOT, Some(&root), "Hi");
        assert_eq!(inbox(&dir).deliver(event, &mut threads).unwrap(), None);
    }

    #[test]
    fn a_bot_message_with_no_user_is_not_delivered() {
        let dir = scratch("bot");
        let (mut threads, root) = with_thread(&dir);
        let event = reply(CHANNEL, "", Some(&root), "Hi");
        assert_eq!(inbox(&dir).deliver(event, &mut threads).unwrap(), None);
    }

    #[test]
    fn a_message_in_another_channel_is_not_delivered() {
        let dir = scratch("channel");
        let (mut threads, root) = with_thread(&dir);
        let event = reply("C2", "U1", Some(&root), "Hi");
        assert_eq!(inbox(&dir).deliver(event, &mut threads).unwrap(), None);
    }

    #[test]
    fn a_top_level_message_lands_with_a_minted_key() {
        let dir = scratch("top-level");
        let (mut threads, _) = with_thread(&dir);
        let event = reply(CHANNEL, "U1", None, "What's blocked today?");
        let delivery = inbox(&dir).deliver(event, &mut threads).unwrap();
        assert!(matches!(
            delivery,
            Some(Delivery::Message { key: Some(k), .. }) if k == "m-1700000100-000001"
        ));
    }

    #[test]
    fn writing_to_a_minted_key_replies_under_the_speakers_message() {
        let (mut inbox, mut threads, _) = two_channels("two-minted");
        let event = reply("C2", "U2", None, "bug: the save button overlaps the footer");
        let (channel, delivery) = inbox.deliver_to(event, &mut threads).unwrap().unwrap();
        assert_eq!(channel, "fixme");
        let Delivery::Message { key: Some(key), user, text, .. } = delivery else {
            panic!("expected a message with a key");
        };
        assert_eq!((user.as_str(), text.as_str()), ("U2", "bug: the save button overlaps the footer"));
        threads.write_in(&Grants::all(), "fixme", &key, "Fixed").unwrap();
        let post = &threads.transport().posts()[1];
        assert_eq!(post.thread.as_deref(), Some("1700000100.000001"));
    }

    #[test]
    fn a_minted_key_survives_a_restart_and_names_the_thread_for_replies() {
        let dir = scratch("minted-restart");
        let (mut threads, _) = with_thread(&dir);
        let event = reply(CHANNEL, "U1", None, "What's blocked today?");
        inbox(&dir).deliver(event, &mut threads).unwrap();
        drop(threads);

        let mut threads = Threads::open(&dir, CHANNEL, FakeSlack::new()).unwrap();
        assert_eq!(threads.key_for(CHANNEL, "1700000100.000001").as_deref(), Some("m-1700000100-000001"));
        let in_it = Event::Message {
            id: "Ev2".to_owned(),
            channel: CHANNEL.to_owned(),
            user: "U1".to_owned(),
            ts: "1700000200.000001".to_owned(),
            thread: Some("1700000100.000001".to_owned()),
            text: "Also this".to_owned(),
        };
        let delivery = inbox(&dir).deliver(in_it, &mut threads).unwrap();
        assert!(matches!(
            delivery,
            Some(Delivery::Message { key: Some(k), .. }) if k == "m-1700000100-000001"
        ));
    }

    #[test]
    fn a_top_level_message_whose_save_failed_is_keyed_on_redelivery_only_once_saved() {
        let dir = scratch("save-fails");
        let (mut threads, _) = with_thread(&dir);
        let blocker = dir.join("threads.tsv.tmp");
        fs::create_dir(&blocker).unwrap();
        let mut inbox = inbox(&dir);
        let event = reply(CHANNEL, "U1", None, "What's blocked today?");
        assert!(inbox.deliver(event.clone(), &mut threads).is_err());
        assert_eq!(threads.key_for(CHANNEL, "1700000100.000001"), None);

        fs::remove_dir(&blocker).unwrap();
        let delivery = inbox.deliver(event, &mut threads).unwrap();
        assert!(matches!(
            delivery,
            Some(Delivery::Message { key: Some(k), .. }) if k == "m-1700000100-000001"
        ));
        drop(threads);
        let threads = Threads::open(&dir, CHANNEL, FakeSlack::new()).unwrap();
        assert_eq!(
            threads.key_for(CHANNEL, "1700000100.000001").as_deref(),
            Some("m-1700000100-000001")
        );
    }

    #[test]
    fn a_minted_key_a_caller_already_holds_is_never_handed_out() {
        let dir = scratch("collision");
        let (mut threads, _) = with_thread(&dir);
        threads
            .write(&Grants::all(), "m-1700000100-000001", "The caller's own")
            .unwrap();
        let event = reply(CHANNEL, "U1", None, "What's blocked today?");
        let delivery = inbox(&dir).deliver(event, &mut threads).unwrap();
        assert!(matches!(delivery, Some(Delivery::Message { key: None, .. })));
        assert_eq!(threads.key_for(CHANNEL, "1700000100.000001"), None);
    }

    #[test]
    fn a_timestamp_that_is_not_digits_dot_digits_mints_no_key() {
        let bad_timestamps = [
            "1700000100.000001\tC1\tx",
            "1700000100.0\nevil",
            "17000:00100.000001",
            "abc",
            "1.2.3",
            ".5",
            "5.",
            "",
        ];
        for (n, bad) in bad_timestamps.into_iter().enumerate() {
            let dir = scratch(&format!("bad-ts-{n}"));
            let (mut threads, _) = with_thread(&dir);
            let mut event = reply(CHANNEL, "U1", None, "Hi");
            if let Event::Message { ts, .. } = &mut event {
                *ts = bad.to_owned();
            }
            let delivery = inbox(&dir).deliver(event, &mut threads).unwrap();
            assert!(matches!(delivery, Some(Delivery::Message { key: None, .. })), "{bad:?}");
            drop(threads);
            Threads::open(&dir, CHANNEL, FakeSlack::new()).unwrap();
        }
    }

    #[test]
    fn a_stranger_top_level_message_opens_no_thread() {
        let dir = scratch("stranger-top");
        let (mut threads, _) = with_thread(&dir);
        let event = reply(CHANNEL, "U9", None, "Hi");
        assert_eq!(inbox(&dir).deliver(event, &mut threads).unwrap(), None);
        assert_eq!(threads.key_for(CHANNEL, "1700000100.000001"), None);
    }

    #[test]
    fn a_reply_in_a_thread_the_door_did_not_open_lands_with_no_key() {
        let dir = scratch("not-ours");
        let (mut threads, _) = with_thread(&dir);
        let event = reply(CHANNEL, "U1", Some("1700000000.999999"), "Hi");
        let delivery = inbox(&dir).deliver(event, &mut threads).unwrap();
        assert!(matches!(delivery, Some(Delivery::Message { key: None, .. })));
    }

    #[test]
    fn an_allowed_reaction_on_an_opening_message_lands_with_the_key() {
        let dir = scratch("reaction");
        let (mut threads, root) = with_thread(&dir);
        let event = Event::Reaction {
            id: "Ev1".to_owned(),
            channel: CHANNEL.to_owned(),
            user: "U1".to_owned(),
            message: root,
            name: "+1".to_owned(),
        };
        assert_eq!(
            inbox(&dir).deliver(event, &mut threads).unwrap(),
            Some(Delivery::Reaction {
                key: "carducci-54".to_owned(),
                user: "U1".to_owned(),
                name: "+1".to_owned(),
            })
        );
    }

    #[test]
    fn a_reaction_on_a_message_the_door_did_not_open_is_not_delivered() {
        let dir = scratch("reaction-other");
        let (mut threads, _) = with_thread(&dir);
        let event = Event::Reaction {
            id: "Ev1".to_owned(),
            channel: CHANNEL.to_owned(),
            user: "U1".to_owned(),
            message: "1700000000.999999".to_owned(),
            name: "+1".to_owned(),
        };
        assert_eq!(inbox(&dir).deliver(event, &mut threads).unwrap(), None);
    }

    /// Two declared channels: `gonk-chat` (C1, speaker U1) and `fixme` (C2, speaker U2), with the
    /// thread `carducci-54` open in `gonk-chat`, and that thread's timestamp.
    fn two_channels(name: &str) -> (Inbox, Threads<FakeSlack>, String) {
        let dir = scratch(name);
        let named = [("gonk-chat".to_owned(), "C1".to_owned()), ("fixme".to_owned(), "C2".to_owned())];
        let mut threads = Threads::open_channels(&dir, &named, FakeSlack::new()).unwrap();
        threads
            .write_in(&Grants::all(), "gonk-chat", "carducci-54", "Blocked on a token")
            .unwrap();
        let root = threads.transport().posts()[0].ts.clone();
        let channels = [
            ("gonk-chat".to_owned(), "C1".to_owned(), vec!["U1".to_owned(), BOT.to_owned()]),
            ("fixme".to_owned(), "C2".to_owned(), vec!["U2".to_owned()]),
        ];
        (Inbox::open_channels(&dir, BOT, &channels).unwrap(), threads, root)
    }

    #[test]
    fn a_speakers_reply_lands_in_the_inbox_of_its_channel() {
        let (mut inbox, mut threads,root) = two_channels("two-reply");
        let event = reply("C1", "U1", Some(&root), "Use the fine-grained token");
        let (channel, delivery) = inbox.deliver_to(event, &mut threads).unwrap().unwrap();
        assert_eq!(channel, "gonk-chat");
        assert!(matches!(delivery, Delivery::Message { key: Some(k), .. } if k == "carducci-54"));
    }

    #[test]
    fn a_stranger_is_in_no_inbox() {
        let (mut inbox, mut threads,root) = two_channels("two-stranger");
        let event = reply("C1", "U9", Some(&root), "Hi");
        assert_eq!(inbox.deliver_to(event, &mut threads).unwrap(), None);
    }

    #[test]
    fn the_door_is_in_no_inbox_even_when_listed_as_a_speaker() {
        let (mut inbox, mut threads,root) = two_channels("two-own");
        let event = reply("C1", BOT, Some(&root), "Hi");
        assert_eq!(inbox.deliver_to(event, &mut threads).unwrap(), None);
    }

    #[test]
    fn a_speaker_of_one_channel_is_in_no_inbox_of_another() {
        let (mut inbox, mut threads,root) = two_channels("two-cross");
        let event = reply("C1", "U2", Some(&root), "Hi");
        assert_eq!(inbox.deliver_to(event, &mut threads).unwrap(), None);
    }

    #[test]
    fn an_undeclared_channel_is_in_no_inbox() {
        let (mut inbox, mut threads,_) = two_channels("two-undeclared");
        let event = reply("C3", "U1", None, "Hi");
        assert_eq!(inbox.deliver_to(event, &mut threads).unwrap(), None);
    }

    #[test]
    fn a_speaker_of_the_other_channel_lands_in_that_channels_inbox() {
        let (mut inbox, mut threads,_) = two_channels("two-other");
        let event = reply("C2", "U2", None, "bug: the save button overlaps the footer");
        let (channel, _) = inbox.deliver_to(event, &mut threads).unwrap().unwrap();
        assert_eq!(channel, "fixme");
    }

    #[test]
    fn a_speakers_reaction_lands_in_the_inbox_of_its_channel() {
        let (mut inbox, mut threads,root) = two_channels("two-reaction");
        let event = Event::Reaction {
            id: "Ev1".to_owned(),
            channel: "C1".to_owned(),
            user: "U1".to_owned(),
            message: root,
            name: "+1".to_owned(),
        };
        let (channel, delivery) = inbox.deliver_to(event, &mut threads).unwrap().unwrap();
        assert_eq!(channel, "gonk-chat");
        assert_eq!(
            delivery,
            Delivery::Reaction {
                key: "carducci-54".to_owned(),
                user: "U1".to_owned(),
                name: "+1".to_owned(),
            }
        );
    }

    #[test]
    fn a_redelivered_event_is_delivered_once_even_across_a_restart() {
        let dir = scratch("redeliver");
        let (mut threads, root) = with_thread(&dir);
        let event = reply(CHANNEL, "U1", Some(&root), "Which token?");
        let mut before = inbox(&dir);
        assert!(before.deliver(event.clone(), &mut threads).unwrap().is_some());
        assert_eq!(before.deliver(event.clone(), &mut threads).unwrap(), None);
        drop(before);

        let mut after = inbox(&dir);
        assert_eq!(after.deliver(event, &mut threads).unwrap(), None);
    }
}
