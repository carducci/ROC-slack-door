//! Review 1 of PR 144 (ledger item 44, task 3.2 of `channel-files`): reading a file.
//!
//! "A malformed address MUST be refused as `bad-file`." The file template binds `{id}` after
//! `:file:`, and a template variable matches nothing empty, so `urn:slack:channel:fixme:file:`
//! never reaches the file endpoint: the catch-all answers it as a resource "not served yet".

#[path = "../src/serve.rs"]
#[allow(dead_code)]
mod serve;

use std::sync::{Arc, Mutex};

use ikigai_core::{Capability, Error, Iri, Request, Verb};
use roc_slack_door::{scope, FakeSlack, Threads};

#[test]
fn an_empty_file_name_is_bad_file() {
    let dir = std::env::temp_dir().join(format!("review-pr144-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let channels = [("fixme".to_owned(), "C2".to_owned())];
    let threads = Arc::new(Mutex::new(Threads::open_channels(&dir, &channels, FakeSlack::new()).unwrap()));
    let kernel = serve::kernel(threads, dir.join("spaces"), vec!["fixme".to_owned()]);
    let capability = Capability::scoped([scope("fixme", "file:read")]);
    let request = Request::new(Verb::Source, Iri::parse("urn:slack:channel:fixme:file:").unwrap());
    let err = futures_executor::block_on(kernel.issue(request, &capability));
    assert!(
        matches!(&err, Err(Error::InvalidArgument { detail, .. }) if detail.starts_with("bad-file")),
        "{err:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
