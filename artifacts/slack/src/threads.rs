//! Threads: a caller's key mapped to the conversation it opened, kept durably in the state
//! directory so the key names the same thread after the door restarts.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::{
    check_key, check_reaction, check_text, is_slack_ts, Entry, Grants, Refusal, SlackTransport,
    TransportError, POST, READ, REACT,
};

/// The map's file in the state directory: one thread per line, `key TAB channel TAB ts`.
const MAP_FILE: &str = "threads.tsv";

/// A conversation the door has opened: the channel it is in and its opening message's timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub channel: String,
    pub ts: String,
}

/// Why a write to a thread did not happen.
#[derive(Debug)]
pub enum ThreadError {
    /// The key is not a valid thread key; nothing was posted.
    Refused(Refusal),
    /// Slack would not post the text, for a reason the caller must fix (a token or a permission).
    Transport(TransportError),
    /// Slack is throttling the door or cannot be reached; retry, after `retry_after_secs` when
    /// Slack gave one.
    Unavailable { retry_after_secs: Option<u64> },
    /// The map could not be read or saved, or Slack answered a timestamp the map cannot hold.
    State(io::Error),
}

/// Slack's transient failures are unavailable, carrying its retry-after; anything else is an
/// error that names what Slack refused.
fn from_transport(error: TransportError) -> ThreadError {
    match error {
        TransportError::RateLimited { retry_after_secs } => ThreadError::Unavailable {
            retry_after_secs: Some(retry_after_secs),
        },
        TransportError::Unavailable(_) => ThreadError::Unavailable {
            retry_after_secs: None,
        },
        rejected @ TransportError::Rejected(_) => ThreadError::Transport(rejected),
    }
}

/// The threads a door holds, over one transport and one configured channel.
pub struct Threads<T: SlackTransport> {
    state_dir: PathBuf,
    /// The declared channels as (name, Slack id); the first is the one the unscoped calls use.
    channels: Vec<(String, String)>,
    transport: T,
    /// Keyed by channel id and then key, so a key is unique within its channel.
    threads: BTreeMap<(String, String), Thread>,
}

impl<T: SlackTransport> Threads<T> {
    /// Opens the map in `state_dir`, creating the directory and an empty map when there is none.
    pub fn open(state_dir: &Path, channel: &str, transport: T) -> io::Result<Self> {
        Self::open_channels(state_dir, &[(String::new(), channel.to_owned())], transport)
    }

    /// Opens the map for the declared `channels`, each a (name, Slack id) pair.
    pub fn open_channels(
        state_dir: &Path,
        channels: &[(String, String)],
        transport: T,
    ) -> io::Result<Self> {
        if channels.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "no channel declared"));
        }
        fs::create_dir_all(state_dir)?;
        let threads = match fs::read_to_string(state_dir.join(MAP_FILE)) {
            Ok(text) => parse(&text)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            state_dir: state_dir.to_owned(),
            channels: channels.to_vec(),
            transport,
            threads,
        })
    }

    /// The Slack id of the declared channel `name`, or `no-channel`.
    fn id_of(&self, name: &str) -> Result<String, ThreadError> {
        self.channels
            .iter()
            .find(|(declared, _)| declared == name)
            .map(|(_, id)| id.clone())
            .ok_or_else(|| ThreadError::Refused(Refusal::NoChannel(name.to_owned())))
    }

    /// The id of the channel the unscoped calls use.
    fn default_id(&self) -> String {
        self.channels[0].1.clone()
    }

    /// `write` in the declared channel `channel`.
    pub fn write_in(
        &mut self,
        grants: &Grants,
        channel: &str,
        key: &str,
        text: &str,
    ) -> Result<(), ThreadError> {
        let id = self.id_of(channel)?;
        self.write_at(grants, &id, key, text)
    }

    /// Posts `text` as a new top-level message in the declared channel `channel`, in no thread.
    pub fn announce(&mut self, grants: &Grants, channel: &str, text: &str) -> Result<(), ThreadError> {
        let id = self.id_of(channel)?;
        grants.require(POST).map_err(ThreadError::Refused)?;
        check_text(text).map_err(ThreadError::Refused)?;
        self.transport.post(&id, None, text).map_err(from_transport)?;
        Ok(())
    }

    /// `read` in the declared channel `channel`.
    pub fn read_in(
        &mut self,
        grants: &Grants,
        channel: &str,
        key: &str,
    ) -> Result<Vec<Entry>, ThreadError> {
        let id = self.id_of(channel)?;
        self.read_at(grants, &id, key)
    }

    /// Writes `text` to the thread `key` names. A key with no thread opens one as a new top-level
    /// message and records it; a key with a thread replies in it.
    pub fn write(&mut self, grants: &Grants, key: &str, text: &str) -> Result<(), ThreadError> {
        let id = self.default_id();
        self.write_at(grants, &id, key, text)
    }

    fn write_at(
        &mut self,
        grants: &Grants,
        channel: &str,
        key: &str,
        text: &str,
    ) -> Result<(), ThreadError> {
        grants.require(POST).map_err(ThreadError::Refused)?;
        check_key(key).map_err(ThreadError::Refused)?;
        check_text(text).map_err(ThreadError::Refused)?;
        let parent = self.threads.get(&slot(channel, key)).map(|thread| thread.ts.clone());
        let ts = self
            .transport
            .post(channel, parent.as_deref(), text)
            .map_err(from_transport)?;
        if parent.is_none() {
            // The map holds only timestamps of Slack's shape: anything else is refused before it
            // can reach a line of threads.tsv, and the key stays free for a retry.
            if !is_slack_ts(&ts) {
                return Err(ThreadError::State(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Slack answered a timestamp of another shape: {ts:?}"),
                )));
            }
            let thread = Thread {
                channel: channel.to_owned(),
                ts,
            };
            self.record(key, thread).map_err(ThreadError::State)?;
        }
        Ok(())
    }

    /// Records `thread` under `key` in its channel and saves the map. The entry is kept only once
    /// the map is saved: a save that fails leaves the map, in memory and on disk, as it was,
    /// a thread the slot already held included. A thread that cannot be one line of threads.tsv
    /// is refused before anything changes, on every route: `write`'s and `adopt`'s alike.
    fn record(&mut self, key: &str, thread: Thread) -> io::Result<()> {
        check_line(&thread)?;
        let slot = slot(&thread.channel, key);
        let held = self.threads.insert(slot.clone(), thread);
        if let Err(error) = self.save() {
            match held {
                Some(held) => self.threads.insert(slot, held),
                None => self.threads.remove(&slot),
            };
            return Err(error);
        }
        Ok(())
    }

    /// Adds the reaction `name` to the opening message of the thread `key` names, or removes it
    /// when `add` is false. A key with no thread is refused before Slack is called.
    pub fn react(
        &mut self,
        grants: &Grants,
        key: &str,
        name: &str,
        add: bool,
    ) -> Result<(), ThreadError> {
        let id = self.default_id();
        self.react_at(grants, &id, key, name, add)
    }

    /// `react` in the declared channel `channel`.
    pub fn react_in(
        &mut self,
        grants: &Grants,
        channel: &str,
        key: &str,
        name: &str,
        add: bool,
    ) -> Result<(), ThreadError> {
        let id = self.id_of(channel)?;
        self.react_at(grants, &id, key, name, add)
    }

    fn react_at(
        &mut self,
        grants: &Grants,
        channel: &str,
        key: &str,
        name: &str,
        add: bool,
    ) -> Result<(), ThreadError> {
        grants.require(REACT).map_err(ThreadError::Refused)?;
        check_key(key).map_err(ThreadError::Refused)?;
        check_reaction(name).map_err(ThreadError::Refused)?;
        let thread = self
            .threads
            .get(&slot(channel, key))
            .ok_or(ThreadError::Refused(Refusal::NoThread))?;
        self.transport
            .react(&thread.channel, &thread.ts, name, add)
            .map_err(from_transport)
    }

    /// The key of the thread in channel id `channel` whose opening message has timestamp `ts`, if
    /// the door opened one: timestamps are unique only within a channel.
    pub fn key_for(&self, channel: &str, ts: &str) -> Option<String> {
        self.threads
            .iter()
            .find(|(_, thread)| thread.channel == channel && thread.ts == ts)
            .map(|((_, key), _)| key.clone())
    }

    /// Records the message `ts` in the channel with Slack id `channel` as the opening message of
    /// the thread `key`, as if the door had posted it, and saves the map. Answers whether the key
    /// now names that message: false, with nothing recorded, when the key is already in use in
    /// that channel for a different message. The entry is kept only once the map is saved.
    pub fn adopt(&mut self, channel: &str, key: &str, ts: &str) -> io::Result<bool> {
        if let Some(held) = self.threads.get(&slot(channel, key)) {
            return Ok(held.ts == ts);
        }
        let thread = Thread {
            channel: channel.to_owned(),
            ts: ts.to_owned(),
        };
        self.record(key, thread)?;
        Ok(true)
    }

    /// Whether the key names a thread the door has opened.
    pub fn exists(&self, grants: &Grants, key: &str) -> Result<bool, ThreadError> {
        grants.require(READ).map_err(ThreadError::Refused)?;
        check_key(key).map_err(ThreadError::Refused)?;
        Ok(self.threads.contains_key(&slot(&self.default_id(), key)))
    }

    /// `exists` in the declared channel `channel`.
    pub fn exists_in(&self, grants: &Grants, channel: &str, key: &str) -> Result<bool, ThreadError> {
        let id = self.id_of(channel)?;
        grants.require(READ).map_err(ThreadError::Refused)?;
        check_key(key).map_err(ThreadError::Refused)?;
        Ok(self.threads.contains_key(&slot(&id, key)))
    }

    /// The conversation the key names, the opening message first.
    pub fn read(&mut self, grants: &Grants, key: &str) -> Result<Vec<Entry>, ThreadError> {
        let id = self.default_id();
        self.read_at(grants, &id, key)
    }

    fn read_at(&mut self, grants: &Grants, channel: &str, key: &str) -> Result<Vec<Entry>, ThreadError> {
        grants.require(READ).map_err(ThreadError::Refused)?;
        check_key(key).map_err(ThreadError::Refused)?;
        let thread = self
            .threads
            .get(&slot(channel, key))
            .ok_or(ThreadError::Refused(Refusal::NoThread))?;
        let messages = self
            .transport
            .replies(&thread.channel, &thread.ts)
            .map_err(from_transport)?;
        Ok(messages.iter().map(Entry::from).collect())
    }

    /// The transport the threads post through.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The transport, mutably: a test feeds it the events a person would have caused.
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Writes the whole map to a temporary file and renames it over the old one, so a crash
    /// leaves either the old map or the new one.
    fn save(&self) -> io::Result<()> {
        let tmp = self.state_dir.join("threads.tsv.tmp");
        let mut file = fs::File::create(&tmp)?;
        for ((_, key), thread) in &self.threads {
            writeln!(file, "{key}\t{}\t{}", thread.channel, thread.ts)?;
        }
        file.sync_all()?;
        fs::rename(tmp, self.state_dir.join(MAP_FILE))
    }
}

/// The map's index for a key in a channel.
fn slot(channel: &str, key: &str) -> (String, String) {
    (channel.to_owned(), key.to_owned())
}

/// Refuses a thread that cannot be one line of threads.tsv and read back the same: its
/// timestamp must be of Slack's shape, and its channel id must hold no tab or newline.
fn check_line(thread: &Thread) -> io::Result<()> {
    if !is_slack_ts(&thread.ts) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a timestamp of another shape: {:?}", thread.ts),
        ));
    }
    if thread.channel.contains(['\t', '\n']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a channel id with a tab or newline: {:?}", thread.channel),
        ));
    }
    Ok(())
}

fn parse(text: &str) -> io::Result<BTreeMap<(String, String), Thread>> {
    text.lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                // The line's timestamp is held to the shape record holds a new one to.
                [key, channel, ts] if is_slack_ts(ts) => Ok((
                    slot(channel, key),
                    Thread {
                        channel: (*channel).to_owned(),
                        ts: (*ts).to_owned(),
                    },
                )),
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("bad line in {MAP_FILE}: {line:?}"),
                )),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Event, FakeSlack, MAX_MESSAGE_LEN};

    /// A fresh, empty state directory for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roc-slack-door-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn all() -> Grants {
        Grants::all()
    }

    #[test]
    fn key_for_tells_equal_timestamps_in_two_channels_apart() {
        let dir = scratch("key-for");
        let channels = [("a".to_owned(), "C1".to_owned()), ("b".to_owned(), "C2".to_owned())];
        let mut threads = Threads::open_channels(&dir, &channels, FakeSlack::new()).unwrap();
        for (channel, key) in [("C1", "one"), ("C2", "two")] {
            let thread = Thread { channel: channel.to_owned(), ts: "1.0".to_owned() };
            threads.threads.insert(slot(channel, key), thread);
        }
        assert_eq!(threads.key_for("C1", "1.0").as_deref(), Some("one"));
        assert_eq!(threads.key_for("C2", "1.0").as_deref(), Some("two"));
    }

    #[test]
    fn open_channels_refuses_an_empty_channel_list() {
        let result = Threads::open_channels(&scratch("no-channels"), &[], FakeSlack::new());
        assert_eq!(result.err().map(|e| e.kind()), Some(io::ErrorKind::InvalidInput));
    }

    #[test]
    fn a_first_write_opens_a_top_level_thread() {
        let dir = scratch("first-write");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write(&all(), "carducci-54", "Blocked on a token").unwrap();
        let posts = threads.transport().posts();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].thread, None);
        assert_eq!(posts[0].text, "Blocked on a token");
    }

    #[test]
    fn a_later_write_replies_in_the_thread() {
        let dir = scratch("reply");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write(&all(), "carducci-54", "Opening").unwrap();
        threads.write(&all(), "carducci-54", "Here is the command").unwrap();
        let posts = threads.transport().posts();
        assert_eq!(posts[1].thread.as_deref(), Some(posts[0].ts.as_str()));
    }

    #[test]
    fn the_thread_survives_a_restart() {
        let dir = scratch("restart");
        let mut before = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        before.write(&all(), "carducci-54", "Opening").unwrap();
        let opening = before.transport().posts()[0].ts.clone();
        drop(before);

        let mut after = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        after.write(&all(), "carducci-54", "Still blocked").unwrap();
        assert_eq!(after.transport().posts()[0].thread.as_deref(), Some(opening.as_str()));
    }

    #[test]
    fn empty_and_whitespace_text_is_refused_and_nothing_is_posted() {
        let dir = scratch("empty-text");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        let err = threads.write(&all(), "carducci-54", "  \n\t ").unwrap_err();
        assert!(matches!(err, ThreadError::Refused(Refusal::EmptyMessage)));
        assert!(threads.transport().posts().is_empty());
    }

    #[test]
    fn text_over_the_limit_is_refused_and_nothing_is_posted() {
        let dir = scratch("long-text");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        let too_long = "x".repeat(MAX_MESSAGE_LEN + 1);
        let err = threads.write(&all(), "carducci-54", &too_long).unwrap_err();
        assert!(matches!(err, ThreadError::Refused(Refusal::MessageTooLong)));
        assert!(threads.transport().posts().is_empty());
    }

    #[test]
    fn text_at_the_limit_is_accepted() {
        let dir = scratch("limit-text");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write(&all(), "carducci-54", &"x".repeat(MAX_MESSAGE_LEN)).unwrap();
        assert_eq!(threads.transport().posts().len(), 1);
    }

    #[test]
    fn reading_a_thread_returns_the_opening_message_then_the_replies() {
        let dir = scratch("read");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write(&all(), "carducci-54", "Blocked on a token").unwrap();
        let root = threads.transport().posts()[0].ts.clone();
        threads.transport_mut().feed(Event::Message {
            id: "Ev1".to_owned(),
            channel: "C1".to_owned(),
            user: "U1".to_owned(),
            ts: "1700000100.000001".to_owned(),
            thread: Some(root),
            text: "Which token?".to_owned(),
        });
        let entries = threads.read(&all(), "carducci-54").unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].bot && entries[0].text == "Blocked on a token");
        assert!(!entries[1].bot && entries[1].user == "U1" && entries[1].text == "Which token?");
    }

    #[test]
    fn the_json_reading_has_no_channel_or_message_ids() {
        let dir = scratch("read-json");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write(&all(), "carducci-54", "Blocked on a token").unwrap();
        let root = threads.transport().posts()[0].ts.clone();
        let json = crate::to_json(&threads.read(&all(), "carducci-54").unwrap());
        assert!(json.contains("\"bot\":true"));
        assert!(!json.contains("C1"));
        assert!(!json.contains(&root));
    }

    #[test]
    fn a_rate_limit_is_unavailable_with_slacks_retry_after() {
        let dir = scratch("rate-limit");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.transport_mut().fail_next(TransportError::RateLimited { retry_after_secs: 30 });
        let err = threads.write(&all(), "carducci-54", "Opening").unwrap_err();
        assert!(matches!(err, ThreadError::Unavailable { retry_after_secs: Some(30) }));
        assert!(threads.transport().posts().is_empty());
    }

    #[test]
    fn an_outage_is_unavailable_without_a_retry_after() {
        let dir = scratch("outage");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write(&all(), "carducci-54", "Opening").unwrap();
        threads.transport_mut().fail_next(TransportError::Unavailable("503".to_owned()));
        let err = threads.read(&all(), "carducci-54").unwrap_err();
        assert!(matches!(err, ThreadError::Unavailable { retry_after_secs: None }));
    }

    #[test]
    fn a_rejected_token_is_an_error_that_names_what_slack_said() {
        let dir = scratch("rejected");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.transport_mut().fail_next(TransportError::Rejected("invalid_auth".to_owned()));
        let err = threads.write(&all(), "carducci-54", "Opening").unwrap_err();
        assert!(matches!(
            err,
            ThreadError::Transport(TransportError::Rejected(ref why)) if why == "invalid_auth"
        ));
    }

    #[test]
    fn a_reaction_marks_the_opening_message_and_can_be_removed() {
        let dir = scratch("react");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write(&all(), "carducci-54", "Opening").unwrap();
        let root = threads.transport().posts()[0].ts.clone();
        threads.react(&all(), "carducci-54", "white_check_mark", true).unwrap();
        threads.react(&all(), "carducci-54", "white_check_mark", true).unwrap();
        assert_eq!(threads.transport().reactions().len(), 1);
        assert!(threads.transport().reactions().contains(&(root, "white_check_mark".to_owned())));
        threads.react(&all(), "carducci-54", "white_check_mark", false).unwrap();
        assert!(threads.transport().reactions().is_empty());
    }

    #[test]
    fn a_reaction_on_a_key_with_no_thread_is_refused_and_nothing_is_added() {
        let dir = scratch("react-none");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        let err = threads.react(&all(), "never-opened", "white_check_mark", true).unwrap_err();
        assert!(matches!(err, ThreadError::Refused(Refusal::NoThread)));
        assert!(threads.transport().reactions().is_empty());
    }

    #[test]
    fn a_reaction_with_a_bad_name_is_refused_and_nothing_is_added() {
        let dir = scratch("react-bad-name");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write(&all(), "carducci-54", "Opening").unwrap();
        for name in ["White Check", "tada!", "", "Tada"] {
            let err = threads.react(&all(), "carducci-54", name, true).unwrap_err();
            assert!(matches!(err, ThreadError::Refused(Refusal::BadReaction)));
        }
        assert!(threads.transport().reactions().is_empty());
    }

    #[test]
    fn a_key_exists_once_its_thread_is_opened() {
        let dir = scratch("exists");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        assert!(!threads.exists(&all(), "carducci-54").unwrap());
        threads.write(&all(), "carducci-54", "Opening").unwrap();
        assert!(threads.exists(&all(), "carducci-54").unwrap());
        assert!(matches!(
            threads.exists(&all(), "item 54"),
            Err(ThreadError::Refused(Refusal::BadKey))
        ));
    }

    #[test]
    fn reading_a_key_with_no_thread_is_refused() {
        let dir = scratch("read-none");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        let err = threads.read(&all(), "never-opened").unwrap_err();
        assert!(matches!(err, ThreadError::Refused(Refusal::NoThread)));
    }

    #[test]
    fn a_bad_key_is_refused_and_nothing_is_posted() {
        let dir = scratch("bad-key");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        let err = threads.write(&all(), "item 54", "Opening").unwrap_err();
        assert!(matches!(err, ThreadError::Refused(Refusal::BadKey)));
        assert!(threads.transport().posts().is_empty());
    }

    #[test]
    fn posting_without_the_post_capability_is_refused_and_nothing_is_posted() {
        let dir = scratch("no-post");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        let read_only = Grants::new([READ]);
        let err = threads.write(&read_only, "carducci-54", "Opening").unwrap_err();
        assert!(matches!(err, ThreadError::Refused(Refusal::NotGranted(POST))));
        assert!(threads.transport().posts().is_empty());
    }

    #[test]
    fn reacting_without_the_react_capability_is_refused_and_nothing_is_added() {
        let dir = scratch("no-react");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write(&all(), "carducci-54", "Opening").unwrap();
        let read_only = Grants::new([READ]);
        let err = threads.react(&read_only, "carducci-54", "tada", true).unwrap_err();
        assert!(matches!(err, ThreadError::Refused(Refusal::NotGranted(REACT))));
        assert!(threads.transport().reactions().is_empty());
    }

    /// A state directory holding a `threads.tsv` in the format from before channels were named.
    fn legacy_dir(name: &str, lines: &str) -> PathBuf {
        let dir = scratch(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(MAP_FILE), lines).unwrap();
        dir
    }

    fn declared() -> [(String, String); 2] {
        [("gonk-chat".to_owned(), "C1".to_owned()), ("ops".to_owned(), "C2".to_owned())]
    }

    #[test]
    fn an_old_line_for_a_declared_channel_is_served_under_its_name() {
        // The opening message is already in Slack, so the reply gets a ts of its own.
        let mut slack = FakeSlack::new();
        let root = slack.post("C2", None, "Opening").unwrap();
        let dir = legacy_dir("legacy-served", &format!("carducci-54\tC2\t{root}\n"));
        let mut threads = Threads::open_channels(&dir, &declared(), slack).unwrap();
        assert!(threads.exists_in(&all(), "ops", "carducci-54").unwrap());
        threads.write_in(&all(), "ops", "carducci-54", "Back again").unwrap();
        let post = &threads.transport().posts()[1];
        assert_eq!(post.thread.as_deref(), Some(root.as_str()));
        assert_ne!(post.ts, root);
        threads.react_in(&all(), "ops", "carducci-54", "tada", true).unwrap();
        assert!(threads.transport().reactions().contains(&(root.clone(), "tada".to_owned())));
        assert!(threads.read_in(&all(), "ops", "carducci-54").is_ok());
        assert!(!threads.exists_in(&all(), "gonk-chat", "carducci-54").unwrap());
    }

    #[test]
    fn an_old_line_for_an_undeclared_channel_is_kept_and_served_by_no_name() {
        let line = "carducci-54\tC9\t1700000000.000001\n";
        let dir = legacy_dir("legacy-undeclared", line);
        let mut threads = Threads::open_channels(&dir, &declared(), FakeSlack::new()).unwrap();
        for name in ["gonk-chat", "ops"] {
            assert!(!threads.exists_in(&all(), name, "carducci-54").unwrap());
        }
        assert!(matches!(
            threads.read_in(&all(), "C9", "carducci-54"),
            Err(ThreadError::Refused(Refusal::NoChannel(_)))
        ));
        threads.write_in(&all(), "ops", "other", "Opening").unwrap();
        drop(threads);
        let saved = fs::read_to_string(dir.join(MAP_FILE)).unwrap();
        assert!(saved.lines().any(|l| l == line.trim_end()));
    }

    #[test]
    fn the_old_file_is_unchanged_after_a_restart() {
        let text = "b\tC9\t1.0\na\tC1\t2.0\nc\tC2\t3.0\n";
        let dir = legacy_dir("legacy-restart", text);
        drop(Threads::open_channels(&dir, &declared(), FakeSlack::new()).unwrap());
        drop(Threads::open_channels(&dir, &declared(), FakeSlack::new()).unwrap());
        assert_eq!(fs::read_to_string(dir.join(MAP_FILE)).unwrap(), text);
    }

    #[test]
    fn an_answered_timestamp_of_another_shape_is_refused_and_no_thread_is_kept() {
        let dir = scratch("bad-answer");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write(&all(), "other", "Opening").unwrap();
        let saved = fs::read_to_string(dir.join(MAP_FILE)).unwrap();
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
        for bad in bad_timestamps {
            threads.transport_mut().answer_next_post(bad);
            let err = threads.write(&all(), "carducci-54", "Opening").unwrap_err();
            assert!(
                matches!(err, ThreadError::State(ref why) if why.kind() == io::ErrorKind::InvalidData),
                "{bad:?}: {err:?}"
            );
            assert!(!threads.exists(&all(), "carducci-54").unwrap(), "{bad:?}");
            assert_eq!(fs::read_to_string(dir.join(MAP_FILE)).unwrap(), saved, "{bad:?}");
        }
        drop(threads);
        let after = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        assert!(after.exists(&all(), "other").unwrap());
        assert!(!after.exists(&all(), "carducci-54").unwrap());
    }

    #[test]
    fn a_thread_whose_save_failed_is_not_kept_and_a_retry_opens_it() {
        let dir = scratch("save-fails");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        let blocker = dir.join("threads.tsv.tmp");
        fs::create_dir(&blocker).unwrap();
        let err = threads.write(&all(), "carducci-54", "Opening").unwrap_err();
        assert!(matches!(err, ThreadError::State(_)), "{err:?}");
        assert!(!threads.exists(&all(), "carducci-54").unwrap());
        assert!(!dir.join(MAP_FILE).exists());

        fs::remove_dir(&blocker).unwrap();
        threads.write(&all(), "carducci-54", "Opening again").unwrap();
        assert!(threads.exists(&all(), "carducci-54").unwrap());
        // A new top-level message, not a reply in a thread the map never held.
        assert_eq!(threads.transport().posts()[1].thread, None);
        drop(threads);
        let after = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        assert!(after.exists(&all(), "carducci-54").unwrap());
    }

    #[test]
    fn a_failed_save_leaves_a_held_thread_as_it_was() {
        let dir = scratch("save-fails-held");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write(&all(), "carducci-54", "Opening").unwrap();
        let held = threads.threads.get(&slot("C1", "carducci-54")).unwrap().clone();
        let saved = fs::read_to_string(dir.join(MAP_FILE)).unwrap();
        fs::create_dir(dir.join("threads.tsv.tmp")).unwrap();
        let other = Thread { channel: "C1".to_owned(), ts: "9.9".to_owned() };
        threads.record("carducci-54", other).unwrap_err();
        assert_eq!(threads.threads.get(&slot("C1", "carducci-54")), Some(&held));
        assert_eq!(fs::read_to_string(dir.join(MAP_FILE)).unwrap(), saved);
    }

    #[test]
    fn an_adopted_timestamp_of_another_shape_is_refused_and_the_map_still_reopens() {
        let dir = scratch("bad-adopt");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write(&all(), "other", "Opening").unwrap();
        let saved = fs::read_to_string(dir.join(MAP_FILE)).unwrap();
        for bad in ["1700000100.000001\tC1\tx", "1700000100.0\nevil", "abc", ""] {
            let err = threads.adopt("C1", "k", bad).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{bad:?}");
            assert!(!threads.exists(&all(), "k").unwrap(), "{bad:?}");
            assert_eq!(fs::read_to_string(dir.join(MAP_FILE)).unwrap(), saved, "{bad:?}");
        }
        drop(threads);
        let after = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        assert!(after.exists(&all(), "other").unwrap());
    }

    #[test]
    fn a_channel_id_with_a_tab_or_newline_is_refused_before_the_map() {
        for (n, bad) in ["C1\tx", "C1\nx"].iter().enumerate() {
            let dir = scratch(&format!("bad-channel-{n}"));
            let mut threads = Threads::open(&dir, bad, FakeSlack::new()).unwrap();
            let err = threads.write(&all(), "carducci-54", "Opening").unwrap_err();
            assert!(
                matches!(err, ThreadError::State(ref why) if why.kind() == io::ErrorKind::InvalidData),
                "{bad:?}: {err:?}"
            );
            let err = threads.adopt(bad, "k", "1700000100.000001").unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{bad:?}");
            assert!(!dir.join(MAP_FILE).exists(), "{bad:?}");
        }
    }

    #[test]
    fn a_map_line_whose_timestamp_is_of_another_shape_is_refused_on_open() {
        for (n, bad) in ["abc", "1.2.3", "", "1700000100.000001 "].iter().enumerate() {
            let dir = legacy_dir(&format!("bad-line-{n}"), &format!("k\tC1\t{bad}\n"));
            let err = Threads::open(&dir, "C1", FakeSlack::new()).err().unwrap();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{bad:?}");
        }
    }

    #[test]
    fn reading_without_the_read_capability_is_refused() {
        let dir = scratch("no-read");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write(&all(), "carducci-54", "Opening").unwrap();
        let post_only = Grants::new([POST]);
        let err = threads.read(&post_only, "carducci-54").unwrap_err();
        assert!(matches!(err, ThreadError::Refused(Refusal::NotGranted(READ))));
    }
}
