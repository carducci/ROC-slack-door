//! Review 2 of PR 112 (ledger item 59, task 5.1): what the door's kernel says about its resources.
//!
//! The door's `Meta` renderer writes each description as Turtle, for a `Meta` with no `as` and for
//! the catalog, "one Turtle graph" a caller queries with SPARQL. A description with no verbs, the
//! catch-all `urn:slack:channel:{rest}`, is written `ik:verb  .`: a predicate with no object, which
//! Turtle's grammar does not allow (`predicateObjectList ::= verb objectList ...`, and an
//! `objectList` holds at least one object). One such entry makes the whole catalog unparsable.
//!
//! And the descriptions of a channel's thread, reaction and inbox are bound under a `{name}`
//! variable, so an undeclared channel's addresses are described as served, with verbs, while every
//! verb there is refused as `no-channel`.

#[path = "../src/serve.rs"]
#[allow(dead_code, unused_imports)]
mod serve;

use std::sync::{Arc, Mutex};

use ikigai_core::{ArgRef, Capability, Iri, Kernel, Request, Verb};
use roc_slack_door::{FakeSlack, Threads};

fn door(name: &str) -> (Kernel, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("roc-door-review-pr112-r2-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let threads = Arc::new(Mutex::new(Threads::open(&dir, "C1", FakeSlack::new()).unwrap()));
    let kernel = serve::kernel(threads, dir.join("spaces"), vec!["gonk-chat".to_owned()]);
    (kernel, dir)
}

/// The lines of a Turtle text where a predicate (`dct:…` or `ik:…`) is directly followed by the
/// `.` or `;` that ends it, with no object between.
fn predicates_without_an_object(turtle: &str) -> Vec<String> {
    let is_predicate = |token: &str| {
        token
            .strip_prefix("dct:")
            .or_else(|| token.strip_prefix("ik:"))
            .is_some_and(|local| !local.is_empty() && local.chars().all(|c| c.is_ascii_alphanumeric()))
    };
    turtle
        .lines()
        .filter(|line| {
            let tokens: Vec<&str> = line.split_whitespace().filter(|token| *token != "[]").collect();
            matches!(tokens.as_slice(), [predicate, "." | ";"] if is_predicate(predicate))
        })
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_turtle_the_doors_kernel_answers_gives_every_predicate_an_object() {
    let (kernel, dir) = door("turtle");
    let mut wrong = Vec::new();
    let catalog = Request::new(Verb::Source, Iri::parse("urn:kernel:catalog").unwrap());
    if let Ok(catalog) = futures_executor::block_on(kernel.issue(catalog, &Capability::root())) {
        let text = String::from_utf8_lossy(&catalog.bytes).into_owned();
        for bad in predicates_without_an_object(&text) {
            wrong.push(format!("the catalog holds `{bad}`"));
        }
    }
    let meta = Request::new(Verb::Meta, Iri::parse("urn:slack:channel:gonk-chat:foo").unwrap());
    if let Ok(meta) = futures_executor::block_on(kernel.issue(meta, &Capability::root())) {
        let text = String::from_utf8_lossy(&meta.bytes).into_owned();
        for bad in predicates_without_an_object(&text) {
            wrong.push(format!("the Meta of urn:slack:channel:gonk-chat:foo holds `{bad}`: {text}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_undeclared_channels_addresses_are_not_described_as_served() {
    let (kernel, dir) = door("undeclared");
    let mut wrong = Vec::new();
    for iri in [
        "urn:slack:channel:random:thread:x",
        "urn:slack:channel:random:thread:x:reaction:tada",
        "urn:slack:channel:random:inbox",
    ] {
        let ask = Request::new(Verb::Meta, Iri::parse(iri).unwrap())
            .with_arg("as", ArgRef::Inline(b"application/json".to_vec()));
        let Ok(meta) = futures_executor::block_on(kernel.issue(ask, &Capability::root())) else {
            continue;
        };
        let meta: serde_json::Value = serde_json::from_slice(&meta.bytes).unwrap_or_default();
        let verbs = meta["verbs"].as_array().cloned().unwrap_or_default();
        for verb in verbs {
            let verb = match verb.as_str() {
                Some("Source") => Verb::Source,
                Some("Sink") => Verb::Sink,
                Some("Delete") => Verb::Delete,
                Some("Exists") => Verb::Exists,
                _ => continue,
            };
            let request = Request::new(verb, Iri::parse(iri).unwrap())
                .with_arg("content", ArgRef::Inline(b"hello".to_vec()));
            if let Err(e) = futures_executor::block_on(kernel.issue(request, &Capability::root())) {
                if e.to_string().contains("no-channel") {
                    wrong.push(format!("{iri} is described as taking {verb:?} ({meta}), but it is refused: {e}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}
