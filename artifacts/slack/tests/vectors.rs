//! Runs every scenario vector in `openspec/vectors.json` against the library and its fake.
//!
//! A vector is a list of steps, run in order against a fresh door. A step either acts (`write`,
//! `read`, `react`, `feed`, `fail_next`, `restart`) or checks (`inbox`, `posts`, `reactions`,
//! `describe`). An `expect` is the outcome a call should have: `"ok"`, a refusal's code such as
//! `"bad-key"`, or `"unavailable"` / `"unavailable:<seconds>"`. In an expected message or
//! delivery only the fields written are compared; `"$root"` stands for the opening message's
//! timestamp the vector has seen. A vector for a scenario the library cannot reach (it reads
//! the door's configuration) is marked `"door": true` and skipped here; the door's own tests
//! cover it.

use std::fs;
use std::path::{Path, PathBuf};

use roc_slack_door::{
    descriptions, to_json, Delivery, Event, FakeSlack, Grants, Inbox, SlackTransport, ThreadError,
    Threads, TransportError, POST, READ, REACT,
};
use serde_json::{json, Value};

const CHANNEL: &str = "C1";
const BOT: &str = "UBOT";

/// The door under test: its thread map, its inbox, and what the vector has learned so far.
struct World {
    dir: PathBuf,
    threads: Threads<FakeSlack>,
    inbox: Inbox,
    /// The opening message's timestamp, once the vector has opened a thread.
    root: Option<String>,
    /// How many events the vector has fed, to number them.
    fed: usize,
}

impl World {
    fn open(dir: &Path) -> World {
        World {
            dir: dir.to_owned(),
            threads: Threads::open(dir, CHANNEL, FakeSlack::new()).unwrap(),
            inbox: Inbox::open(dir, CHANNEL, BOT, ["U1"]).unwrap(),
            root: None,
            fed: 0,
        }
    }

    fn restart(&mut self) {
        self.threads = Threads::open(&self.dir, CHANNEL, FakeSlack::new()).unwrap();
        self.inbox = Inbox::open(&self.dir, CHANNEL, BOT, ["U1"]).unwrap();
    }

    /// Replaces `"$root"` with the opening message's timestamp.
    fn resolve(&self, expected: &Value) -> Value {
        match expected {
            Value::String(s) if s == "$root" => json!(self.root.clone().unwrap_or_default()),
            Value::Object(map) => Value::Object(
                map.iter().map(|(k, v)| (k.clone(), self.resolve(v))).collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.resolve(v)).collect()),
            other => other.clone(),
        }
    }
}

fn grants(step: &Value) -> Grants {
    match step.get("as").and_then(Value::as_str) {
        Some("read") => Grants::new([READ]),
        Some("post") => Grants::new([POST]),
        Some("react") => Grants::new([REACT]),
        _ => Grants::all(),
    }
}

/// The outcome of a call, in the words an `expect` uses.
fn outcome<T>(result: Result<T, ThreadError>) -> String {
    match result {
        Ok(_) => "ok".to_owned(),
        Err(ThreadError::Refused(refusal)) => refusal.code().to_owned(),
        Err(ThreadError::Unavailable {
            retry_after_secs: Some(secs),
        }) => format!("unavailable:{secs}"),
        Err(ThreadError::Unavailable {
            retry_after_secs: None,
        }) => "unavailable".to_owned(),
        Err(ThreadError::Transport(error)) => format!("rejected:{error:?}"),
        Err(ThreadError::State(error)) => format!("state:{error}"),
    }
}

/// Whether `actual` has every field `expected` names, with equal values.
fn subset(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => e
            .iter()
            .all(|(key, value)| a.get(key).is_some_and(|found| subset(found, value))),
        _ => actual == expected,
    }
}

fn expect_outcome(what: &str, actual: &str, step: &Value) -> Result<(), String> {
    let expected = step["expect"].as_str().ok_or("expect is a string here")?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{what}: expected {expected:?}, got {actual:?}"))
    }
}

/// Compares a list of objects in order: the same length, each element a subset.
fn expect_list(what: &str, actual: &[Value], expected: &Value) -> Result<(), String> {
    let expected = expected.as_array().ok_or("expected a list")?;
    if actual.len() != expected.len() {
        return Err(format!(
            "{what}: expected {} item(s), got {}: {}",
            expected.len(),
            actual.len(),
            Value::Array(actual.to_vec())
        ));
    }
    for (index, (have, want)) in actual.iter().zip(expected).enumerate() {
        if !subset(have, want) {
            return Err(format!("{what}[{index}]: expected {want}, got {have}"));
        }
    }
    Ok(())
}

fn delivery_json(delivery: &Delivery) -> Value {
    match delivery {
        Delivery::Message {
            key,
            user,
            time,
            text,
        } => json!({ "key": key, "user": user, "time": time, "text": text }),
        Delivery::Reaction { key, user, name } => json!({ "key": key, "user": user, "reaction": name }),
    }
}

fn run_step(world: &mut World, step: &Value) -> Result<(), String> {
    let object = step.as_object().ok_or("a step is an object")?;
    let (kind, args) = object
        .iter()
        .find(|(key, _)| key.as_str() != "expect")
        .ok_or("a step names an action")?;
    match kind.as_str() {
        "write" => {
            let key = args["key"].as_str().unwrap();
            let text = args["text"].as_str().unwrap();
            let was_open = world.threads.exists(&Grants::all(), key).unwrap_or(false);
            let result = world.threads.write(&grants(args), key, text);
            let ok = result.is_ok();
            let actual = outcome(result);
            if ok && !was_open {
                world.root = world
                    .threads
                    .transport()
                    .posts()
                    .last()
                    .map(|post| post.ts.clone());
            }
            expect_outcome("write", &actual, step)
        }
        "read" => {
            let key = args["key"].as_str().unwrap();
            match &step["expect"] {
                Value::String(_) => {
                    let actual = outcome(world.threads.read(&grants(args), key));
                    expect_outcome("read", &actual, step)
                }
                expected => {
                    let entries = world
                        .threads
                        .read(&grants(args), key)
                        .map_err(|e| format!("read: {}", outcome::<()>(Err(e))))?;
                    let messages: Vec<Value> = serde_json::from_str(&to_json(&entries)).unwrap();
                    expect_list("read", &messages, expected)
                }
            }
        }
        "react" => {
            let key = args["key"].as_str().unwrap();
            let name = args["name"].as_str().unwrap();
            let add = args["add"].as_bool().unwrap_or(true);
            let actual = outcome(world.threads.react(&grants(args), key, name, add));
            expect_outcome("react", &actual, step)
        }
        "feed" => {
            world.fed += 1;
            let n = world.fed;
            let id = args["id"]
                .as_str()
                .map_or_else(|| format!("Ev{n}"), str::to_owned);
            let event = if let Some(message) = args.get("message") {
                let thread = match message.get("in").and_then(Value::as_str) {
                    Some(_) => world.root.clone(),
                    None => None,
                };
                Event::Message {
                    id,
                    channel: CHANNEL.to_owned(),
                    user: message["user"].as_str().unwrap_or_default().to_owned(),
                    ts: format!("1700000{:03}.000001", 100 + n),
                    thread,
                    text: message["text"].as_str().unwrap_or_default().to_owned(),
                }
            } else {
                let reaction = &args["reaction"];
                Event::Reaction {
                    id,
                    channel: CHANNEL.to_owned(),
                    user: reaction["user"].as_str().unwrap_or_default().to_owned(),
                    message: world.root.clone().unwrap_or_default(),
                    name: reaction["name"].as_str().unwrap_or_default().to_owned(),
                }
            };
            world.threads.transport_mut().feed(event);
            Ok(())
        }
        "inbox" => {
            let events = world.threads.transport_mut().take_events();
            let mut deliveries = Vec::new();
            for event in events {
                if let Some(delivery) = world
                    .inbox
                    .deliver(event, &mut world.threads)
                    .map_err(|e| format!("inbox: {e}"))?
                {
                    deliveries.push(delivery_json(&delivery));
                }
            }
            expect_list("inbox", &deliveries, &world.resolve(args))
        }
        "posts" => {
            let posts: Vec<Value> = world
                .threads
                .transport()
                .posts()
                .into_iter()
                .map(|post| json!({ "text": post.text, "thread": post.thread }))
                .collect();
            expect_list("posts", &posts, &world.resolve(args))
        }
        "reactions" => {
            let count = world.threads.transport().reactions().len();
            if Some(count as u64) == args.as_u64() {
                Ok(())
            } else {
                Err(format!("reactions: expected {args}, got {count}"))
            }
        }
        "restart" => {
            world.restart();
            Ok(())
        }
        "fail_next" => {
            let failure = match args.as_str().unwrap_or_default() {
                text if text.starts_with("rate-limited:") => TransportError::RateLimited {
                    retry_after_secs: text["rate-limited:".len()..].parse().unwrap(),
                },
                "unavailable" => TransportError::Unavailable("outage".to_owned()),
                other => return Err(format!("unknown failure {other:?}")),
            };
            world.threads.transport_mut().fail_next(failure);
            Ok(())
        }
        "describe" => {
            let resource = args.as_str().unwrap_or_default();
            let found = descriptions()
                .into_iter()
                .find(|meta| meta.resource == resource)
                .ok_or_else(|| format!("no description for {resource}"))?;
            let text = format!("{} {}", found.resource, found.description);
            if text.contains("xoxb-") || text.contains("xapp-") {
                Err(format!("{resource}: the description carries a token"))
            } else {
                Ok(())
            }
        }
        other => Err(format!("unknown step {other:?}")),
    }
}

/// Runs one vector. `Ok(true)` ran, `Ok(false)` was skipped (a door-only vector).
fn run_vector(root: &Path, number: usize, vector: &Value) -> Result<bool, String> {
    if vector.get("door").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(false);
    }
    let id = vector["id"].as_str().unwrap();
    let dir = root.join(format!("vector-{number}"));
    let _ = fs::remove_dir_all(&dir);
    let mut world = World::open(&dir);
    for (index, step) in vector["steps"].as_array().unwrap().iter().enumerate() {
        run_step(&mut world, step).map_err(|why| format!("{id} step {index}: {why}"))?;
    }
    Ok(true)
}

#[test]
fn every_scenario_vector_passes_against_the_fake() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let vectors: Value = serde_json::from_str(
        &fs::read_to_string(manifest.join("../../openspec/vectors.json")).unwrap(),
    )
    .unwrap();
    let root = std::env::temp_dir().join(format!("roc-slack-door-vectors-{}", std::process::id()));
    let mut ran = 0;
    let mut skipped = Vec::new();
    for (number, vector) in vectors["scenarios"].as_array().unwrap().iter().enumerate() {
        match run_vector(&root, number, vector) {
            Ok(true) => ran += 1,
            Ok(false) => skipped.push(vector["id"].as_str().unwrap().to_owned()),
            Err(why) => panic!("{why}"),
        }
    }
    assert!(ran > 0, "no vector ran");
    assert_eq!(skipped.len(), 1, "only the door-only vector is skipped: {skipped:?}");
    let _ = fs::remove_dir_all(&root);
}
