//! Threads: a caller's key mapped to the conversation it opened, kept durably in the state
//! directory so the key names the same thread after the door restarts.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::{check_key, check_text, Refusal, SlackTransport, TransportError};

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
    /// Slack would not post the text.
    Transport(TransportError),
    /// The map could not be read or saved.
    State(io::Error),
}

/// The threads a door holds, over one transport and one configured channel.
pub struct Threads<T: SlackTransport> {
    state_dir: PathBuf,
    channel: String,
    transport: T,
    threads: BTreeMap<String, Thread>,
}

impl<T: SlackTransport> Threads<T> {
    /// Opens the map in `state_dir`, creating the directory and an empty map when there is none.
    pub fn open(state_dir: &Path, channel: &str, transport: T) -> io::Result<Self> {
        fs::create_dir_all(state_dir)?;
        let threads = match fs::read_to_string(state_dir.join(MAP_FILE)) {
            Ok(text) => parse(&text)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            state_dir: state_dir.to_owned(),
            channel: channel.to_owned(),
            transport,
            threads,
        })
    }

    /// Writes `text` to the thread `key` names. A key with no thread opens one as a new top-level
    /// message and records it; a key with a thread replies in it.
    pub fn write(&mut self, key: &str, text: &str) -> Result<(), ThreadError> {
        check_key(key).map_err(ThreadError::Refused)?;
        check_text(text).map_err(ThreadError::Refused)?;
        let parent = self.threads.get(key).map(|thread| thread.ts.clone());
        let ts = self
            .transport
            .post(&self.channel, parent.as_deref(), text)
            .map_err(ThreadError::Transport)?;
        if parent.is_none() {
            let thread = Thread {
                channel: self.channel.clone(),
                ts,
            };
            self.threads.insert(key.to_owned(), thread);
            self.save().map_err(ThreadError::State)?;
        }
        Ok(())
    }

    /// The transport the threads post through.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Writes the whole map to a temporary file and renames it over the old one, so a crash
    /// leaves either the old map or the new one.
    fn save(&self) -> io::Result<()> {
        let tmp = self.state_dir.join("threads.tsv.tmp");
        let mut file = fs::File::create(&tmp)?;
        for (key, thread) in &self.threads {
            writeln!(file, "{key}\t{}\t{}", thread.channel, thread.ts)?;
        }
        file.sync_all()?;
        fs::rename(tmp, self.state_dir.join(MAP_FILE))
    }
}

fn parse(text: &str) -> io::Result<BTreeMap<String, Thread>> {
    text.lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                [key, channel, ts] => Ok((
                    (*key).to_owned(),
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
    use crate::{FakeSlack, MAX_MESSAGE_LEN};

    /// A fresh, empty state directory for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roc-slack-door-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_first_write_opens_a_top_level_thread() {
        let dir = scratch("first-write");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write("carducci-54", "Blocked on a token").unwrap();
        let posts = threads.transport().posts();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].thread, None);
        assert_eq!(posts[0].text, "Blocked on a token");
    }

    #[test]
    fn a_later_write_replies_in_the_thread() {
        let dir = scratch("reply");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write("carducci-54", "Opening").unwrap();
        threads.write("carducci-54", "Here is the command").unwrap();
        let posts = threads.transport().posts();
        assert_eq!(posts[1].thread.as_deref(), Some(posts[0].ts.as_str()));
    }

    #[test]
    fn the_thread_survives_a_restart() {
        let dir = scratch("restart");
        let mut before = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        before.write("carducci-54", "Opening").unwrap();
        let opening = before.transport().posts()[0].ts.clone();
        drop(before);

        let mut after = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        after.write("carducci-54", "Still blocked").unwrap();
        assert_eq!(after.transport().posts()[0].thread.as_deref(), Some(opening.as_str()));
    }

    #[test]
    fn empty_and_whitespace_text_is_refused_and_nothing_is_posted() {
        let dir = scratch("empty-text");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        let err = threads.write("carducci-54", "  \n\t ").unwrap_err();
        assert!(matches!(err, ThreadError::Refused(Refusal::EmptyMessage)));
        assert!(threads.transport().posts().is_empty());
    }

    #[test]
    fn text_over_the_limit_is_refused_and_nothing_is_posted() {
        let dir = scratch("long-text");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        let too_long = "x".repeat(MAX_MESSAGE_LEN + 1);
        let err = threads.write("carducci-54", &too_long).unwrap_err();
        assert!(matches!(err, ThreadError::Refused(Refusal::MessageTooLong)));
        assert!(threads.transport().posts().is_empty());
    }

    #[test]
    fn text_at_the_limit_is_accepted() {
        let dir = scratch("limit-text");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        threads.write("carducci-54", &"x".repeat(MAX_MESSAGE_LEN)).unwrap();
        assert_eq!(threads.transport().posts().len(), 1);
    }

    #[test]
    fn a_bad_key_is_refused_and_nothing_is_posted() {
        let dir = scratch("bad-key");
        let mut threads = Threads::open(&dir, "C1", FakeSlack::new()).unwrap();
        let err = threads.write("item 54", "Opening").unwrap_err();
        assert!(matches!(err, ThreadError::Refused(Refusal::BadKey)));
        assert!(threads.transport().posts().is_empty());
    }
}
