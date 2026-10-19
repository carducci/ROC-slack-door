//! Socket Mode: the door's outbound WebSocket to Slack, so the door needs no public address. It
//! opens a connection with the app-level token, acknowledges every envelope Slack sends, turns
//! the events it hears into `Event`s, and reconnects with backoff when the connection drops.
//! Nothing here prints a token or the connection's URL, which carries a ticket.

use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use roc_slack_door::Event;
use serde_json::{json, Value};
use tungstenite::Message;

const OPEN_URL: &str = "https://slack.com/api/apps.connections.open";
const MAX_BACKOFF_SECS: u64 = 60;

/// Keeps a Socket Mode connection open for the life of the process, sending each event heard
/// to `events`. It never returns; a failed connection is retried after `backoff`.
pub fn run(app_token: &str, events: &Sender<Event>) -> ! {
    let mut attempt = 0;
    loop {
        match connect_and_listen(app_token, events, &mut attempt) {
            Ok(()) => eprintln!("roc-slack-door: Slack asked the door to reconnect"),
            Err(why) => eprintln!("roc-slack-door: connection lost: {why}"),
        }
        thread::sleep(backoff(attempt));
        attempt = attempt.saturating_add(1);
    }
}

/// How long to wait before retry number `attempt`: one second, doubling, capped at a minute.
pub fn backoff(attempt: u32) -> Duration {
    let secs = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
    Duration::from_secs(secs.min(MAX_BACKOFF_SECS))
}

/// Opens one connection and listens until it drops. `attempt` is reset to zero once Slack's
/// `hello` arrives, so a connection that worked does not inherit the backoff of earlier failures.
fn connect_and_listen(
    app_token: &str,
    events: &Sender<Event>,
    attempt: &mut u32,
) -> Result<(), String> {
    let url = open_url(app_token)?;
    let (mut socket, _) = tungstenite::connect(url.as_str())
        .map_err(|e| format!("WebSocket connect failed: {e}"))?;
    loop {
        let message = socket.read().map_err(|e| format!("read failed: {e}"))?;
        let Message::Text(text) = message else {
            continue;
        };
        let envelope: Value =
            serde_json::from_str(&text).map_err(|e| format!("unreadable envelope: {e}"))?;
        if let Some(id) = envelope["envelope_id"].as_str() {
            let ack = json!({ "envelope_id": id }).to_string();
            socket
                .send(Message::Text(ack))
                .map_err(|e| format!("acknowledge failed: {e}"))?;
        }
        match envelope["type"].as_str() {
            Some("hello") => *attempt = 0,
            Some("disconnect") => return Ok(()),
            Some("events_api") => {
                if let Some(event) = event_from(&envelope["payload"]) {
                    // The receiver is gone only when the door is stopping; nothing to do then.
                    let _ = events.send(event);
                }
            }
            _ => {}
        }
    }
}

/// Asks Slack for a connection URL with the app-level token.
fn open_url(app_token: &str) -> Result<String, String> {
    let answer = ureq::post(OPEN_URL)
        .set("Authorization", &format!("Bearer {app_token}"))
        .call()
        .map_err(|e| format!("apps.connections.open failed: {e}"))?;
    let value: Value = answer
        .into_json()
        .map_err(|e| format!("apps.connections.open answer unreadable: {e}"))?;
    if value["ok"].as_bool() != Some(true) {
        let error = value["error"].as_str().unwrap_or("unknown_error");
        return Err(format!("apps.connections.open refused: {error}"));
    }
    value["url"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "apps.connections.open gave no url".to_owned())
}

/// The event an `events_api` payload carries, if it is one the door hears: a message or a
/// reaction. Edits and deletions (messages with a subtype) are not heard.
fn event_from(payload: &Value) -> Option<Event> {
    let id = payload["event_id"].as_str()?.to_owned();
    let event = &payload["event"];
    let text_of = |field: &str| event[field].as_str().map(str::to_owned);
    match event["type"].as_str()? {
        "message" => {
            if event.get("subtype").is_some() {
                return None;
            }
            Some(Event::Message {
                id,
                channel: text_of("channel")?,
                user: text_of("user").unwrap_or_default(),
                ts: text_of("ts")?,
                thread: text_of("thread_ts"),
                text: text_of("text").unwrap_or_default(),
            })
        }
        "reaction_added" => Some(Event::Reaction {
            id,
            channel: event["item"]["channel"].as_str()?.to_owned(),
            user: text_of("user").unwrap_or_default(),
            message: event["item"]["ts"].as_str()?.to_owned(),
            name: text_of("reaction")?,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backoff_doubles_from_one_second_and_stops_at_a_minute() {
        assert_eq!(backoff(0), Duration::from_secs(1));
        assert_eq!(backoff(3), Duration::from_secs(8));
        assert_eq!(backoff(10), Duration::from_secs(60));
        assert_eq!(backoff(200), Duration::from_secs(60));
    }

    #[test]
    fn a_reply_in_a_thread_becomes_a_message_event() {
        let payload = json!({
            "event_id": "Ev1",
            "event": {
                "type": "message", "channel": "C1", "user": "U1",
                "ts": "1700000100.000001", "thread_ts": "1700000000.000001",
                "text": "Which token?"
            }
        });
        assert_eq!(
            event_from(&payload),
            Some(Event::Message {
                id: "Ev1".to_owned(),
                channel: "C1".to_owned(),
                user: "U1".to_owned(),
                ts: "1700000100.000001".to_owned(),
                thread: Some("1700000000.000001".to_owned()),
                text: "Which token?".to_owned(),
            })
        );
    }

    #[test]
    fn a_bot_message_has_no_user_and_an_edit_is_not_heard() {
        let bot = json!({ "event_id": "Ev2", "event": {
            "type": "message", "channel": "C1", "bot_id": "B1", "ts": "1.0", "text": "x" } });
        assert!(matches!(event_from(&bot), Some(Event::Message { ref user, .. }) if user.is_empty()));
        let edit = json!({ "event_id": "Ev3", "event": {
            "type": "message", "subtype": "message_changed", "channel": "C1", "ts": "1.0" } });
        assert_eq!(event_from(&edit), None);
    }

    #[test]
    fn a_reaction_becomes_a_reaction_event_on_the_reacted_message() {
        let payload = json!({
            "event_id": "Ev4",
            "event": {
                "type": "reaction_added", "user": "U1", "reaction": "+1",
                "item": { "type": "message", "channel": "C1", "ts": "1700000000.000001" }
            }
        });
        assert_eq!(
            event_from(&payload),
            Some(Event::Reaction {
                id: "Ev4".to_owned(),
                channel: "C1".to_owned(),
                user: "U1".to_owned(),
                message: "1700000000.000001".to_owned(),
                name: "+1".to_owned(),
            })
        );
    }
}
