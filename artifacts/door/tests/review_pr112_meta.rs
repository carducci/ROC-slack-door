//! Review of PR 112 (ledger item 59, task 5.1): what the door's `Meta` says a name takes, the
//! name takes.
//!
//! The catch-all binding `urn:slack:channel:{rest}` describes itself as `urn:slack:channel:{name}`,
//! a resource whose Sink "posts a new top-level message". So every address under a declared channel
//! that no other template takes, `urn:slack:channel:gonk-chat:thread:` (an empty key) or
//! `urn:slack:channel:gonk-chat:foo:bar`, is described as taking Sink, while a Sink there is
//! refused. A caller that reads the description before it writes is told a write posts.

#[path = "../src/serve.rs"]
#[allow(dead_code, unused_imports)]
mod serve;

use std::sync::{Arc, Mutex};

use ikigai_core::{ArgRef, Capability, Iri, Request, Verb};
use roc_slack_door::{FakeSlack, Threads};

#[test]
fn an_address_described_as_taking_sink_takes_sink() {
    let dir = std::env::temp_dir().join(format!("roc-door-review-pr112-meta-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let threads = Arc::new(Mutex::new(Threads::open(&dir, "C1", FakeSlack::new()).unwrap()));
    let kernel = serve::kernel(Arc::clone(&threads), dir.join("spaces"), vec!["gonk-chat".to_owned()]);
    let mut wrong = Vec::new();
    for iri in ["urn:slack:channel:gonk-chat:thread:", "urn:slack:channel:gonk-chat:foo:bar"] {
        let ask = Request::new(Verb::Meta, Iri::parse(iri).unwrap())
            .with_arg("as", ArgRef::Inline(b"application/json".to_vec()));
        let Ok(meta) = futures_executor::block_on(kernel.issue(ask, &Capability::root())) else {
            continue;
        };
        let meta: serde_json::Value = serde_json::from_slice(&meta.bytes).unwrap_or_default();
        let claims_sink = meta["verbs"].as_array().is_some_and(|verbs| verbs.iter().any(|v| v == "Sink"));
        let write = Request::new(Verb::Sink, Iri::parse(iri).unwrap())
            .with_arg("content", ArgRef::Inline(b"hello".to_vec()));
        let written = futures_executor::block_on(kernel.issue(write, &Capability::root()));
        if claims_sink {
            if let Err(e) = written {
                wrong.push(format!("{iri} is described as {meta}, but a Sink there is refused: {e}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}
