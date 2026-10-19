//! The Slack Web API over HTTPS, behind `SlackTransport`. Every call is a POST with a JSON body,
//! authorised by the bot token in the `Authorization` header. The token is never put in an
//! error or a log line: errors carry only what Slack answered.

use std::time::Duration;

use roc_slack_door::{Event, Message, SlackTransport, TransportError};
use serde_json::{json, Value};

const BASE_URL: &str = "https://slack.com/api";
const TIMEOUT: Duration = Duration::from_secs(20);

/// The Web API, speaking as the door's bot.
pub struct WebApi {
    agent: ureq::Agent,
    base_url: String,
    token: String,
}

impl WebApi {
    pub fn new(bot_token: &str) -> Self {
        WebApi {
            agent: ureq::AgentBuilder::new().timeout(TIMEOUT).build(),
            base_url: BASE_URL.to_owned(),
            token: bot_token.to_owned(),
        }
    }

    /// One Web API method. Slack answers HTTP 200 with `ok`; a false `ok` is a refusal with the
    /// error Slack gave. HTTP 429 is a rate limit and 5xx an outage; both are transient.
    fn call(&self, method: &str, body: Value) -> Result<Value, TransportError> {
        let url = format!("{}/{method}", self.base_url);
        let response = self
            .agent
            .post(&url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .send_json(body);
        match response {
            Ok(response) => {
                let value: Value = response
                    .into_json()
                    .map_err(|e| TransportError::Unavailable(format!("unreadable answer: {e}")))?;
                answer(value)
            }
            Err(ureq::Error::Status(429, response)) => Err(TransportError::RateLimited {
                retry_after_secs: response
                    .header("Retry-After")
                    .and_then(|secs| secs.parse().ok())
                    .unwrap_or(1),
            }),
            Err(ureq::Error::Status(code, _)) if code >= 500 => {
                Err(TransportError::Unavailable(format!("HTTP {code}")))
            }
            Err(ureq::Error::Status(code, _)) => {
                Err(TransportError::Rejected(format!("HTTP {code}")))
            }
            Err(ureq::Error::Transport(transport)) => {
                Err(TransportError::Unavailable(transport.to_string()))
            }
        }
    }
}

/// Reads Slack's `ok` answer: the value when it is true, Slack's error when it is false.
fn answer(value: Value) -> Result<Value, TransportError> {
    if value["ok"].as_bool() == Some(true) {
        Ok(value)
    } else {
        let error = value["error"].as_str().unwrap_or("unknown_error");
        Err(TransportError::Rejected(error.to_owned()))
    }
}

/// A message from `conversations.replies`. A message from a bot has a `bot_id` and no `user`.
fn message_from(value: &Value) -> Option<Message> {
    let ts = value["ts"].as_str()?.to_owned();
    let text = value["text"].as_str().unwrap_or_default().to_owned();
    let (user, bot) = match value["user"].as_str() {
        Some(user) => (user.to_owned(), false),
        None => (value["bot_id"].as_str()?.to_owned(), true),
    };
    Some(Message { ts, user, bot, text })
}

/// Treats an answer that already says the work is done as success, so a repeated reaction or
/// removal changes nothing and does not fail.
fn already_done(result: Result<Value, TransportError>, done: &str) -> Result<(), TransportError> {
    match result {
        Ok(_) => Ok(()),
        Err(TransportError::Rejected(why)) if why == done => Ok(()),
        Err(other) => Err(other),
    }
}

impl SlackTransport for WebApi {
    fn post(
        &mut self,
        channel: &str,
        thread: Option<&str>,
        text: &str,
    ) -> Result<String, TransportError> {
        let mut body = json!({ "channel": channel, "text": text });
        if let Some(thread) = thread {
            body["thread_ts"] = json!(thread);
        }
        let value = self.call("chat.postMessage", body)?;
        value["ts"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| TransportError::Unavailable("no timestamp in the answer".to_owned()))
    }

    fn react(
        &mut self,
        channel: &str,
        message: &str,
        name: &str,
        add: bool,
    ) -> Result<(), TransportError> {
        let body = json!({ "channel": channel, "timestamp": message, "name": name });
        if add {
            already_done(self.call("reactions.add", body), "already_reacted")
        } else {
            already_done(self.call("reactions.remove", body), "no_reaction")
        }
    }

    fn replies(&mut self, channel: &str, root: &str) -> Result<Vec<Message>, TransportError> {
        let value = self.call("conversations.replies", json!({ "channel": channel, "ts": root }))?;
        Ok(value["messages"]
            .as_array()
            .map(|messages| messages.iter().filter_map(message_from).collect())
            .unwrap_or_default())
    }

    /// Events arrive over the Socket Mode connection (slack-resources 4.3), not from this
    /// transport; until that is built there are none.
    fn take_events(&mut self) -> Vec<Event> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bot_message_has_its_bot_id_as_the_author() {
        let value = json!({ "ts": "1700000000.000100", "bot_id": "B1", "text": "Opening" });
        let message = message_from(&value).unwrap();
        assert!(message.bot);
        assert_eq!(message.user, "B1");
        assert_eq!(message.text, "Opening");
    }

    #[test]
    fn a_person_is_not_a_bot() {
        let value = json!({ "ts": "1700000100.000001", "user": "U1", "text": "Which token?" });
        let message = message_from(&value).unwrap();
        assert!(!message.bot);
        assert_eq!(message.user, "U1");
    }

    #[test]
    fn a_false_ok_is_slacks_error() {
        let refused = answer(json!({ "ok": false, "error": "missing_scope" }));
        assert_eq!(refused, Err(TransportError::Rejected("missing_scope".to_owned())));
    }

    #[test]
    fn a_repeated_reaction_is_success() {
        let already = Err(TransportError::Rejected("already_reacted".to_owned()));
        assert_eq!(already_done(already, "already_reacted"), Ok(()));
        let other = Err(TransportError::Rejected("invalid_name".to_owned()));
        assert_eq!(
            already_done(other, "already_reacted"),
            Err(TransportError::Rejected("invalid_name".to_owned()))
        );
    }
}
