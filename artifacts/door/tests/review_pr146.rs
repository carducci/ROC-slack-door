//! Review 1 of PR 146 (ledger item 45, task 3.3 of `channel-files`): deleting a file.
//!
//! The file index says "Callers serialize their use of one state directory". The door's intake
//! keeps a file (`objects.put`, then `files.add`) while it holds the thread map's lock; the new
//! delete runs on the socket's thread without it, so it can land between the two. And "Deleting
//! a name that isn't there SHALL succeed and change nothing": a delete of a name this channel
//! never had still removes bytes no index names.

#[path = "../src/serve.rs"]
#[allow(dead_code)]
mod serve;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ikigai_core::{Capability, ContentId, Error, Iri, Kernel, Request, Verb};
use roc_slack_door::{scope, FakeSlack, FileEntry, Files, Objects, Threads};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nreview-146";

type Shared = Arc<Mutex<Threads<FakeSlack>>>;

fn door(name: &str) -> (Kernel, Shared, PathBuf) {
    let dir = std::env::temp_dir().join(format!("review-pr146-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let channels = [("gonk-chat".to_owned(), "C1".to_owned()), ("fixme".to_owned(), "C2".to_owned())];
    let threads = Arc::new(Mutex::new(Threads::open_channels(&dir, &channels, FakeSlack::new()).unwrap()));
    let declared = vec!["gonk-chat".to_owned(), "fixme".to_owned()];
    (serve::kernel(Arc::clone(&threads), dir.join("spaces"), declared), threads, dir)
}

fn entry(id: &ContentId) -> FileEntry {
    FileEntry { id: id.clone(), content_type: "image/png".to_owned(), name: "F1".to_owned() }
}

fn issue(kernel: &Kernel, verb: Verb, iri: &str, channel: &str, action: &str) -> ikigai_core::Result<Vec<u8>> {
    let request = Request::new(verb, Iri::parse(iri).unwrap());
    let capability = Capability::scoped([scope(channel, action)]);
    futures_executor::block_on(kernel.issue(request, &capability)).map(|r| r.bytes)
}

/// `gonk-chat` named these bytes once; `fixme` is receiving the same bytes now. The retention
/// chore deletes gonk-chat's name while fixme's intake holds the lock between keeping the bytes
/// and naming them. Serialized, the delete sees fixme's name and keeps the object; unserialized,
/// it removes the object and fixme's new name points at nothing.
#[test]
fn a_delete_does_not_land_between_intake_keeping_bytes_and_naming_them() {
    let (kernel, threads, dir) = door("race");
    let objects = Objects::open(&dir).unwrap();
    let files = Files::open(&dir).unwrap();
    let id = objects.put(PNG).unwrap();
    files.add("gonk-chat", entry(&id)).unwrap();
    let kernel = Arc::new(kernel);

    let intake = threads.lock().unwrap();
    // Intake, step 1: the bytes are kept (already stored, so nothing is written).
    objects.put(PNG).unwrap();
    let deleting = {
        let kernel = Arc::clone(&kernel);
        let iri = format!("urn:slack:channel:gonk-chat:file:{id}");
        std::thread::spawn(move || issue(&kernel, Verb::Delete, &iri, "gonk-chat", "file:delete"))
    };
    std::thread::sleep(Duration::from_millis(300));
    // Intake, step 2: fixme names them.
    files.add("fixme", entry(&id)).unwrap();
    drop(intake);
    // Whether the delete succeeded is not the point; fixme's file must survive it.
    let _ = deleting.join().unwrap();

    let read = issue(&kernel, Verb::Source, &format!("urn:slack:channel:fixme:file:{id}"), "fixme", "file:read");
    assert!(matches!(&read, Ok(bytes) if bytes == PNG), "fixme's file is lost: {read:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Bytes no index names (an intake that stopped between keeping and naming them) are not this
/// channel's name, so deleting `gonk-chat`'s name for them changes nothing.
#[test]
fn deleting_a_name_the_channel_never_had_changes_nothing() {
    let (kernel, _threads, dir) = door("absent");
    let objects = Objects::open(&dir).unwrap();
    let id = objects.put(PNG).unwrap();
    let iri = format!("urn:slack:channel:gonk-chat:file:{id}");
    let deleted = issue(&kernel, Verb::Delete, &iri, "gonk-chat", "file:delete");
    assert!(!matches!(deleted, Err(Error::Denied(_))), "{deleted:?}");
    assert_eq!(objects.get(&id).unwrap().as_deref(), Some(PNG), "a delete of a name that isn't there removed the bytes");
    let _ = std::fs::remove_dir_all(&dir);
}
