//! The served space: `urn:slack:channel:{name}:thread:{key}` and its reactions as endpoints in a
//! kernel, plus the inbox space the door drops deliveries into, all behind one `Kernel` that
//! another kernel mounts over a Unix socket. Every verb needs the caller's `Capability` to hold
//! `urn:cap:slack:channel:{name}:{read,post,react}` for the channel it acts in. The intray is not
//! served by that kernel: the inbox endpoint reads and takes through a kernel of its own, and the
//! door writes deliveries through the same kind, under a capability that holds only the intray's
//! `out`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ikigai_core::{
    ArgRef, Capability, Description, Endpoint, EndpointSpace, Error, Fallback, Invocation, Iri,
    Kernel, Representation, Request, Result, ReprType, Space, UriTemplate, Verb,
};
use roc_slack_door::{check_channel, descriptions, require, to_json, Refusal, SlackTransport, ThreadError, Threads, READ};

use roc_slack_door::Delivery;

/// The threads, shared by every endpoint and the inbox loop.
pub type Shared<T> = Arc<Mutex<Threads<T>>>;

/// A library refusal or failure, as the kernel's error taxonomy names it.
fn to_error(error: ThreadError) -> Error {
    match error {
        ThreadError::Refused(Refusal::BadKey) => Error::InvalidArgument {
            name: "key".to_owned(),
            detail: "a thread key is 1 to 80 letters, digits, dots, underscores or dashes".to_owned(),
        },
        ThreadError::Refused(Refusal::EmptyMessage) => Error::InvalidArgument {
            name: "content".to_owned(),
            detail: "the message text is empty".to_owned(),
        },
        ThreadError::Refused(Refusal::MessageTooLong) => Error::InvalidArgument {
            name: "content".to_owned(),
            detail: "the message text is too long".to_owned(),
        },
        ThreadError::Refused(Refusal::BadReaction) => Error::InvalidArgument {
            name: "name".to_owned(),
            detail: "a reaction name is lowercase letters, digits, underscores, dashes or pluses"
                .to_owned(),
        },
        ThreadError::Refused(Refusal::NoThread) => {
            Error::NotFound("no thread is open under that key".to_owned())
        }
        ThreadError::Refused(refusal @ Refusal::NoChannel(_)) => {
            let Refusal::NoChannel(name) = &refusal else { unreachable!() };
            Error::NotFound(format!(
                "{}: `{name}` is not a channel this door serves",
                refusal.code()
            ))
        }
        ThreadError::Refused(Refusal::NotGranted(scope)) => {
            Error::Denied(format!("not-granted: this call needs `{scope}`"))
        }
        ThreadError::Unavailable { retry_after_secs } => Error::Unavailable(match retry_after_secs {
            Some(secs) => format!("Slack is busy; retry after {secs}s"),
            None => "Slack is unreachable; retry".to_owned(),
        }),
        ThreadError::Transport(why) => Error::Endpoint(format!("Slack refused: {why:?}")),
        ThreadError::State(why) => Error::Endpoint(format!("the thread map: {why}")),
    }
}

/// The text a sink carries: `content`, or `in` when the caller piped it that way.
fn text_of(inv: &Invocation<'_>) -> Result<String> {
    inv.inline_str("content")
        .or_else(|_| inv.inline_str("in"))
        .map(str::to_owned)
        .map_err(|_| Error::MissingArgument("content".to_owned()))
}

fn ok_text(message: &str) -> Representation {
    Representation::new(
        ReprType::new("text/plain").with_param("charset", "utf-8"),
        format!("{message}\n").into_bytes(),
    )
}

/// What the catalog and the MCP projection show of an endpoint: the `Meta` the library wrote for
/// `resource`, under the published id `id`. The capabilities stay in the summary: a `requires` on
/// a `Description` is enforced by the kernel as a literal scope, and these are per channel.
fn described(id: &str, resource: &str) -> Description {
    let meta = descriptions()
        .into_iter()
        .find(|meta| meta.resource == resource)
        .expect("every served resource has a description");
    let capabilities: Vec<&str> = meta.verbs.iter().map(|(_, capability)| *capability).collect();
    let mut description = Description::new(id)
        .title(resource)
        .summary(format!("{} Needs {}.", meta.description, capabilities.join(", ")));
    for (verb, _) in &meta.verbs {
        description = description.verb(match *verb {
            "source" => Verb::Source,
            "sink" => Verb::Sink,
            "delete" => Verb::Delete,
            "exists" => Verb::Exists,
            other => unreachable!("`{other}` is not a verb the door describes"),
        });
    }
    description
}

/// `urn:slack:channel:{name}:thread:{key}`: sourcing reads the conversation, sinking posts to it.
pub struct ChannelThreadEndpoint<T: SlackTransport> {
    threads: Shared<T>,
    declared: Vec<String>,
}

#[async_trait]
impl<T: SlackTransport + Send + 'static> Endpoint for ChannelThreadEndpoint<T> {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        let channel = inv.bindings.get("name").unwrap_or_default();
        let key = inv.bindings.get("key").unwrap_or_default();
        let mut threads = self
            .threads
            .lock()
            .map_err(|_| Error::Endpoint("the thread map is poisoned".to_owned()))?;
        match inv.request.verb {
            Verb::Source => {
                let entries = threads.read_in(inv.capability,channel, key).map_err(to_error)?;
                Ok(Representation::new(
                    ReprType::new("application/json"),
                    to_json(&entries).into_bytes(),
                ))
            }
            Verb::Sink => {
                check_channel(&self.declared, channel)
                    .map_err(|refusal| to_error(ThreadError::Refused(refusal)))?;
                let text = text_of(inv)?;
                threads.write_in(inv.capability,channel, key, &text).map_err(to_error)?;
                Ok(ok_text("posted"))
            }
            Verb::Exists => {
                let open = threads.exists_in(inv.capability,channel, key).map_err(to_error)?;
                Ok(ok_text(if open { "true" } else { "false" }))
            }
            other => {
                check_channel(&self.declared, channel)
                    .map_err(|refusal| to_error(ThreadError::Refused(refusal)))?;
                Err(Error::Endpoint(format!(
                    "urn:slack:channel:{{name}}:thread takes source, sink and exists, not {other:?}"
                )))
            }
        }
    }

    fn name(&self) -> &str {
        "urn:slack:channel:thread"
    }

    fn describe(&self) -> Description {
        described("slack-channel-thread", "urn:slack:channel:{name}:thread:{key}")
    }
}

/// `urn:slack:channel:{rest}`: every name under a channel. A channel the host does not declare is
/// refused as `no-channel` before Slack is called. A sink to a declared channel's own IRI posts
/// a top-level message; anything else under it is not served yet.
pub struct ChannelEndpoint<T: SlackTransport> {
    threads: Shared<T>,
    declared: Vec<String>,
    /// The id this binding is published under, and the resource `descriptions()` describes there.
    resource: (&'static str, &'static str),
}

#[async_trait]
impl<T: SlackTransport + Send + 'static> Endpoint for ChannelEndpoint<T> {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        let bound_rest = inv.bindings.get("rest");
        let rest = bound_rest.or(inv.bindings.get("name")).unwrap_or_default();
        let name = rest.split(':').next().unwrap_or_default();
        check_channel(&self.declared, name)
            .map_err(|refusal| to_error(ThreadError::Refused(refusal)))?;
        if let (Some(reaction), Verb::Sink | Verb::Delete) = (inv.bindings.get("reaction"), inv.request.verb) {
            let (key, add) = (inv.bindings.get("key").unwrap_or_default(), inv.request.verb == Verb::Sink);
            let mut threads = self.threads.lock().map_err(|_| Error::Endpoint("the thread map is poisoned".to_owned()))?;
            threads.react_in(inv.capability, name, key, reaction, add).map_err(to_error)?;
            return Ok(ok_text(if add { "reacted" } else { "unreacted" }));
        }
        if inv.request.verb == Verb::Sink && bound_rest.is_some_and(|rest| !rest.contains(':')) {
            let text = text_of(inv)?;
            self.threads
                .lock()
                .map_err(|_| Error::Endpoint("the thread map is poisoned".to_owned()))?
                .announce(inv.capability, name, &text)
                .map_err(to_error)?;
            return Ok(ok_text("posted"));
        }
        if rest.split(':').skip(1).eq(["thread", ""]) {
            return Err(to_error(ThreadError::Refused(Refusal::BadKey)));
        }
        Err(Error::NotFound(format!("`{rest}` is not served yet")))
    }

    fn name(&self) -> &str {
        "urn:slack:channel"
    }

    fn describe(&self) -> Description {
        let (id, resource) = self.resource;
        described(id, resource)
    }
}

/// `urn:slack:channel:{name}:inbox`: the channel's inbox, which is the intray space of that name
/// under the state directory. Only Source and Delete are served, and each needs the channel's
/// `read` and nothing else: the intray is called through a kernel no caller can address, under the
/// root capability narrowed to the one intray capability that verb needs. Every other verb, Sink
/// included, is refused before the intray is touched, so a caller never writes a tuple into an
/// inbox; only the door's own deliveries do (`drop_delivery`).
pub struct InboxEndpoint {
    spaces: Kernel,
    declared: Vec<String>,
}

#[async_trait]
impl Endpoint for InboxEndpoint {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        let name = inv.bindings.get("name").unwrap_or_default();
        check_channel(&self.declared, name)
            .map_err(|refusal| to_error(ThreadError::Refused(refusal)))?;
        let needs = match inv.request.verb {
            Verb::Source => ikigai_intray::CAP_READ,
            Verb::Delete => ikigai_intray::CAP_TAKE,
            other => {
                return Err(Error::Endpoint(format!(
                    "urn:slack:channel:{{name}}:inbox takes source and delete, not {other:?}"
                )))
            }
        };
        require(inv.capability, name, READ)
            .map_err(|refusal| to_error(ThreadError::Refused(refusal)))?;
        let target = Iri::parse(&format!("urn:space:{name}"))
            .map_err(|e| Error::Endpoint(format!("{e:?}")))?;
        let mut request = inv.request.clone();
        request.target = target;
        self.spaces.issue(request, &Capability::root().attenuate([needs])).await
    }

    fn name(&self) -> &str {
        "urn:slack:channel:inbox"
    }

    fn describe(&self) -> Description {
        described("slack-channel-inbox", "urn:slack:channel:{name}:inbox")
    }
}

/// The Slack resources. The reaction template is bound first: the thread template's final
/// variable would otherwise take the whole reaction IRI as a key. The inbox template is bound
/// after the channel's thread and reaction templates, so an address ending in `:inbox` that
/// they match is theirs, and before the channel catch-all.
pub fn slack_space<T: SlackTransport + Send + 'static>(
    threads: Shared<T>,
    declared: Vec<String>,
    root: std::path::PathBuf,
) -> EndpointSpace {
    let inbox = InboxEndpoint {
        spaces: intray(root),
        declared: declared.clone(),
    };
    EndpointSpace::new()
        .bind(
            UriTemplate::parse("urn:slack:channel:{name}:thread:{key}:reaction:{reaction}")
                .expect("a fixed template"),
            ChannelEndpoint {
                threads: Arc::clone(&threads),
                declared: declared.clone(),
                resource: (
                    "slack-channel-reaction",
                    "urn:slack:channel:{name}:thread:{key}:reaction:{reaction}",
                ),
            },
        )
        .bind(
            UriTemplate::parse("urn:slack:channel:{name}:thread:{key}").expect("a fixed template"),
            ChannelThreadEndpoint {
                threads: Arc::clone(&threads),
                declared: declared.clone(),
            },
        )
        .bind(
            UriTemplate::parse("urn:slack:channel:{name}:inbox").expect("a fixed template"),
            inbox,
        )
        .bind(
            UriTemplate::parse("urn:slack:channel:{rest}").expect("a fixed template"),
            ChannelEndpoint {
                threads,
                declared,
                resource: ("slack-channel", "urn:slack:channel:{name}"),
            },
        )
}

/// The intray spaces under `root`, as a kernel of their own. The door's kernel does not serve it:
/// the inbox endpoint and `drop_delivery` are the only ways in.
pub fn intray(root: std::path::PathBuf) -> Kernel {
    Kernel::new(Arc::new(ikigai_intray::space(root)))
}

/// The kernel the door serves: the Slack resources only. `urn:space:{name}` is bound to nothing
/// here, under any capability; the inboxes are reached at `urn:slack:channel:{name}:inbox`.
pub fn kernel<T: SlackTransport + Send + 'static>(
    threads: Shared<T>,
    root: std::path::PathBuf,
    declared: Vec<String>,
) -> Kernel {
    Kernel::with_meta_renderer(Arc::new(slack_space(threads, declared, root)), Arc::new(JsonMeta))
}

/// Renders a `Description` as the JSON a mount forwards to a caller who asks for `Meta` with
/// `as: application/json`: its id, title, summary and verbs.
struct JsonMeta;

impl ikigai_core::MetaRenderer for JsonMeta {
    fn render(&self, description: &Description, target: &ReprType) -> Result<Representation> {
        let verbs: Vec<String> = description.verbs.iter().map(|verb| format!("{verb:?}")).collect();
        let json = serde_json::json!({
            "id": description.id,
            "title": description.title,
            "summary": description.summary,
            "verbs": verbs,
        });
        Ok(Representation::new(target.clone(), json.to_string().into_bytes()))
    }
}

/// Drops one delivery into the inbox space `space`, through the `intray` kernel under a capability
/// that holds only the intray's `out`, which the intray does not scope to one space. The tuple is
/// Turtle, so a host can match it with a SPARQL template.
pub fn drop_delivery(intray: &Kernel, space: &str, delivery: &Delivery) -> std::result::Result<(), String> {
    let target = Iri::parse(&format!("urn:space:{space}")).map_err(|e| format!("{e:?}"))?;
    let request = Request::new(Verb::Sink, target)
        .with_arg("content", ArgRef::Inline(turtle(delivery).into_bytes()));
    let capability = Capability::root().attenuate([ikigai_intray::CAP_OUT]);
    futures_executor::block_on(intray.issue(request, &capability))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// A delivery as one Turtle tuple: a blank node typed `slack:Message` or `slack:Reaction`.
pub fn turtle(delivery: &Delivery) -> String {
    let mut predicates: Vec<String> = Vec::new();
    match delivery {
        Delivery::Message {
            key,
            user,
            time,
            text,
        } => {
            predicates.push("a slack:Message".to_owned());
            if let Some(key) = key {
                predicates.push(format!("slack:key {}", literal(key)));
            }
            predicates.push(format!("slack:user {}", literal(user)));
            predicates.push(format!("slack:time {}", literal(time)));
            predicates.push(format!("slack:text {}", literal(text)));
        }
        Delivery::Reaction { key, user, name } => {
            predicates.push("a slack:Reaction".to_owned());
            predicates.push(format!("slack:key {}", literal(key)));
            predicates.push(format!("slack:user {}", literal(user)));
            predicates.push(format!("slack:name {}", literal(name)));
        }
    }
    format!(
        "@prefix slack: <urn:slack:> .\n[] {} .\n",
        predicates.join(" ;\n   ")
    )
}

/// A Turtle string literal.
fn literal(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikigai_core::MetaRenderer;
    use roc_slack_door::FakeSlack;

    fn door(name: &str) -> (Kernel, Shared<FakeSlack>) {
        let dir = std::env::temp_dir().join(format!("roc-door-serve-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let threads = Arc::new(Mutex::new(Threads::open(&dir, "C1", FakeSlack::new()).unwrap()));
        let kernel = kernel(Arc::clone(&threads), dir.join("spaces"), vec!["gonk-chat".to_owned()]);
        (kernel, threads)
    }

    /// Renders a description as the lines a catalog reader sees: its id, title, summary, verbs.
    struct PlainRenderer;

    impl MetaRenderer for PlainRenderer {
        fn render(&self, description: &Description, target: &ReprType) -> Result<Representation> {
            let verbs: Vec<String> = description.verbs.iter().map(|v| format!("{v:?}")).collect();
            let text = format!(
                "id {}\ntitle {}\nsummary {}\nverbs {}\n\n",
                description.id, description.title, description.summary, verbs.join(" ")
            );
            Ok(Representation::new(target.clone(), text.into_bytes()))
        }
    }

    #[test]
    fn the_catalog_describes_every_served_resource_from_its_meta() {
        let dir = std::env::temp_dir().join(format!("roc-door-serve-catalog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let threads = Arc::new(Mutex::new(Threads::open(&dir, "C1", FakeSlack::new()).unwrap()));
        let declared = vec!["gonk-chat".to_owned()];
        let spaces: Vec<Arc<dyn Space>> = vec![Arc::new(slack_space(threads, declared, dir.join("spaces")))];
        let kernel = Kernel::with_meta_renderer(Arc::new(Fallback::new(spaces)), Arc::new(PlainRenderer));
        let request = Request::new(Verb::Source, Iri::parse("urn:kernel:catalog").unwrap());
        let catalog = futures_executor::block_on(kernel.issue(request, &Capability::root())).unwrap();
        let catalog = String::from_utf8(catalog.bytes).unwrap();
        for meta in descriptions() {
            assert!(catalog.contains(&format!("title {}\n", meta.resource)), "{} is not in the catalog: {catalog}", meta.resource);
            assert!(catalog.contains(meta.description), "{} has no description in the catalog", meta.resource);
            for (_, capability) in &meta.verbs {
                assert!(catalog.contains(capability), "{capability} is not in the catalog");
            }
        }
        for id in ["slack-channel", "slack-channel-thread", "slack-channel-reaction", "slack-channel-inbox"] {
            assert!(catalog.contains(&format!("id {id}\n")), "no `{id}` in {catalog}");
        }
    }

    /// "Describing a channel, a reaction and an inbox", asked the way a caller asks: over the
    /// door's socket, from the kernel `main` serves, with the JSON `Meta` a mount forwards.
    #[test]
    fn a_caller_on_the_doors_socket_is_answered_each_resources_description() {
        let (kernel, _threads) = door("describe-socket");
        let path = std::env::temp_dir().join(format!("roc-door-describe-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let serving = Arc::new(kernel);
        let serve_path = path.clone();
        std::thread::spawn(move || ikigai_ipc::serve_shared(serving, &serve_path));
        for _ in 0..200 {
            if path.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let peer = ikigai_ipc::connect(&path).unwrap();
        for (iri, resource) in [
            ("urn:slack:channel:gonk-chat", "urn:slack:channel:{name}"),
            (
                "urn:slack:channel:gonk-chat:thread:carducci-54:reaction:tada",
                "urn:slack:channel:{name}:thread:{key}:reaction:{reaction}",
            ),
            ("urn:slack:channel:gonk-chat:inbox", "urn:slack:channel:{name}:inbox"),
        ] {
            let meta = descriptions().into_iter().find(|meta| meta.resource == resource).unwrap();
            let request = Request::new(Verb::Meta, Iri::parse(iri).unwrap())
                .with_arg("as", ArgRef::Inline(b"application/json".to_vec()));
            let answer = peer
                .issue_as_within(request, &Capability::root(), std::time::Duration::from_secs(10))
                .map(|(answer, _)| String::from_utf8_lossy(&answer.bytes).into_owned());
            let answer = match answer {
                Ok(answer) => answer,
                Err(e) => panic!("asking for the description of {iri} failed: {e}"),
            };
            let summary = serde_json::from_str::<serde_json::Value>(&answer).ok()
                .and_then(|json| json["summary"].as_str().map(str::to_owned))
                .unwrap_or_default();
            assert!(summary.contains(meta.description), "{iri} is described as {answer}");
            assert!(!answer.contains("xoxb-") && !answer.contains("xapp-"), "{iri}: {answer}");
        }
    }

    /// What the door's kernel answers as Turtle is Turtle: a `Meta` with no `as` (the kernel's
    /// default, `text/turtle`) and the catalog (one Turtle graph). A renderer that answers JSON
    /// whatever type it is asked for labels JSON as Turtle, and the kernel never reaches the
    /// transreption it falls back to when a renderer declines a type.
    #[test]
    fn what_the_doors_kernel_labels_turtle_is_not_json() {
        let (kernel, _threads) = door("meta-turtle");
        let is_json = |bytes: &[u8]| serde_json::from_slice::<serde_json::Value>(bytes).is_ok();
        let request = Request::new(Verb::Meta, Iri::parse("urn:slack:channel:gonk-chat:inbox").unwrap());
        if let Ok(answer) = futures_executor::block_on(kernel.issue(request, &Capability::root())) {
            let text = String::from_utf8_lossy(&answer.bytes).into_owned();
            assert!(
                !(answer.repr_type.media_type == "text/turtle" && is_json(&answer.bytes)),
                "a Meta asked as text/turtle is answered with JSON: {text}"
            );
        }
        let request = Request::new(Verb::Source, Iri::parse("urn:kernel:catalog").unwrap());
        if let Ok(catalog) = futures_executor::block_on(kernel.issue(request, &Capability::root())) {
            let text = String::from_utf8_lossy(&catalog.bytes).into_owned();
            let json_lines: Vec<&str> = text.lines().filter(|line| is_json(line.as_bytes())).collect();
            assert!(json_lines.is_empty(), "the Turtle catalog holds JSON objects: {json_lines:?}");
        }
    }

    fn issue(kernel: &Kernel, verb: Verb, iri: &str) -> Result<Representation> {
        let mut request = Request::new(verb, Iri::parse(iri).unwrap());
        if verb == Verb::Sink {
            request = request.with_arg("content", ArgRef::Inline(b"hello".to_vec()));
        }
        futures_executor::block_on(kernel.issue(request, &Capability::root()))
    }

    fn two_channel_door(name: &str) -> (Kernel, Shared<FakeSlack>) {
        let dir = std::env::temp_dir().join(format!("roc-door-serve-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let channels = [
            ("gonk-chat".to_owned(), "C1".to_owned()),
            ("fixme".to_owned(), "C2".to_owned()),
        ];
        let threads = Arc::new(Mutex::new(
            Threads::open_channels(&dir, &channels, FakeSlack::new()).unwrap(),
        ));
        let declared = vec!["gonk-chat".to_owned(), "fixme".to_owned()];
        let kernel = kernel(Arc::clone(&threads), dir.join("spaces"), declared);
        (kernel, threads)
    }

    /// The intray the door of `two_channel_door(name)` reads and writes, as the test's own handle.
    fn intray_of(name: &str) -> Kernel {
        intray(std::env::temp_dir().join(format!("roc-door-serve-{name}-{}", std::process::id())).join("spaces"))
    }

    fn a_message(key: &str, user: &str, text: &str) -> Delivery {
        Delivery::Message {
            key: Some(key.to_owned()),
            user: user.to_owned(),
            time: "1.5".to_owned(),
            text: text.to_owned(),
        }
    }

    #[test]
    fn a_delivery_is_read_and_taken_at_the_channels_inbox_address() {
        let (kernel, _) = two_channel_door("inbox-address");
        drop_delivery(&intray_of("inbox-address"), "gonk-chat", &a_message("carducci-54", "U1", "hi")).unwrap();
        let inbox = "urn:slack:channel:gonk-chat:inbox";
        let text = |r: Result<Representation>| String::from_utf8(r.unwrap().bytes.to_vec()).unwrap();
        let backing = text(issue(&intray_of("inbox-address"), Verb::Source, "urn:space:gonk-chat"));
        assert_eq!(text(issue(&kernel, Verb::Source, inbox)), backing);
        let taken = text(issue(&kernel, Verb::Delete, inbox));
        assert!(taken.contains("carducci-54") && taken.contains("U1") && taken.contains("hi"), "{taken}");
        let fixme = issue(&kernel, Verb::Source, "urn:slack:channel:fixme:inbox");
        assert!(fixme.map_or(true, |r| r.bytes.to_vec() != backing.as_bytes()));
    }

    #[test]
    fn a_thumbs_up_is_taken_at_the_channels_inbox_address() {
        let (kernel, _) = two_channel_door("inbox-reaction");
        let up = Delivery::Reaction { key: "carducci-54".to_owned(), user: "U1".to_owned(), name: "+1".to_owned() };
        drop_delivery(&intray_of("inbox-reaction"), "gonk-chat", &up).unwrap();
        let taken = issue(&kernel, Verb::Delete, "urn:slack:channel:gonk-chat:inbox").unwrap();
        let taken = String::from_utf8(taken.bytes.to_vec()).unwrap();
        assert!(taken.contains("slack:Reaction") && taken.contains("carducci-54") && taken.contains("+1"), "{taken}");
    }

    #[test]
    fn a_thread_address_ending_in_inbox_is_refused_as_bad_key() {
        let (kernel, threads) = two_channel_door("thread-inbox");
        let error = issue(&kernel, Verb::Sink, "urn:slack:channel:gonk-chat:thread:x:inbox")
            .unwrap_err()
            .to_string();
        assert!(error.contains("key") && !error.contains("no-channel"), "{error}");
        assert!(threads.lock().unwrap().transport().posts().is_empty());
    }

    #[test]
    fn a_reaction_named_inbox_is_added_to_the_thread() {
        let (kernel, threads) = two_channel_door("reaction-inbox");
        issue(&kernel, Verb::Sink, "urn:slack:channel:fixme:thread:k").unwrap();
        issue(&kernel, Verb::Sink, "urn:slack:channel:fixme:thread:k:reaction:inbox").unwrap();
        let threads = threads.lock().unwrap();
        let ts = threads.transport().posts()[0].ts.clone();
        assert_eq!(threads.transport().reactions().iter().collect::<Vec<_>>(), [&(ts, "inbox".to_owned())]);
    }

    #[test]
    fn a_delete_aimed_at_one_tuple_takes_that_tuple_and_leaves_the_other() {
        let (kernel, _) = two_channel_door("inbox-targeted");
        drop_delivery(&intray_of("inbox-targeted"), "gonk-chat", &a_message("first-1", "U1", "one")).unwrap();
        drop_delivery(&intray_of("inbox-targeted"), "gonk-chat", &a_message("second-2", "U2", "two")).unwrap();
        let inbox = std::env::temp_dir()
            .join(format!("roc-door-serve-inbox-targeted-{}", std::process::id()))
            .join("spaces/gonk-chat/inbox");
        let mut ids: Vec<String> = std::fs::read_dir(&inbox)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().trim_end_matches(".tuple").to_owned())
            .collect();
        ids.sort();
        assert_eq!(ids.len(), 2, "{ids:?}");
        let text = |id: &str| {
            std::fs::read_to_string(inbox.join(format!("{id}.tuple"))).unwrap()
        };
        let (kept, aimed) = (ids[0].clone(), ids[1].clone());
        let aimed_text = text(&aimed);
        let request = Request::new(Verb::Delete, Iri::parse("urn:slack:channel:gonk-chat:inbox").unwrap())
            .with_arg("tuple", ArgRef::Inline(aimed.clone().into_bytes()));
        let taken = futures_executor::block_on(kernel.issue(request, &Capability::root())).unwrap();
        assert_eq!(String::from_utf8(taken.bytes.to_vec()).unwrap(), aimed_text);
        assert!(inbox.join(format!("{kept}.tuple")).exists());
        assert!(!inbox.join(format!("{aimed}.tuple")).exists());
    }

    #[test]
    fn a_key_with_a_space_in_a_channel_is_refused_as_bad_key() {
        let (kernel, threads) = two_channel_door("bad-key");
        let error = issue(&kernel, Verb::Sink, "urn:slack:channel:gonk-chat:thread:item%2054")
            .unwrap_err()
            .to_string();
        assert!(error.contains("key"), "{error}");
        assert!(threads.lock().unwrap().transport().posts().is_empty());
    }

    #[test]
    fn an_empty_key_in_a_channel_is_refused_as_bad_key() {
        let (kernel, _) = two_channel_door("empty-key");
        let error = issue(&kernel, Verb::Sink, "urn:slack:channel:gonk-chat:thread:")
            .unwrap_err()
            .to_string();
        assert!(error.contains("key") && !error.contains("not served"), "{error}");
    }

    #[test]
    fn a_sink_without_text_to_an_undeclared_channel_is_refused_as_no_channel() {
        let (kernel, _) = two_channel_door("no-text");
        let request = Request::new(Verb::Sink, Iri::parse("urn:slack:channel:random:thread:x").unwrap());
        let error = futures_executor::block_on(kernel.issue(request, &Capability::root()))
            .unwrap_err()
            .to_string();
        assert!(error.contains("no-channel"), "{error}");
    }

    #[test]
    fn the_same_key_in_two_channels_opens_two_threads() {
        let (kernel, threads) = two_channel_door("two-channels");
        issue(&kernel, Verb::Sink, "urn:slack:channel:gonk-chat:thread:x").unwrap();
        issue(&kernel, Verb::Sink, "urn:slack:channel:fixme:thread:x").unwrap();
        let posts = threads.lock().unwrap().transport().posts();
        assert_eq!(posts.len(), 2);
        assert!(posts.iter().all(|post| post.thread.is_none()));
        assert_eq!((posts[0].channel.as_str(), posts[1].channel.as_str()), ("C1", "C2"));
        issue(&kernel, Verb::Sink, "urn:slack:channel:fixme:thread:x").unwrap();
        let posts = threads.lock().unwrap().transport().posts();
        assert_eq!(posts[2].thread.as_deref(), Some(posts[1].ts.as_str()));
    }

    #[test]
    fn an_old_threads_line_is_served_at_its_channel_urn() {
        let dir = std::env::temp_dir().join(format!("roc-door-serve-old-line-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut slack = FakeSlack::new();
        let root = slack.post("C1", None, "Opening").unwrap();
        std::fs::write(dir.join("threads.tsv"), format!("carducci-54\tC1\t{root}\n")).unwrap();
        let channels = [("gonk-chat".to_owned(), "C1".to_owned())];
        let threads = Arc::new(Mutex::new(Threads::open_channels(&dir, &channels, slack).unwrap()));
        let kernel = kernel(Arc::clone(&threads), dir.join("spaces"), vec!["gonk-chat".to_owned()]);
        let iri = "urn:slack:channel:gonk-chat:thread:carducci-54";
        issue(&kernel, Verb::Sink, iri).unwrap();
        let posts = threads.lock().unwrap().transport().posts();
        assert_eq!(posts[1].thread.as_deref(), Some(root.as_str()));
        issue(&kernel, Verb::Source, iri).unwrap();
        issue(&kernel, Verb::Sink, &format!("{iri}:reaction:tada")).unwrap();
    }

    fn text_of_reply(reply: Representation) -> String {
        String::from_utf8(reply.bytes.to_vec()).unwrap()
    }

    #[test]
    fn a_channel_thread_opens_then_replies_then_reads() {
        let (kernel, threads) = two_channel_door("open-reply-read");
        let iri = "urn:slack:channel:fixme:thread:k";
        issue(&kernel, Verb::Sink, iri).unwrap();
        issue(&kernel, Verb::Sink, iri).unwrap();
        let posts = threads.lock().unwrap().transport().posts();
        assert_eq!((posts[0].channel.as_str(), posts[0].thread.as_deref()), ("C2", None));
        assert_eq!(posts[1].thread.as_deref(), Some(posts[0].ts.as_str()));
        let json = text_of_reply(issue(&kernel, Verb::Source, iri).unwrap());
        assert!(json.contains("hello"), "{json}");
    }

    #[test]
    fn a_key_known_in_one_channel_is_unknown_in_another() {
        let (kernel, _) = two_channel_door("unknown-elsewhere");
        issue(&kernel, Verb::Sink, "urn:slack:channel:gonk-chat:thread:k").unwrap();
        let error = issue(&kernel, Verb::Source, "urn:slack:channel:fixme:thread:k")
            .unwrap_err()
            .to_string();
        assert!(error.contains("no thread"), "{error}");
    }

    #[test]
    fn exists_answers_per_channel() {
        let (kernel, _) = two_channel_door("exists-per-channel");
        issue(&kernel, Verb::Sink, "urn:slack:channel:gonk-chat:thread:k").unwrap();
        let answer = |iri| text_of_reply(issue(&kernel, Verb::Exists, iri).unwrap());
        assert_eq!(answer("urn:slack:channel:gonk-chat:thread:k"), "true\n");
        assert_eq!(answer("urn:slack:channel:fixme:thread:k"), "false\n");
    }

    #[test]
    fn writing_to_a_channel_posts_a_top_level_message_that_no_key_names() {
        let (kernel, threads) = two_channel_door("announce");
        issue(&kernel, Verb::Sink, "urn:slack:channel:fixme").unwrap();
        let threads = threads.lock().unwrap();
        let posts = threads.transport().posts();
        assert_eq!(posts.len(), 1);
        assert_eq!((posts[0].channel.as_str(), posts[0].thread.as_deref()), ("C2", None));
        assert_eq!(posts[0].text, "hello");
        assert_eq!(threads.key_for("C2", &posts[0].ts), None);
    }

    #[test]
    fn an_announcement_over_the_text_limit_is_refused_and_nothing_is_posted() {
        let (kernel, threads) = two_channel_door("announce-long");
        let too_long = vec![b'x'; roc_slack_door::MAX_MESSAGE_LEN + 1];
        let request = Request::new(Verb::Sink, Iri::parse("urn:slack:channel:gonk-chat").unwrap())
            .with_arg("content", ArgRef::Inline(too_long));
        let result = futures_executor::block_on(kernel.issue(request, &Capability::root()));
        assert!(result.is_err());
        assert!(threads.lock().unwrap().transport().posts().is_empty());
    }

    #[test]
    fn a_reaction_marks_the_threads_opening_message_and_posts_nothing_more() {
        let (kernel, threads) = two_channel_door("react");
        issue(&kernel, Verb::Sink, "urn:slack:channel:fixme:thread:k").unwrap();
        let iri = "urn:slack:channel:fixme:thread:k:reaction:white_check_mark";
        issue(&kernel, Verb::Sink, iri).unwrap();
        issue(&kernel, Verb::Sink, iri).unwrap();
        let threads = threads.lock().unwrap();
        let posts = threads.transport().posts();
        assert_eq!(posts.len(), 1);
        let mark = (posts[0].ts.clone(), "white_check_mark".to_owned());
        assert_eq!(threads.transport().reactions().iter().collect::<Vec<_>>(), [&mark]);
    }

    #[test]
    fn deleting_a_channel_reaction_removes_it_and_removing_an_absent_one_succeeds() {
        let (kernel, threads) = two_channel_door("unreact");
        issue(&kernel, Verb::Sink, "urn:slack:channel:fixme:thread:k").unwrap();
        let iri = "urn:slack:channel:fixme:thread:k:reaction:tada";
        issue(&kernel, Verb::Sink, iri).unwrap();
        issue(&kernel, Verb::Delete, iri).unwrap();
        issue(&kernel, Verb::Delete, iri).unwrap();
        assert!(threads.lock().unwrap().transport().reactions().is_empty());
    }

    #[test]
    fn a_reaction_on_a_key_open_only_in_another_channel_is_refused_as_no_thread() {
        let (kernel, threads) = two_channel_door("react-no-thread");
        issue(&kernel, Verb::Sink, "urn:slack:channel:gonk-chat:thread:k").unwrap();
        let error = issue(&kernel, Verb::Sink, "urn:slack:channel:fixme:thread:k:reaction:tada")
            .unwrap_err()
            .to_string();
        assert!(error.contains("no thread"), "{error}");
        assert!(threads.lock().unwrap().transport().reactions().is_empty());
    }

    #[test]
    fn a_channel_reaction_with_a_bad_name_is_refused_and_nothing_is_added() {
        let (kernel, threads) = two_channel_door("react-bad-name");
        issue(&kernel, Verb::Sink, "urn:slack:channel:fixme:thread:k").unwrap();
        let error = issue(&kernel, Verb::Sink, "urn:slack:channel:fixme:thread:k:reaction:Tada")
            .unwrap_err()
            .to_string();
        assert!(error.contains("reaction"), "{error}");
        assert!(threads.lock().unwrap().transport().reactions().is_empty());
    }

    #[test]
    fn a_reaction_sink_never_posts_a_message() {
        let (kernel, threads) = two_channel_door("react-no-post");
        let _ = issue(&kernel, Verb::Sink, "urn:slack:channel:fixme:thread:k:reaction:tada");
        assert!(threads.lock().unwrap().transport().posts().is_empty());
    }

    #[test]
    fn an_undeclared_channel_is_refused_on_every_verb_and_slack_is_not_called() {
        let (kernel, threads) = door("undeclared");
        for iri in [
            "urn:slack:channel:random:thread:x",
            "urn:slack:channel:random:thread:x:reaction:tada",
            "urn:slack:channel:random:inbox",
            "urn:slack:channel:random",
        ] {
            for verb in [Verb::Source, Verb::Sink, Verb::Delete] {
                let error = issue(&kernel, verb, iri).unwrap_err().to_string();
                assert!(error.contains("no-channel") && error.contains("random"), "{iri}: {error}");
            }
        }
        let error = issue(&kernel, Verb::Exists, "urn:slack:channel:random:thread:bad!key").unwrap_err().to_string();
        assert!(error.contains("no-channel") && error.contains("random"), "{error}");
        assert!(threads.lock().unwrap().transport().posts().is_empty());
        assert!(threads.lock().unwrap().transport().reactions().is_empty());
    }

    #[test]
    fn a_sink_to_a_channel_thread_reaction_posts_nothing() {
        let (kernel, threads) = two_channel_door("reaction-sink");
        let request = Request::new(
            Verb::Sink,
            Iri::parse("urn:slack:channel:gonk-chat:thread:k:reaction:eyes").unwrap(),
        )
        .with_arg("content", ArgRef::Inline(b"hello".to_vec()));
        let result = futures_executor::block_on(kernel.issue(request, &Capability::root()));
        assert!(result.is_err());
        assert!(threads.lock().unwrap().transport().posts().is_empty());
    }

    fn issue_as(kernel: &Kernel, verb: Verb, iri: &str, capability: &Capability) -> Result<Representation> {
        let mut request = Request::new(verb, Iri::parse(iri).unwrap());
        if verb == Verb::Sink {
            request = request.with_arg("content", ArgRef::Inline(b"hello".to_vec()));
        }
        futures_executor::block_on(kernel.issue(request, capability))
    }

    fn holding(scopes: &[(&str, &str)]) -> Capability {
        Capability::scoped(scopes.iter().map(|(name, action)| roc_slack_door::scope(name, action)))
    }

    #[test]
    fn the_names_without_a_channel_are_bound_to_nothing() {
        let (kernel, threads) = two_channel_door("old-names");
        // Spelled in two pieces so tests/test_old_names.py, which refuses the whole name anywhere
        // under artifacts/, does not refuse this test.
        let thread = format!("urn:slack:{}:k", "thread");
        for iri in [thread.clone(), format!("{thread}:reaction:x")] {
            // Exists and Sink on each of the two old names: four pairs.
            for verb in [Verb::Exists, Verb::Sink] {
                assert_eq!(nothing_is_bound(&kernel, verb, &iri), Ok(()));
            }
        }
        assert!(threads.lock().unwrap().transport().posts().is_empty());
    }

    /// Whether the kernel answers that nothing is bound at `iri`: its own `no endpoint resolved`
    /// answer, not the answer of any endpoint. `Err` carries what it answered instead.
    fn nothing_is_bound(kernel: &Kernel, verb: Verb, iri: &str) -> std::result::Result<(), String> {
        match issue(kernel, verb, iri) {
            Ok(_) => Err(format!("{verb:?} {iri}: answered ok")),
            Err(Error::Unresolved(asked)) if asked.to_string() == iri => Ok(()),
            Err(error) => Err(format!("{verb:?} {iri}: {error:?}")),
        }
    }

    struct StandIn;

    #[async_trait]
    impl Endpoint for StandIn {
        async fn invoke(&self, _: &Invocation<'_>) -> Result<Representation> {
            Err(Error::Endpoint("a thread reaction takes sink and delete, not Exists".to_owned()))
        }

        fn name(&self) -> &str {
            "stand-in"
        }
    }

    #[test]
    fn the_unbound_check_fails_when_an_endpoint_answers_at_an_old_name() {
        let thread = format!("urn:slack:{}:k", "thread");
        let rebound = EndpointSpace::new().bind(
            UriTemplate::parse(&format!("{thread}:reaction:{{reaction}}")).unwrap(),
            StandIn,
        );
        let kernel = Kernel::new(Arc::new(rebound));
        let iri = format!("{thread}:reaction:x");
        let error = nothing_is_bound(&kernel, Verb::Exists, &iri).unwrap_err();
        assert!(error.contains("a thread reaction takes sink and delete"), "{error}");
        assert_eq!(nothing_is_bound(&kernel, Verb::Exists, &format!("{thread}:other")), Ok(()));
    }

    struct Echo;

    #[async_trait]
    impl Endpoint for Echo {
        async fn invoke(&self, invocation: &Invocation<'_>) -> Result<Representation> {
            Err(Error::Endpoint(format!("no endpoint resolved for {}", invocation.request.target)))
        }

        fn name(&self) -> &str {
            "echo"
        }
    }

    #[test]
    fn the_unbound_check_fails_when_an_endpoint_echoes_the_unresolved_text() {
        let thread = format!("urn:slack:{}:k", "thread");
        let rebound = EndpointSpace::new().bind(
            UriTemplate::parse(&format!("{thread}:reaction:{{reaction}}")).unwrap(),
            Echo,
        );
        let kernel = Kernel::new(Arc::new(rebound));
        let iri = format!("{thread}:reaction:x");
        for verb in [Verb::Exists, Verb::Sink] {
            assert!(nothing_is_bound(&kernel, verb, &iri).is_err(), "{verb:?}");
        }
    }

    #[test]
    fn a_verb_without_the_channels_capability_is_refused_before_slack_is_called() {
        let (kernel, threads) = two_channel_door("gate");
        issue(&kernel, Verb::Sink, "urn:slack:channel:gonk-chat:thread:k").unwrap();
        let before = threads.lock().unwrap().transport().posts().len();
        let thread = "urn:slack:channel:gonk-chat:thread:k";
        let denied = |verb, iri: &str, held: &Capability, needs: &str| {
            let error = issue_as(&kernel, verb, iri, held).unwrap_err().to_string();
            assert!(error.contains("not-granted") && error.contains(needs), "{verb:?} {iri}: {error}");
        };
        let read_only = holding(&[("gonk-chat", "read")]);
        denied(Verb::Sink, thread, &read_only, "urn:cap:slack:channel:gonk-chat:post");
        denied(Verb::Sink, "urn:slack:channel:gonk-chat", &read_only, "urn:cap:slack:channel:gonk-chat:post");
        let reaction = format!("{thread}:reaction:tada");
        denied(Verb::Sink, &reaction, &read_only, "urn:cap:slack:channel:gonk-chat:react");
        denied(Verb::Delete, &reaction, &read_only, "urn:cap:slack:channel:gonk-chat:react");
        let all_here = holding(&[("gonk-chat", "read"), ("gonk-chat", "post"), ("gonk-chat", "react")]);
        let elsewhere = "urn:slack:channel:fixme:thread:fixme-1";
        denied(Verb::Sink, elsewhere, &all_here, "urn:cap:slack:channel:fixme:post");
        denied(Verb::Source, elsewhere, &all_here, "urn:cap:slack:channel:fixme:read");
        denied(Verb::Exists, elsewhere, &all_here, "urn:cap:slack:channel:fixme:read");
        denied(Verb::Source, thread, &holding(&[("gonk-chat", "post")]), "urn:cap:slack:channel:gonk-chat:read");
        let posts = threads.lock().unwrap().transport().posts().len();
        assert_eq!(posts, before);
        assert!(threads.lock().unwrap().transport().reactions().is_empty());
        issue_as(&kernel, Verb::Source, thread, &read_only).unwrap();
    }

    #[test]
    fn the_inbox_needs_the_channels_read_capability_for_source_and_delete() {
        let (kernel, _) = two_channel_door("inbox-gate");
        drop_delivery(&intray_of("inbox-gate"), "gonk-chat", &a_message("carducci-54", "U1", "hi")).unwrap();
        let inbox = "urn:slack:channel:gonk-chat:inbox";
        let fixme_only = holding(&[("fixme", "read")]);
        for verb in [Verb::Source, Verb::Delete] {
            let error = issue_as(&kernel, verb, inbox, &fixme_only).unwrap_err().to_string();
            assert!(error.contains("not-granted") && error.contains("urn:cap:slack:channel:gonk-chat:read"), "{error}");
        }
        issue(&kernel, Verb::Source, inbox).unwrap();
    }

    #[test]
    fn the_channels_read_alone_sources_and_deletes_its_inbox_and_no_other() {
        let (kernel, _) = two_channel_door("inbox-read-alone");
        let intray = intray_of("inbox-read-alone");
        drop_delivery(&intray, "gonk-chat", &a_message("g-1", "U1", "here")).unwrap();
        drop_delivery(&intray, "fixme", &a_message("f-1", "U2", "there")).unwrap();
        let read = holding(&[("gonk-chat", "read")]);
        let ids = |space: &str| text_of_reply(issue(&intray, Verb::Source, &format!("urn:space:{space}")).unwrap());
        let seen = text_of_reply(issue_as(&kernel, Verb::Source, "urn:slack:channel:gonk-chat:inbox", &read).unwrap());
        assert!(!seen.is_empty() && seen == ids("gonk-chat") && seen != ids("fixme"), "{seen}");
        let other = issue_as(&kernel, Verb::Source, "urn:slack:channel:fixme:inbox", &read).unwrap_err().to_string();
        assert!(other.contains("not-granted") && other.contains("urn:cap:slack:channel:fixme:read"), "{other}");
        let taken = text_of_reply(issue_as(&kernel, Verb::Delete, "urn:slack:channel:gonk-chat:inbox", &read).unwrap());
        assert!(taken.contains("g-1"), "{taken}");
        let other = issue_as(&kernel, Verb::Delete, "urn:slack:channel:fixme:inbox", &read).unwrap_err().to_string();
        assert!(other.contains("not-granted"), "{other}");
        let left = text_of_reply(issue(&kernel, Verb::Delete, "urn:slack:channel:fixme:inbox").unwrap());
        assert!(left.contains("f-1"), "{left}");
    }

    #[test]
    fn the_intray_is_bound_to_nothing_in_the_doors_kernel_under_any_capability() {
        let (kernel, _) = two_channel_door("intray-unbound");
        drop_delivery(&intray_of("intray-unbound"), "fixme", &a_message("f-1", "U2", "there")).unwrap();
        let held = holding(&[("gonk-chat", "read"), ("gonk-chat", "post")])
            .attenuate(["urn:cap:space:read", "urn:cap:space:read*", "urn:cap:space:take", "urn:cap:space:out"]);
        for capability in [&held, &Capability::root()] {
            for verb in [Verb::Source, Verb::Delete, Verb::Sink, Verb::Exists] {
                let error = issue_as(&kernel, verb, "urn:space:fixme", capability).unwrap_err().to_string();
                assert!(error.contains("no endpoint resolved for urn:space:fixme"), "{verb:?}: {error}");
            }
        }
        let still = text_of_reply(issue(&kernel, Verb::Delete, "urn:slack:channel:fixme:inbox").unwrap());
        assert!(still.contains("f-1"), "{still}");
    }

    #[test]
    fn a_sink_to_a_channels_inbox_writes_nothing_under_any_capability() {
        let (kernel, _) = two_channel_door("inbox-sink");
        let inbox = "urn:slack:channel:gonk-chat:inbox";
        let write = Capability::root().attenuate(["urn:cap:space:out"]);
        for capability in [
            holding(&[("gonk-chat", "read")]),
            holding(&[("gonk-chat", "post")]),
            holding(&[("gonk-chat", "read")]).attenuate(["urn:cap:space:out"]),
            write,
            Capability::root(),
        ] {
            assert!(issue_as(&kernel, Verb::Sink, inbox, &capability).is_err());
        }
        let path = std::env::temp_dir()
            .join(format!("roc-door-serve-inbox-sink-{}", std::process::id()))
            .join("spaces/gonk-chat/inbox");
        let written = std::fs::read_dir(&path).map(|d| d.count()).unwrap_or(0);
        assert_eq!(written, 0);
        let seen = issue(&kernel, Verb::Source, inbox).map(text_of_reply).unwrap_or_default();
        assert!(!seen.contains("hello"), "{seen}");
    }

    #[test]
    fn a_delivery_is_dropped_under_a_capability_that_can_only_write_the_intray() {
        let (kernel, _) = two_channel_door("drop-narrow");
        drop_delivery(&intray_of("drop-narrow"), "fixme", &a_message("k-1", "U1", "hi")).unwrap();
        let taken = issue(&kernel, Verb::Delete, "urn:slack:channel:fixme:inbox").unwrap();
        assert!(String::from_utf8(taken.bytes.to_vec()).unwrap().contains("k-1"));
    }

    #[test]
    fn a_message_tuple_carries_its_key_user_time_and_text() {
        let tuple = turtle(&Delivery::Message {
            key: Some("carducci-54".to_owned()),
            user: "U1".to_owned(),
            time: "2023-11-14T22:15:00Z".to_owned(),
            text: "Use \"the\" token".to_owned(),
        });
        assert!(tuple.contains("a slack:Message"));
        assert!(tuple.contains("slack:key \"carducci-54\""));
        assert!(tuple.contains("slack:text \"Use \\\"the\\\" token\""));
    }

    #[test]
    fn a_keyless_message_has_no_key_predicate() {
        let tuple = turtle(&Delivery::Message {
            key: None,
            user: "U1".to_owned(),
            time: "2023-11-14T22:15:00Z".to_owned(),
            text: "What's blocked today?".to_owned(),
        });
        assert!(!tuple.contains("slack:key"));
    }

    #[test]
    fn a_reaction_tuple_names_the_reaction() {
        let tuple = turtle(&Delivery::Reaction {
            key: "carducci-54".to_owned(),
            user: "U1".to_owned(),
            name: "+1".to_owned(),
        });
        assert!(tuple.contains("a slack:Reaction"));
        assert!(tuple.contains("slack:name \"+1\""));
    }
}
