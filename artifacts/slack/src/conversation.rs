//! A conversation as a caller reads it: who said what, and when, in UTC. Slack's ids for the
//! channel and for messages stay behind; the Slack user id of an author is kept.

use crate::Message;

/// One message of a conversation, as a caller reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub user: String,
    pub bot: bool,
    /// The time in UTC, as `2023-11-14T22:13:20Z`.
    pub time: String,
    pub text: String,
}

impl From<&Message> for Entry {
    fn from(message: &Message) -> Self {
        Entry {
            user: message.user.clone(),
            bot: message.bot,
            time: utc(&message.ts),
            text: message.text.clone(),
        }
    }
}

/// Renders a conversation as a JSON array, one object per message.
pub fn to_json(entries: &[Entry]) -> String {
    let items: Vec<String> = entries
        .iter()
        .map(|entry| {
            format!(
                "{{\"user\":{},\"bot\":{},\"time\":{},\"text\":{}}}",
                json_string(&entry.user),
                entry.bot,
                json_string(&entry.time),
                json_string(&entry.text),
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// The time in UTC of a Slack timestamp such as `1700000000.000100`.
pub(crate) fn utc(ts: &str) -> String {
    let secs: i64 = ts.split('.').next().and_then(|whole| whole.parse().ok()).unwrap_or(0);
    let (year, month, day) = civil(secs.div_euclid(86_400));
    let in_day = secs.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        in_day / 3600,
        in_day % 3600 / 60,
        in_day % 60,
    )
}

/// The civil date of a count of days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = (if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 }) as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slack_timestamp_reads_as_utc() {
        assert_eq!(utc("1700000000.000100"), "2023-11-14T22:13:20Z");
        assert_eq!(utc("0.000001"), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn json_escapes_quotes_backslashes_and_newlines() {
        let entry = Entry {
            user: "U1".to_owned(),
            bot: false,
            time: "1970-01-01T00:00:00Z".to_owned(),
            text: "say \"hi\"\\\nthere".to_owned(),
        };
        assert_eq!(
            to_json(&[entry]),
            "[{\"user\":\"U1\",\"bot\":false,\"time\":\"1970-01-01T00:00:00Z\",\"text\":\"say \\\"hi\\\"\\\\\\nthere\"}]"
        );
    }
}
