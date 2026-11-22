//! Reading `urn:slack:channel:{name}:file:b3:{hex}` through the door's kernel.

use std::sync::{Arc, Mutex};

use ikigai_core::{Capability, Error, Expiry, Iri, Kernel, Representation, Request, Result, Verb};
use roc_slack_door::{descriptions, scope, FakeSlack, FileEntry, Files, Objects, Threads};

use crate::serve::kernel;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nbytes";

/// A two-channel door whose `fixme` received `PNG`; returns the file's `b3:` digest too.
fn door_with_file(name: &str) -> (Kernel, String) {
    let dir = std::env::temp_dir().join(format!("roc-door-files-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let channels = [("gonk-chat".to_owned(), "C1".to_owned()), ("fixme".to_owned(), "C2".to_owned())];
    let threads = Arc::new(Mutex::new(Threads::open_channels(&dir, &channels, FakeSlack::new()).unwrap()));
    let id = Objects::open(&dir).unwrap().put(PNG).unwrap();
    let entry = FileEntry { id: id.clone(), content_type: "image/png".to_owned(), name: "shot".to_owned() };
    Files::open(&dir).unwrap().add("fixme", entry).unwrap();
    let declared = vec!["gonk-chat".to_owned(), "fixme".to_owned()];
    (kernel(threads, dir.join("spaces"), declared), id.to_string())
}

fn source(kernel: &Kernel, iri: &str, scopes: &[(&str, &str)]) -> Result<Representation> {
    let capability = Capability::scoped(scopes.iter().map(|(n, a)| scope(n, a)));
    let request = Request::new(Verb::Source, Iri::parse(iri).unwrap());
    futures_executor::block_on(kernel.issue(request, &capability))
}

#[test]
fn a_file_is_read_as_its_bytes_with_the_indexed_type_and_cacheable() {
    let (kernel, id) = door_with_file("read");
    let got = source(&kernel, &format!("urn:slack:channel:fixme:file:{id}"), &[("fixme", "file:read")]).unwrap();
    assert_eq!(got.bytes, PNG);
    assert_eq!(got.repr_type.canonical(), "image/png");
    assert_eq!(got.expiry, Expiry::Never);
}

#[test]
fn the_same_bytes_in_another_channel_are_no_file() {
    let (kernel, id) = door_with_file("other");
    let err = source(&kernel, &format!("urn:slack:channel:gonk-chat:file:{id}"), &[("gonk-chat", "file:read")]);
    assert!(matches!(&err, Err(Error::NotFound(m)) if m.starts_with("no-file")), "{err:?}");
}

#[test]
fn a_malformed_address_is_bad_file() {
    let (kernel, id) = door_with_file("bad");
    let upper = format!("b3:{}", id["b3:".len()..].to_uppercase());
    for tail in ["b3:xyz", "sha:abc", upper.as_str(), "b3:"] {
        let err = source(&kernel, &format!("urn:slack:channel:fixme:file:{tail}"), &[("fixme", "file:read")]);
        assert!(matches!(&err, Err(Error::InvalidArgument { detail, .. }) if detail.starts_with("bad-file")), "{tail}: {err:?}");
    }
}

#[test]
fn reading_a_file_needs_file_read_and_nothing_else_admits_it() {
    let (kernel, id) = door_with_file("cap");
    let iri = format!("urn:slack:channel:fixme:file:{id}");
    for held in [("fixme", "read"), ("fixme", "post"), ("gonk-chat", "file:read")] {
        let err = source(&kernel, &iri, &[held]);
        assert!(matches!(err, Err(Error::Denied(_))), "{held:?}: {err:?}");
    }
}

#[test]
fn a_caller_holding_only_file_read_cannot_post() {
    let (kernel, _) = door_with_file("triage");
    let capability = Capability::scoped([scope("fixme", "file:read")]);
    let request = Request::new(Verb::Sink, Iri::parse("urn:slack:channel:fixme:thread:triage").unwrap())
        .with_arg("content", ikigai_core::ArgRef::Inline(b"hello".to_vec()));
    let err = futures_executor::block_on(kernel.issue(request, &capability));
    assert!(matches!(&err, Err(Error::Denied(m)) if m.contains("urn:cap:slack:channel:fixme:post")), "{err:?}");
}

#[test]
fn the_file_resource_is_described() {
    let described = descriptions();
    let meta = described.iter().find(|m| m.resource.ends_with(":file:b3:{hex}")).unwrap();
    assert_eq!(meta.verbs, vec![("source", "urn:cap:slack:channel:{name}:file:read")]);
}
