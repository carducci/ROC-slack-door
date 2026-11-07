//! Review of PR 131 round 1a (ledger item 42, task 2.2): deleting a file's name, and its object
//! once no channel names it.

use std::fs;
use std::path::PathBuf;

use roc_slack_door::{FileEntry, Files, Objects};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("files-review-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// `delete` saves the channel's index without the name before it reads the other channels'
/// indexes. When one of those reads fails, the call errs with the name already gone, and a retry
/// finds no name, succeeds and changes nothing: the object stays though no channel names it,
/// and no delete can ever remove it.
#[test]
fn a_delete_that_errs_leaves_a_name_a_retry_can_finish() {
    let dir = temp_dir("orphan");
    let objects = Objects::open(&dir).unwrap();
    let files = Files::open(&dir).unwrap();
    let id = objects.put(b"a screenshot").unwrap();
    let entry = FileEntry {
        id: id.clone(),
        content_type: "image/png".into(),
        name: "a.png".into(),
    };
    files.add("fixme", entry).unwrap();
    // Another channel's index that can't be read.
    let other = dir.join("files/other.tsv");
    fs::write(&other, "not an index line\n").unwrap();

    let first = files.delete(&objects, "fixme", &id);
    assert!(first.is_err(), "an unreadable index makes the delete fail");

    // The failure is fixed, and the retention chore tries again.
    fs::remove_file(&other).unwrap();
    let _ = files.delete(&objects, "fixme", &id);
    let named = files.get("fixme", &id).unwrap().is_some();
    let stored = objects.get(&id).unwrap().is_some();
    assert!(
        named || !stored,
        "no channel names the bytes, yet the object is still stored and no delete can remove it"
    );
    let _ = fs::remove_dir_all(&dir);
}
