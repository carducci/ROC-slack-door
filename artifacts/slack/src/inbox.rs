//! The inbox: what allowed people say in the configured channel, turned into deliveries for a
//! host to act on. A delivery names the thread it belongs to by the key the caller chose, never
//! by a Slack id. A speaker's top-level message opens a thread under a key the door mints, and a
//! reply in a thread the door does not know carries no key.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::conversation::utc;
use crate::intake::{self, Accepts};
use crate::{check_key, is_slack_ts, Event, Files, Objects, SlackTransport, Threads};

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
        /// One entry per file the message carried, in its order: its resource name, or the
        /// reason it was refused (`files-not-accepted`, `file-unavailable`, `file-too-large` or
        /// `file-type-refused`).
        files: Vec<String>,
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

/// The filter between Slack's events and the inbox: only allowed people, in the configured
/// channel, never the door's own bot, and each event at most once across restarts.
#[derive(Debug)]
pub struct Inbox {
    channels: Vec<InboxChannel>,
    bot_user: String,
    state_file: PathBuf,
    delivered: BTreeSet<String>,
    /// What each channel, by name, accepts as files; a channel not here takes none.
    accepts: BTreeMap<String, Accepts>,
    objects: Objects,
    files: Files,
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
            accepts: BTreeMap::new(),
            objects: Objects::open(state_dir)?,
            files: Files::open(state_dir)?,
        })
    }

    /// Lets the declared channel `name` take files: those whose bytes are one of `accepts`'s
    /// types and no larger than its limit. They are kept in the state directory's object store
    /// and named in the channel's index.
    pub fn accept_files(&mut self, name: &str, accepts: Accepts) {
        self.accepts.insert(name.to_owned(), accepts);
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
                files,
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
                let accepts = self.accepts.get(&name);
                let files = files
                    .iter()
                    .map(|file| {
                        let transport = threads.transport_mut();
                        intake::take(transport, &self.objects, &self.files, &name, accepts, file)
                    })
                    .collect::<io::Result<_>>()?;
                Some((
                    name,
                    Delivery::Message {
                        key,
                        user,
                        time: utc(&ts),
                        text,
                        files,
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
    use crate::FakeSlack;
    use ikigai_core::Capability;

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
            .write(&Capability::root(), "carducci-54", "Blocked on a token")
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
            files: Vec::new(),
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
                files: Vec::new(),
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
    fn a_file_in_a_channel_that_takes_none_lands_refused_as_files_not_accepted() {
        // Spec `files`, "A file in a channel that takes none": the tuple carries the text and
        // its one file refused as `files-not-accepted`, not the text alone as if no file came.
        let dir = scratch("file-none");
        let (mut threads, _) = with_thread(&dir);
        let Event::Message { id, channel, user, ts, thread, text, .. } =
            reply(CHANNEL, "U1", None, "cosmetic: the save button overlaps the footer")
        else {
            unreachable!()
        };
        let event = Event::Message {
            id, channel, user, ts, thread, text,
            files: vec![crate::SlackFile {
                id: "F1".to_owned(),
                size: 48213,
                url_private: "https://files.slack.com/files-pri/T1-F1/screenshot.png".to_owned(),
            }],
        };
        let delivery = inbox(&dir).deliver(event, &mut threads).unwrap();
        let shown = format!("{delivery:?}");
        assert!(delivery.is_some(), "the post with a file did not land");
        assert!(
            shown.contains("files-not-accepted"),
            "the file was dropped without a refusal: {shown}"
        );
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
        threads.write_in(&Capability::root(), "fixme", &key, "Fixed").unwrap();
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
            files: Vec::new(),
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
            .write(&Capability::root(), "m-1700000100-000001", "The caller's own")
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
            .write_in(&Capability::root(), "gonk-chat", "carducci-54", "Blocked on a token")
            .unwrap();
        let root = threads.transport().posts()[0].ts.clone();
        let channels = [
            ("gonk-chat".to_owned(), "C1".to_owned(), vec!["U1".to_owned(), BOT.to_owned()]),
            ("fixme".to_owned(), "C2".to_owned(), vec!["U2".to_owned()]),
        ];
        (Inbox::open_channels(&dir, BOT, &channels).unwrap(), threads, root)
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR a screenshot";
    const EXE: &[u8] = b"MZ\x90\0\x03\0\0\0 a Windows executable";

    /// `two_channels`, with `fixme` taking PNGs of at most 100 bytes, and the state directory.
    fn fixme_takes_pngs(name: &str) -> (Inbox, Threads<FakeSlack>, PathBuf) {
        let (mut inbox, threads, _) = two_channels(name);
        inbox.accept_files("fixme", Accepts { types: vec!["image/png".into()], max_bytes: 100 });
        let dir =
            std::env::temp_dir().join(format!("roc-slack-door-inbox-{name}-{}", std::process::id()));
        (inbox, threads, dir)
    }

    /// A post by `fixme`'s speaker carrying `files`, each a (Slack's size, private URL) pair.
    fn post_with(id: &str, channel: &str, files: &[(u64, &str)]) -> Event {
        Event::Message {
            id: id.to_owned(),
            channel: channel.to_owned(),
            user: if channel == "C2" { "U2" } else { "U1" }.to_owned(),
            ts: "1700000200.000100".to_owned(),
            thread: None,
            text: "cosmetic: the save button overlaps the footer".to_owned(),
            files: files
                .iter()
                .map(|(size, url)| crate::SlackFile {
                    id: "F0SHOT0001".to_owned(),
                    size: *size,
                    url_private: url.to_string(),
                })
                .collect(),
        }
    }

    const SHOT: &str = "https://files.slack.com/files-pri/T1-F1/screenshot.png";

    fn files_of(delivery: Option<(String, Delivery)>) -> Vec<String> {
        match delivery {
            Some((_, Delivery::Message { files, .. })) => files,
            other => panic!("the post did not land: {other:?}"),
        }
    }

    /// Every object in the store, by path.
    fn objects_in(dir: &Path) -> Vec<PathBuf> {
        fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
            for e in fs::read_dir(dir).into_iter().flatten().flatten() {
                if e.path().is_dir() {
                    walk(&e.path(), found);
                } else {
                    found.push(e.path());
                }
            }
        }
        let mut found = Vec::new();
        walk(&dir.join("objects"), &mut found);
        found
    }

    #[test]
    fn a_screenshot_lands_named_by_its_digest_and_is_kept_with_its_sniffed_type() {
        let (mut inbox, mut threads, dir) = fixme_takes_pngs("file-png");
        threads.transport_mut().serve_file(SHOT, PNG);
        let event = post_with("Ev1", "C2", &[(PNG.len() as u64, SHOT)]);
        let files = files_of(inbox.deliver_to(event, &mut threads).unwrap());
        let id = ikigai_core::ContentId::of(PNG);
        assert_eq!(files, vec![format!("urn:slack:channel:fixme:file:{id}")]);
        assert_eq!(Objects::open(&dir).unwrap().get(&id).unwrap().as_deref(), Some(PNG));
        let entry = Files::open(&dir).unwrap().get("fixme", &id).unwrap().unwrap();
        assert_eq!(entry.content_type, "image/png");
    }

    #[test]
    fn a_renamed_executable_is_refused_by_its_bytes_and_nothing_is_stored() {
        let (mut inbox, mut threads, dir) = fixme_takes_pngs("file-exe");
        threads.transport_mut().serve_file(SHOT, EXE);
        let event = post_with("Ev1", "C2", &[(EXE.len() as u64, SHOT)]);
        let files = files_of(inbox.deliver_to(event, &mut threads).unwrap());
        assert_eq!(files, vec!["file-type-refused".to_owned()]);
        assert!(objects_in(&dir).is_empty());
        assert!(Files::open(&dir).unwrap().list("fixme").unwrap().is_empty());
    }

    #[test]
    fn an_image_of_a_type_the_channel_does_not_list_is_refused() {
        let (mut inbox, mut threads, dir) = fixme_takes_pngs("file-gif");
        threads.transport_mut().serve_file(SHOT, b"GIF89a\x01\0\x01\0");
        let event = post_with("Ev1", "C2", &[(10, SHOT)]);
        let files = files_of(inbox.deliver_to(event, &mut threads).unwrap());
        assert_eq!(files, vec!["file-type-refused".to_owned()]);
        assert!(objects_in(&dir).is_empty());
    }

    #[test]
    fn a_file_slack_reports_too_large_is_refused_without_a_download() {
        let (mut inbox, mut threads, dir) = fixme_takes_pngs("file-big-reported");
        threads.transport_mut().serve_file(SHOT, PNG);
        let event = post_with("Ev1", "C2", &[(101, SHOT)]);
        let files = files_of(inbox.deliver_to(event, &mut threads).unwrap());
        assert_eq!(files, vec!["file-too-large".to_owned()]);
        assert!(threads.transport().downloads().is_empty());
        assert!(objects_in(&dir).is_empty());
    }

    #[test]
    fn a_file_larger_than_slack_said_is_cut_at_one_byte_past_the_limit_and_refused() {
        let (mut inbox, mut threads, dir) = fixme_takes_pngs("file-big-bytes");
        let mut big = PNG.to_vec();
        big.resize(5000, 0);
        threads.transport_mut().serve_file(SHOT, &big);
        let event = post_with("Ev1", "C2", &[(10, SHOT)]);
        let files = files_of(inbox.deliver_to(event, &mut threads).unwrap());
        assert_eq!(files, vec!["file-too-large".to_owned()]);
        assert!(objects_in(&dir).is_empty());
    }

    #[test]
    fn a_file_of_exactly_the_limit_is_kept() {
        let (mut inbox, mut threads, dir) = fixme_takes_pngs("file-at-limit");
        let mut at = PNG.to_vec();
        at.resize(100, 0);
        threads.transport_mut().serve_file(SHOT, &at);
        let event = post_with("Ev1", "C2", &[(100, SHOT)]);
        let files = files_of(inbox.deliver_to(event, &mut threads).unwrap());
        assert!(files[0].starts_with("urn:slack:channel:fixme:file:b3:"), "{files:?}");
        assert_eq!(objects_in(&dir).len(), 1);
    }

    #[test]
    fn a_file_slack_cannot_serve_is_unavailable_and_the_post_still_lands() {
        let (mut inbox, mut threads, dir) = fixme_takes_pngs("file-unavailable");
        threads.transport_mut().serve_file(SHOT, PNG);
        threads.transport_mut().fail_next(crate::TransportError::Unavailable("HTTP 503".into()));
        let missing = "https://files.slack.com/files-pri/T1-F9/gone.png";
        let event = post_with("Ev1", "C2", &[(10, SHOT), (0, ""), (10, missing)]);
        let files = files_of(inbox.deliver_to(event, &mut threads).unwrap());
        assert_eq!(files, vec!["file-unavailable"; 3]);
        assert!(objects_in(&dir).is_empty());
    }

    #[test]
    fn the_bot_token_goes_only_to_slacks_file_host() {
        let (mut inbox, mut threads, _) = fixme_takes_pngs("file-host");
        let elsewhere = "https://files.slack.com.example.org/a.png";
        threads.transport_mut().serve_file(elsewhere, PNG);
        let event = post_with("Ev1", "C2", &[(10, elsewhere), (10, "http://files.slack.com/a.png")]);
        let files = files_of(inbox.deliver_to(event, &mut threads).unwrap());
        assert_eq!(files, vec!["file-unavailable"; 2]);
        assert!(threads.transport().downloads().is_empty());
    }

    #[test]
    fn a_file_in_a_channel_that_takes_none_is_not_downloaded() {
        let (mut inbox, mut threads, dir) = fixme_takes_pngs("file-none-download");
        threads.transport_mut().serve_file(SHOT, PNG);
        let event = post_with("Ev1", "C1", &[(10, SHOT)]);
        let files = files_of(inbox.deliver_to(event, &mut threads).unwrap());
        assert_eq!(files, vec!["files-not-accepted".to_owned()]);
        assert!(threads.transport().downloads().is_empty());
        assert!(objects_in(&dir).is_empty());
    }

    #[test]
    fn the_same_screenshot_in_two_posts_is_one_object_under_one_name() {
        let (mut inbox, mut threads, dir) = fixme_takes_pngs("file-twice");
        threads.transport_mut().serve_file(SHOT, PNG);
        let first = files_of(inbox.deliver_to(post_with("Ev1", "C2", &[(10, SHOT)]), &mut threads).unwrap());
        let mut second = post_with("Ev2", "C2", &[(10, SHOT)]);
        if let Event::Message { ts, .. } = &mut second {
            *ts = "1700000300.000100".to_owned();
        }
        let second = files_of(inbox.deliver_to(second, &mut threads).unwrap());
        assert_eq!(first, second);
        assert_eq!(objects_in(&dir).len(), 1);
    }

    #[test]
    fn a_file_the_store_cannot_keep_does_not_lose_the_post() {
        let (mut inbox, mut threads, dir) = fixme_takes_pngs("file-store-fails");
        let other = "https://files.slack.com/files-pri/T1-F2/second.png";
        let second = [PNG, b" another"].concat();
        threads.transport_mut().serve_file(SHOT, PNG);
        threads.transport_mut().serve_file(other, &second);
        // The second file's fan-out directory is taken by a plain file, as a full or broken
        // disk would refuse it, so the store can keep the first file and not the second.
        let hex = ikigai_core::ContentId::of(&second).to_string();
        fs::write(dir.join("objects").join(&hex[3..5]), b"not a directory").unwrap();
        let event = post_with("Ev1", "C2", &[(10, SHOT), (10, other)]);
        let delivered = inbox.deliver_to(event, &mut threads);
        let named = Files::open(&dir).unwrap().list("fixme").unwrap();
        assert!(
            matches!(delivered, Ok(Some(_))),
            "the post was lost ({delivered:?}), and the files it kept are named by no tuple: {named:?}"
        );
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
