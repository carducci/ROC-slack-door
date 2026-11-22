//! Review 1 of PR 149 (ledger item 46, task 4.1 of `channel-files`): the vectors.
//!
//! Spec `files`, "Retention removes a screenshot": WHEN the retention chore deletes the file
//! from David's post, THEN reading it is refused as `no-file`, and its object is gone. The
//! chore holds only `urn:cap:slack:channel:{name}:file:delete` (design D5) and reaches the door
//! through its kernel. The vector names `deleting_the_only_name_removes_the_object`, a unit test
//! of the library's `Files::delete`, so it is green while the door's file endpoint answers
//! every verb but Source with "takes source, not Delete": the scenario can't be run at all.

#[path = "../src/serve.rs"]
#[allow(dead_code)]
mod serve;

use std::sync::{Arc, Mutex};

use ikigai_core::{Capability, Error, Iri, Request, Verb};
use roc_slack_door::{scope, FakeSlack, FileEntry, Files, Objects, Threads};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nbytes";

#[test]
fn retention_deletes_a_file_through_the_door_with_only_file_delete() {
    let dir = std::env::temp_dir().join(format!("review-pr149-retention-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let channels = [("gonk-chat".to_owned(), "C1".to_owned()), ("fixme".to_owned(), "C2".to_owned())];
    let threads = Arc::new(Mutex::new(Threads::open_channels(&dir, &channels, FakeSlack::new()).unwrap()));
    let id = Objects::open(&dir).unwrap().put(PNG).unwrap();
    let entry = FileEntry { id: id.clone(), content_type: "image/png".to_owned(), name: "shot".to_owned() };
    Files::open(&dir).unwrap().add("fixme", entry).unwrap();
    let declared = vec!["gonk-chat".to_owned(), "fixme".to_owned()];
    let kernel = serve::kernel(threads, dir.join("spaces"), declared);
    let iri = format!("urn:slack:channel:fixme:file:{id}");

    let chore = Capability::scoped([scope("fixme", "file:delete")]);
    let deleted = futures_executor::block_on(kernel.issue(Request::new(Verb::Delete, Iri::parse(&iri).unwrap()), &chore));
    assert!(deleted.is_ok(), "the retention chore could not delete {iri}: {deleted:?}");

    let reader = Capability::scoped([scope("fixme", "file:read")]);
    let read = futures_executor::block_on(kernel.issue(Request::new(Verb::Source, Iri::parse(&iri).unwrap()), &reader));
    assert!(matches!(&read, Err(Error::NotFound(m)) if m.starts_with("no-file")), "reading a deleted file: {read:?}");
    assert!(Objects::open(&dir).unwrap().get(&id).unwrap().is_none(), "the object is not gone");
    let _ = std::fs::remove_dir_all(&dir);
}
