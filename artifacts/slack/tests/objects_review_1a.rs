//! Review of PR 126 round 1a (ledger item 41, task 2.1): the object store's jail and its
//! write-once promise.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};

use ikigai_core::ContentId;
use roc_slack_door::Objects;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("objects-review-1a-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The task asks for a store jailed by `ikigai-fs`, whose paths resolve through `openat` on
/// directory handles. The store resolves `objects/xx/yy/<rest>` through `std::fs`, which follows
/// a symbolic link, so a link at a fan-out directory carries the write out of `objects/`.
#[cfg(unix)]
#[test]
fn a_symbolic_link_in_the_store_does_not_carry_a_write_out_of_it() {
    let dir = temp_dir("jail");
    let state = dir.join("state");
    let outside = dir.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let store = Objects::open(&state).unwrap();
    let id = ContentId::of(b"a screenshot");
    let fan_out = store.path(&id).parent().unwrap().parent().unwrap().to_path_buf();
    std::os::unix::fs::symlink(&outside, &fan_out).unwrap();
    let _ = store.put(b"a screenshot");
    let escaped = fs::read_dir(&outside).unwrap().count();
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(escaped, 0, "a put wrote through a symbolic link, outside the store");
}

/// "An existing object is never rewritten" (design D1). Two puts of the same bytes at once both
/// find no object, and each renames its temporary file over the path, so the second rename
/// replaces the object the first one stored. `rename` overwrites; a link that refuses an
/// existing name would not.
#[cfg(unix)]
#[test]
fn an_object_already_stored_is_not_replaced_by_a_concurrent_put() {
    let dir = temp_dir("rewrite");
    let bytes: Vec<u8> = (0..16 * 1024 * 1024).map(|i| (i * 31 % 251) as u8).collect();
    let id = ContentId::of(&bytes);
    let mut rewritten = Vec::new();
    for round in 0..5 {
        let store = Arc::new(Objects::open(&dir.join(round.to_string())).unwrap());
        let path = store.path(&id);
        let gate = Arc::new(Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let (store, gate, bytes) = (store.clone(), gate.clone(), bytes.clone());
                std::thread::spawn(move || {
                    gate.wait();
                    store.put(&bytes).unwrap()
                })
            })
            .collect();
        // The first object to appear, by inode.
        let first = loop {
            if let Ok(meta) = fs::symlink_metadata(&path) {
                break std::os::unix::fs::MetadataExt::ino(&meta);
            }
            std::thread::yield_now();
        };
        for w in workers {
            w.join().unwrap();
        }
        let last = std::os::unix::fs::MetadataExt::ino(&fs::symlink_metadata(&path).unwrap());
        if first != last {
            rewritten.push(round);
        }
    }
    let _ = fs::remove_dir_all(&dir);
    assert!(rewritten.is_empty(), "a stored object was replaced in rounds {rewritten:?}");
}
