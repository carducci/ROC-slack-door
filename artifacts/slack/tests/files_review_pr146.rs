//! Review 2 of PR 146 (ledger item 45, task 3.3 of `channel-files`): a delete that fails part way.
//!
//! `delete` saves the channel's index without the name, then removes the object. When the remove
//! fails, the call errs with the name already gone; since PR 146 a name that isn't there changes
//! nothing, so the retry finds no name, succeeds, and leaves the object that no channel names and
//! no delete can ever remove. Retention's delete then answers "deleted" while the screenshot stays.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use roc_slack_door::{FileEntry, Files, Objects};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("files-review-pr146-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_delete_whose_object_remove_failed_can_be_retried_to_remove_the_object() {
    let dir = temp_dir("remove");
    let objects = Objects::open(&dir).unwrap();
    let files = Files::open(&dir).unwrap();
    let id = objects.put(b"a screenshot").unwrap();
    files
        .add("fixme", FileEntry { id: id.clone(), content_type: "image/png".into(), name: "F1".into() })
        .unwrap();
    // The object's directory refuses the unlink (a full or read-only disk does the same).
    let fanout = objects.path(&id).parent().unwrap().to_path_buf();
    fs::set_permissions(&fanout, fs::Permissions::from_mode(0o555)).unwrap();
    let first = files.delete(&objects, "fixme", &id);
    fs::set_permissions(&fanout, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(first.is_err(), "the remove was refused, so the delete fails: {first:?}");

    // The failure is fixed, and the retention chore tries again.
    let _ = files.delete(&objects, "fixme", &id);
    let named = files.get("fixme", &id).unwrap().is_some();
    let stored = objects.get(&id).unwrap().is_some();
    assert!(
        named || !stored,
        "no channel names the bytes, yet the object is still stored and no delete can remove it"
    );
    let _ = fs::remove_dir_all(&dir);
}
