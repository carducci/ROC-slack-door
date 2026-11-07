//! Review of PR 126 (ledger item 41, task 2.1): the object store's temporary name.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};

use ikigai_core::ContentId;
use roc_slack_door::Objects;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("objects-review-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The same screenshot posted in two messages, kept by two threads of one door at once: both
/// must be kept, as one object. The temporary name is `.{pid}.tmp`, shared by every writer in
/// the process, so one writer's `create_new` fails on the other's file, and its clean-up then
/// removes the file the other is about to rename.
#[test]
fn two_puts_of_the_same_bytes_at_once_both_succeed() {
    let dir = temp_dir("race");
    let bytes: Vec<u8> = (0..4 * 1024 * 1024).map(|i| (i * 31 % 251) as u8).collect();
    let mut failures = Vec::new();
    for round in 0..20 {
        let round_dir = dir.join(round.to_string());
        let store = Arc::new(Objects::open(&round_dir).unwrap());
        let gate = Arc::new(Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let (store, gate, bytes) = (store.clone(), gate.clone(), bytes.clone());
                std::thread::spawn(move || {
                    gate.wait();
                    store.put(&bytes)
                })
            })
            .collect();
        for w in workers {
            if let Err(e) = w.join().unwrap() {
                failures.push(format!("round {round}: {e}"));
            }
        }
        let id = ContentId::of(&bytes);
        if store.get(&id).unwrap().as_deref() != Some(&bytes[..]) {
            failures.push(format!("round {round}: the object is not stored"));
        }
    }
    let _ = fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "a concurrent put of the same bytes failed: {failures:?}");
}

/// A door that crashed mid-write leaves `.{pid}.tmp` behind; restarted under the same process
/// id (pids are reused; in a container the door is pid 1 every time), its next put of any object in that directory must
/// still succeed. `create_new` on the leftover name fails it instead.
#[test]
fn a_temporary_name_left_by_an_earlier_run_does_not_fail_a_put() {
    let dir = temp_dir("stale");
    let store = Objects::open(&dir).unwrap();
    let id = ContentId::of(b"a screenshot");
    let parent = store.path(&id).parent().unwrap().to_path_buf();
    fs::create_dir_all(&parent).unwrap();
    fs::write(parent.join(format!(".{}.tmp", std::process::id())), b"half a scre").unwrap();
    let put = store.put(b"a screenshot");
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(put.ok(), Some(id), "a leftover temporary name failed the put");
}
