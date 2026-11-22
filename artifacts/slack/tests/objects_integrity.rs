//! Task 4.1 of `channel-files`: the object store's integrity check from the design. Every object
//! under `objects/` has bytes whose BLAKE3 digest is its own path, so a name can be trusted for
//! the content it names (and the check can later run as a chore).

use std::fs;
use std::path::{Path, PathBuf};

use ikigai_core::ContentId;
use roc_slack_door::Objects;

fn files_under(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.path().is_dir() {
            files_under(&entry.path(), found);
        } else {
            found.push(entry.path());
        }
    }
}

#[test]
fn every_object_has_the_digest_its_path_names() {
    let dir = std::env::temp_dir().join(format!("objects-integrity-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let store = Objects::open(&dir).unwrap();
    for n in 0..20u8 {
        store.put(&vec![n; 1 + usize::from(n) * 100]).unwrap();
    }
    let root = dir.join("objects");
    let mut found = Vec::new();
    files_under(&root, &mut found);
    assert_eq!(found.len(), 20);
    for path in found {
        let parts: Vec<String> = path
            .strip_prefix(&root)
            .unwrap()
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(parts.len(), 3, "{path:?}: objects/xx/yy/<rest>");
        let named = format!("b3:{}{}{}", parts[0], parts[1], parts[2]);
        assert_eq!(ContentId::of(&fs::read(&path).unwrap()).to_string(), named, "{path:?}");
    }
    let _ = fs::remove_dir_all(&dir);
}
