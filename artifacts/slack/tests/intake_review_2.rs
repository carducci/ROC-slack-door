//! Review 2 of PR 140 (ledger item 43, task 3.1 of `channel-files`): taking in a speaker's file.
//!
//! `keep` puts the bytes in the object store, then names them in the channel's index. When the
//! index can't be written, the file is noted `file-unavailable` on the tuple, but its object
//! stays: no channel names it, and no tuple carries its address, so the retention chore can
//! never find it to delete. A speaker's screenshot outlives every rule meant to remove it.

use std::fs;
use std::path::{Path, PathBuf};

use roc_slack_door::{Accepts, Delivery, Event, FakeSlack, Inbox, SlackFile, Threads};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR a screenshot";
const SHOT: &str = "https://files.slack.com/files-pri/T1-F1/screenshot.png";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("intake-review-2-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
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
fn a_file_noted_unavailable_leaves_no_object_that_nothing_names() {
    let dir = scratch("orphan");
    let named = [("fixme".to_owned(), "C2".to_owned())];
    let mut threads = Threads::open_channels(&dir, &named, FakeSlack::new()).unwrap();
    threads.transport_mut().serve_file(SHOT, PNG);
    let channels = [("fixme".to_owned(), "C2".to_owned(), vec!["U2".to_owned()])];
    let mut inbox = Inbox::open_channels(&dir, "UBOT", &channels).unwrap();
    inbox.accept_files("fixme", Accepts { types: vec!["image/png".into()], max_bytes: 100 });
    // The channel's index can't be read, as a damaged or half-written disk would leave it.
    fs::write(dir.join("files/fixme.tsv"), "not an index line\n").unwrap();

    let event = Event::Message {
        id: "Ev1".to_owned(),
        channel: "C2".to_owned(),
        user: "U2".to_owned(),
        ts: "1700000200.000100".to_owned(),
        thread: None,
        text: "cosmetic: the save button overlaps the footer".to_owned(),
        files: vec![SlackFile {
            id: "F0SHOT0001".to_owned(),
            size: PNG.len() as u64,
            url_private: SHOT.to_owned(),
        }],
    };
    let delivered = inbox.deliver_to(event, &mut threads).unwrap();
    let Some((_, Delivery::Message { files, .. })) = delivered else {
        panic!("the post did not land: {delivered:?}");
    };
    assert_eq!(files, vec!["file-unavailable".to_owned()]);
    assert!(
        objects_in(&dir).is_empty(),
        "a file noted unavailable left its bytes stored, named by no channel and no tuple: {:?}",
        objects_in(&dir)
    );
}
