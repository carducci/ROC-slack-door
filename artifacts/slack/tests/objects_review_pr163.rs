//! Review of PR 163 round 1a (ledger item 406, task 2.1): the object store on `ikigai-fs`
//! keeps the `put`/`get` interface it had on `std::fs`.

use std::fs;
use std::path::PathBuf;

use ikigai_core::ContentId;
use roc_slack_door::Objects;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("objects-review-pr163-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A channel's size limit is any positive number of bytes the host sets (`file_max_bytes`), and
/// intake keeps every accepted file within it. `ikigai-fs` refuses a `Sink` past its root's
/// 64 MiB `max_write`, and the kernel refuses a `Source` past the run's 64 MiB byte budget, so a
/// file of 65 MiB in a channel whose limit is 100 MB is noted `file-unavailable`, and an object
/// the old store kept at that size can no longer be read.
#[test]
fn an_object_over_64_mib_is_kept_and_read_back() {
    let dir = temp_dir("big");
    let store = Objects::open(&dir).unwrap();
    let bytes: Vec<u8> = (0..65usize << 20).map(|i| (i % 251) as u8).collect();
    let put = store.put(&bytes);
    let got = put.as_ref().ok().map(|id| store.get(id));
    let _ = fs::remove_dir_all(&dir);
    let id = put.expect("a 65 MiB object is refused by put");
    assert_eq!(id, ContentId::of(&bytes));
    let got = got.unwrap().expect("a 65 MiB object is refused by get");
    assert_eq!(got.map(|b| b.len()), Some(bytes.len()), "a 65 MiB object does not read back");
}

/// `get` answers the bytes stored under an id. `ikigai-fs` answers a `Source` of a directory
/// with its listing, so a directory at an object's path reads back as `Some` of empty bytes,
/// bytes whose content address is not `id`. On `std::fs` it was an error.
#[test]
fn a_directory_at_an_objects_path_is_not_read_as_its_bytes() {
    let dir = temp_dir("dir");
    let store = Objects::open(&dir).unwrap();
    let id = ContentId::of(b"a screenshot");
    fs::create_dir_all(store.path(&id)).unwrap();
    let got = store.get(&id);
    let _ = fs::remove_dir_all(&dir);
    assert!(!matches!(got, Ok(Some(_))), "a directory was read as the object's bytes: {got:?}");
}
