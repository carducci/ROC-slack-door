//! The per-channel file index: which bytes a channel names, and under what type and name.
//!
//! Each channel's index is `files/{channel}.tsv` under the state directory, one line per file:
//! `b3:{hex}`, the type, the name, tab separated. The bytes themselves are in the object store
//! and may be named by several channels; deleting a name removes the object only when no
//! channel's index names it any more.
//!
//! Callers serialize their use of one state directory; the check for other names and the
//! removal of the object are not atomic against a concurrent add in another process.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ikigai_core::ContentId;

use crate::Objects;

/// One file a channel names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub id: ContentId,
    pub content_type: String,
    pub name: String,
}

/// The indexes of every channel, beside the object store they point into.
pub struct Files {
    dir: PathBuf,
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg.to_string())
}

fn plain(field: &str) -> bool {
    !field.contains(['\t', '\n', '\r'])
}

impl Files {
    /// The indexes under `state_dir/files`, made if they are not there.
    pub fn open(state_dir: &Path) -> io::Result<Files> {
        let dir = state_dir.join("files");
        fs::create_dir_all(&dir)?;
        Ok(Files { dir })
    }

    fn index_path(&self, channel: &str) -> io::Result<PathBuf> {
        let ok = !channel.is_empty()
            && channel.len() <= 80
            && channel.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !ok {
            return Err(bad("a channel name is letters, digits, '-' and '_'"));
        }
        Ok(self.dir.join(format!("{channel}.tsv")))
    }

    fn load(&self, channel: &str) -> io::Result<Vec<FileEntry>> {
        let text = match fs::read_to_string(self.index_path(channel)?) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let corrupt = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
        let mut entries = Vec::new();
        for line in text.lines().filter(|l| !l.is_empty()) {
            let mut parts = line.splitn(3, '\t');
            let (Some(id), Some(ty), Some(name)) = (parts.next(), parts.next(), parts.next())
            else {
                return Err(corrupt("a malformed index line"));
            };
            let id = ContentId::parse(id).map_err(|_| corrupt("a malformed content id"))?;
            entries.push(FileEntry { id, content_type: ty.into(), name: name.into() });
        }
        Ok(entries)
    }

    fn save(&self, channel: &str, entries: &[FileEntry]) -> io::Result<()> {
        let path = self.index_path(channel)?;
        let mut text = String::new();
        for e in entries {
            text.push_str(&format!("{}\t{}\t{}\n", e.id, e.content_type, e.name));
        }
        let tmp = self.dir.join(format!(".{channel}.{}.tmp", std::process::id()));
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &path).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
    }

    /// Name `entry.id` in `channel`. Naming bytes already named replaces the type and name.
    pub fn add(&self, channel: &str, entry: FileEntry) -> io::Result<()> {
        if !plain(&entry.content_type) || !plain(&entry.name) {
            return Err(bad("a type or name may not hold a tab or a line break"));
        }
        let mut entries = self.load(channel)?;
        entries.retain(|e| e.id != entry.id);
        entries.push(entry);
        self.save(channel, &entries)
    }

    /// What `channel` calls these bytes, if it names them.
    pub fn get(&self, channel: &str, id: &ContentId) -> io::Result<Option<FileEntry>> {
        Ok(self.load(channel)?.into_iter().find(|e| &e.id == id))
    }

    /// Every file `channel` names.
    pub fn list(&self, channel: &str) -> io::Result<Vec<FileEntry>> {
        self.load(channel)
    }

    /// Remove `channel`'s name for `id`, and the object too once no channel names it. A name
    /// that is not there succeeds and changes nothing. Returns whether the object was removed.
    pub fn delete(&self, objects: &Objects, channel: &str, id: &ContentId) -> io::Result<bool> {
        let mut entries = self.load(channel)?;
        let before = entries.len();
        entries.retain(|e| &e.id != id);
        if entries.len() == before {
            return Ok(false);
        }
        self.save(channel, &entries)?;
        for other in self.channels()? {
            if self.get(&other, id)?.is_some() {
                return Ok(false);
            }
        }
        objects.remove(id)?;
        Ok(true)
    }

    fn channels(&self) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for e in fs::read_dir(&self.dir)? {
            let file = e?.file_name().to_string_lossy().into_owned();
            if let Some(name) = file.strip_suffix(".tsv") {
                if !name.starts_with('.') {
                    names.push(name.to_string());
                }
            }
        }
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("files-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(id: ContentId, name: &str) -> FileEntry {
        FileEntry { id, content_type: "image/png".into(), name: name.into() }
    }

    #[test]
    fn a_name_is_indexed_and_read_back() {
        let dir = temp_dir("index");
        let objects = Objects::open(&dir).unwrap();
        let files = Files::open(&dir).unwrap();
        let id = objects.put(b"shot").unwrap();
        files.add("fixme", entry(id.clone(), "shot.png")).unwrap();
        assert_eq!(files.get("fixme", &id).unwrap(), Some(entry(id.clone(), "shot.png")));
        assert!(files.get("other", &id).unwrap().is_none());
        let text = fs::read_to_string(dir.join("files/fixme.tsv")).unwrap();
        assert_eq!(text, format!("{id}\timage/png\tshot.png\n"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_the_only_name_removes_the_object() {
        let dir = temp_dir("only");
        let objects = Objects::open(&dir).unwrap();
        let files = Files::open(&dir).unwrap();
        let id = objects.put(b"shot").unwrap();
        files.add("fixme", entry(id.clone(), "a.png")).unwrap();
        assert!(files.delete(&objects, "fixme", &id).unwrap());
        assert!(files.get("fixme", &id).unwrap().is_none());
        assert!(objects.get(&id).unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_object_stays_while_another_channel_names_it() {
        let dir = temp_dir("shared");
        let objects = Objects::open(&dir).unwrap();
        let files = Files::open(&dir).unwrap();
        let id = objects.put(b"shot").unwrap();
        files.add("fixme", entry(id.clone(), "a.png")).unwrap();
        files.add("other", entry(id.clone(), "b.png")).unwrap();
        assert!(!files.delete(&objects, "fixme", &id).unwrap());
        assert!(objects.get(&id).unwrap().is_some());
        assert!(files.get("other", &id).unwrap().is_some());
        assert!(files.delete(&objects, "other", &id).unwrap());
        assert!(objects.get(&id).unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_a_name_that_is_not_there_changes_nothing() {
        let dir = temp_dir("absent");
        let objects = Objects::open(&dir).unwrap();
        let files = Files::open(&dir).unwrap();
        let id = objects.put(b"shot").unwrap();
        files.add("other", entry(id.clone(), "b.png")).unwrap();
        assert!(!files.delete(&objects, "fixme", &id).unwrap());
        assert!(objects.get(&id).unwrap().is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_channel_name_cannot_leave_the_directory_and_fields_cannot_break_a_line() {
        let dir = temp_dir("jail");
        let files = Files::open(&dir).unwrap();
        let id = ContentId::of(b"x");
        assert!(files.add("../x", entry(id.clone(), "a")).is_err());
        assert!(files.get("a/b", &id).is_err());
        assert!(files.add("fixme", entry(id, "a\tb")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
