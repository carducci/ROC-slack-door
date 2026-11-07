//! The object store: a file's bytes, kept once under their content address.
//!
//! Bytes live at `objects/{first two hex}/{next two hex}/{rest}` under the state directory,
//! named by `ContentId::of` of the bytes. Every path is built from the 64 hex digits of a
//! digest and nothing else, so no caller-supplied text reaches the file system.
//!
//! This is built on `std::fs` with that digest-only jail; moving it onto `ikigai-fs` is the
//! open question of the draft pull request.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ikigai_core::ContentId;

/// Counts the temporary names this process has made, so no two writers share one.
static NEXT_TMP: AtomicU64 = AtomicU64::new(0);

/// A write-once store of bytes by content.
pub struct Objects {
    root: PathBuf,
}

impl Objects {
    /// The store under `state_dir/objects`, made if it is not there.
    pub fn open(state_dir: &Path) -> io::Result<Objects> {
        let root = state_dir.join("objects");
        fs::create_dir_all(&root)?;
        Ok(Objects { root })
    }

    /// Where an object lives: `objects/xx/yy/<rest>`.
    pub fn path(&self, id: &ContentId) -> PathBuf {
        let hex = id.to_string();
        let hex = &hex["b3:".len()..];
        self.root.join(&hex[0..2]).join(&hex[2..4]).join(&hex[4..])
    }

    /// Keep `bytes` and return their content address. Content already stored is not written
    /// again; new content goes to a temporary name and is hard-linked into place, which
    /// refuses a name already taken, so a reader never sees half an object and a stored
    /// object is never replaced.
    pub fn put(&self, bytes: &[u8]) -> io::Result<ContentId> {
        let id = ContentId::of(bytes);
        let path = self.path(&id);
        if path.is_file() {
            return Ok(id);
        }
        let dir = path.parent().expect("an object path has a parent");
        // Each fan-out directory is made one level at a time and must be a real directory:
        // a symbolic link there would carry the write out of the store.
        let mut at = self.root.clone();
        for part in dir.strip_prefix(&self.root).expect("an object lives under the root") {
            at.push(part);
            match fs::symlink_metadata(&at) {
                Ok(m) if m.is_dir() => {}
                Ok(_) => return Err(io::Error::other("a fan-out path is not a plain directory")),
                Err(e) if e.kind() == io::ErrorKind::NotFound => match fs::create_dir(&at) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e),
                },
                Err(e) => return Err(e),
            }
        }
        // A name of this write's own: the process id, a counter shared by every writer in the
        // process, and a retry on a name already taken (a leftover of an earlier run, whose
        // process id and counter may repeat). `create_new` never follows or overwrites it.
        let (tmp, mut file) = loop {
            let n = NEXT_TMP.fetch_add(1, Ordering::Relaxed);
            let tmp = dir.join(format!(".{}.{n}.tmp", std::process::id()));
            match fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
                Ok(file) => break (tmp, file),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        };
        let written = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            // A hard link refuses a name already taken, where a rename would replace it: an
            // object stored is never rewritten.
            match fs::hard_link(&tmp, &path) {
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && path.is_file() => Ok(()),
                other => other,
            }
        })();
        let _ = fs::remove_file(&tmp);
        written?;
        Ok(id)
    }

    /// The bytes stored under `id`, if any.
    pub fn get(&self, id: &ContentId) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.path(id)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("objects-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn bytes_land_at_their_fan_out_path() {
        let dir = temp_dir("path");
        let store = Objects::open(&dir).unwrap();
        let id = store.put(b"a screenshot").unwrap();
        assert_eq!(id, ContentId::of(b"a screenshot"));
        let hex = id.to_string();
        let hex = &hex[3..];
        let expected = dir.join("objects").join(&hex[0..2]).join(&hex[2..4]).join(&hex[4..]);
        assert_eq!(fs::read(expected).unwrap(), b"a screenshot");
        assert_eq!(store.get(&id).unwrap().unwrap(), b"a screenshot");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_same_bytes_twice_are_one_object_and_not_written_again() {
        let dir = temp_dir("twice");
        let store = Objects::open(&dir).unwrap();
        let first = store.put(b"same").unwrap();
        let path = store.path(&first);
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let second = store.put(b"same").unwrap();
        assert_eq!(first, second);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
        let names = fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(names, 1, "no temporary name is left behind");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_object_is_none() {
        let dir = temp_dir("missing");
        let store = Objects::open(&dir).unwrap();
        assert!(store.get(&ContentId::of(b"never stored")).unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_leaves_no_object_and_no_temporary_name() {
        let dir = temp_dir("fail");
        let store = Objects::open(&dir).unwrap();
        let id = ContentId::of(b"blocked");
        // A directory where the object should go makes the rename fail.
        fs::create_dir_all(store.path(&id)).unwrap();
        assert!(store.put(b"blocked").is_err());
        let parent = store.path(&id).parent().unwrap().to_path_buf();
        let leftovers = fs::read_dir(parent)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
