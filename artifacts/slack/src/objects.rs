//! The object store: a file's bytes, kept once under their content address.
//!
//! Bytes live at `objects/{first two hex}/{next two hex}/{rest}` under the state directory,
//! named by `ContentId::of` of the bytes. Every path is built from the 64 hex digits of a
//! digest and nothing else, so no caller-supplied text reaches the file system.
//!
//! Every read, write and removal goes through `ikigai-fs`'s jail over `objects/`: the path is
//! walked one name at a time with `openat` and no link followed, a new object is published
//! with a link that refuses a name already taken, and the capability this store holds names
//! the jail's root and nothing wider.

use std::io;
use std::path::{Path, PathBuf};
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use base64::Engine;
use ikigai_core::{
    ArgRef, Body, Capability, ContentId, EndpointSpace, Error, Iri, Kernel, Representation, Request, UriTemplate, Verb,
};

/// A write-once store of bytes by content.
pub struct Objects {
    root: PathBuf,
    kernel: Kernel,
}

impl std::fmt::Debug for Objects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Objects").field("root", &self.root).finish_non_exhaustive()
    }
}

/// A refusal or failure of the jail as an `io::Error`; `NotFound` stays `NotFound`.
fn io_error(e: Error) -> io::Error {
    match e {
        Error::NotFound(m) => io::Error::new(io::ErrorKind::NotFound, m),
        other => io::Error::other(other.to_string()),
    }
}

/// Wakes the thread that polls a future.
struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Poll `future` to its end on this thread. The store is called from inside the door's own
/// executor, which `futures_executor::block_on` refuses to nest in; `ikigai-fs` does blocking
/// file calls and never waits on another task, so parking this thread is enough.
fn block_on<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(out) => return out,
            Poll::Pending => std::thread::park(),
        }
    }
}

impl Objects {
    /// The store under `state_dir/objects`, made if it is not there.
    pub fn open(state_dir: &Path) -> io::Result<Objects> {
        let root = state_dir.join("objects");
        std::fs::create_dir_all(&root)?;
        // A channel's size limit is the host's, so the store sets no ceiling of its own on a write.
        let files = ikigai_fs::FileEndpoint::new(&root).max_write(u64::MAX);
        let grammar = UriTemplate::parse(ikigai_fs::FILE_TEMPLATE).expect("FILE_TEMPLATE is a valid template");
        let kernel = Kernel::new(Arc::new(EndpointSpace::new().bind(grammar, files)));
        Ok(Objects { root, kernel })
    }

    /// Where an object lives: `objects/xx/yy/<rest>`.
    pub fn path(&self, id: &ContentId) -> PathBuf {
        self.root.join(Self::relative(id))
    }

    /// The object's path inside the jail: `xx/yy/<rest>`.
    fn relative(id: &ContentId) -> String {
        let hex = id.to_string();
        let hex = &hex["b3:".len()..];
        format!("{}/{}/{}", &hex[0..2], &hex[2..4], &hex[4..])
    }

    /// Run `verb` on `rel` inside the jail, holding read, write and delete on the jail's root
    /// only. A path outside the jail is refused whatever the capability.
    fn issue(&self, verb: Verb, rel: &str, args: Vec<(&str, Vec<u8>)>) -> Result<Representation, Error> {
        let repr = block_on(self.issue_body(verb, rel, args))?;
        let bytes = repr.bytes.as_bytes().cloned().ok_or_else(|| Error::Endpoint("a streamed answer".into()))?;
        Ok(Representation::new(repr.repr_type, bytes))
    }

    /// `issue`, answering the body as the endpoint produced it, so a file reads past the run's
    /// byte budget.
    async fn issue_body(
        &self,
        verb: Verb,
        rel: &str,
        args: Vec<(&str, Vec<u8>)>,
    ) -> Result<Representation<Body>, Error> {
        let iri = Iri::parse(&format!("urn:file:{rel}"))
            .map_err(|e| Error::InvalidArgument { name: "path".into(), detail: e.to_string() })?;
        let mut request = Request::new(verb, iri);
        for (name, value) in args {
            request = request.with_arg(name, ArgRef::Inline(value));
        }
        let held = Capability::scoped(["urn:cap:fs:read:.", "urn:cap:fs:write:.", "urn:cap:fs:delete:."]);
        self.kernel.issue_body(request, &held).await
    }

    /// Keep `bytes` and return their content address. Content already stored is not written
    /// again; new content is staged and synced in its directory and published with a link
    /// that refuses a name already taken, so a reader never sees half an object and a stored
    /// object is never replaced.
    pub fn put(&self, bytes: &[u8]) -> io::Result<ContentId> {
        let id = ContentId::of(bytes);
        let content = base64::engine::general_purpose::STANDARD.encode(bytes).into_bytes();
        let args = vec![("content", content), ("encoding", b"base64".to_vec()), ("if-none-match", b"*".to_vec())];
        match self.issue(Verb::Sink, &Self::relative(&id), args) {
            // A lost create means the object is there already.
            Ok(_) | Err(Error::Conflict(_)) => Ok(id),
            Err(e) => Err(io_error(e)),
        }
    }

    /// Remove the object under `id`. An object that is not there succeeds and changes nothing.
    pub fn remove(&self, id: &ContentId) -> io::Result<()> {
        match self.issue(Verb::Delete, &Self::relative(id), vec![]) {
            Ok(_) | Err(Error::NotFound(_)) => Ok(()),
            Err(e) => Err(io_error(e)),
        }
    }

    /// The bytes stored under `id`, if any.
    pub fn get(&self, id: &ContentId) -> io::Result<Option<Vec<u8>>> {
        let args = vec![("as", b"application/octet-stream".to_vec())];
        let read = async {
            let repr = self.issue_body(Verb::Source, &Self::relative(id), args).await?;
            // A file answers a body that reads from the file; a buffer is a directory's listing.
            if matches!(repr.bytes, Body::Bytes(_)) {
                return Err(Error::Endpoint(format!("{} is not a file", Self::relative(id))));
            }
            Ok(repr.bytes.collect(usize::MAX).await?.to_vec())
        };
        match block_on(read) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(Error::NotFound(_)) => Ok(None),
            Err(e) => Err(io_error(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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

    /// What the jail refuses: a path that climbs out, an absolute one, a link, a second spelling.
    #[test]
    fn the_jail_refuses_a_path_outside_the_store() {
        let dir = temp_dir("outside");
        let store = Objects::open(&dir).unwrap();
        fs::write(dir.join("secret"), b"not an object").unwrap();
        for rel in ["../secret", "..", "/etc/passwd", "ab/../../secret", "./ab", "ab//cd"] {
            for verb in [Verb::Source, Verb::Delete] {
                let got = store.issue(verb, rel, vec![]);
                assert!(
                    matches!(got, Err(Error::InvalidArgument { .. })),
                    "{verb:?} {rel}: {got:?}"
                );
            }
            let args = vec![("content", b"x".to_vec())];
            assert!(store.issue(Verb::Sink, rel, args).is_err(), "sink {rel}");
        }
        assert_eq!(fs::read(dir.join("secret")).unwrap(), b"not an object");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2, "nothing new beside the store");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_jail_refuses_a_link_inside_the_store() {
        let dir = temp_dir("link");
        let outside = dir.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret"), b"not an object").unwrap();
        let store = Objects::open(&dir).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("objects").join("ln")).unwrap();
        let got = store.issue(Verb::Source, "ln/secret", vec![("as", b"application/octet-stream".to_vec())]);
        assert!(matches!(got, Err(Error::InvalidArgument { .. })), "{got:?}");
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
