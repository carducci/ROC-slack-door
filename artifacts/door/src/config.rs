//! The host's configuration for the door. Values come from a `KEY=VALUE` file the host names,
//! and environment variables of the same names override it. A start with anything missing or
//! wrong is refused, naming each problem. Nothing here is printed: the tokens are redacted in
//! `Debug`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

pub const BOT_TOKEN: &str = "SLACK_BOT_TOKEN";
pub const APP_TOKEN: &str = "SLACK_APP_TOKEN";
pub const CHANNELS: &str = "SLACK_CHANNELS";
pub const STATE_DIR: &str = "ROC_STATE_DIR";

/// Settings that no longer exist, each with what replaces it. Setting one is refused.
const RETIRED: [(&str, &str); 3] = [
    ("SLACK_CHANNEL", "SLACK_CHANNELS and SLACK_CHANNEL_<NAME>_ID"),
    ("SLACK_ALLOWED_USERS", "SLACK_CHANNEL_<NAME>_SPEAKERS"),
    ("ROC_INBOX_SPACE", "the channel's own inbox, urn:slack:channel:<name>:inbox"),
];

/// One channel the host declares: its name, Slack id, and the Slack user ids allowed to speak.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelConfig {
    pub name: String,
    pub id: String,
    pub speakers: Vec<String>,
}

/// Everything the door needs from its host.
#[derive(Clone)]
pub struct Config {
    pub bot_token: String,
    pub app_token: String,
    pub channels: Vec<ChannelConfig>,
    pub state_dir: PathBuf,
}

/// Why a start is refused: one message per problem, each naming the setting or channel.
#[derive(Debug, PartialEq, Eq)]
pub struct Refused(pub Vec<String>);

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("bot_token", &"<redacted>")
            .field("app_token", &"<redacted>")
            .field("channels", &self.channels)
            .field("state_dir", &self.state_dir)
            .finish()
    }
}

/// A channel name is 1 to 40 lowercase letters, digits and `-`.
fn valid_name(name: &str) -> bool {
    (1..=40).contains(&name.len())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The prefix of a channel's keys: its name uppercased, with `-` changed to `_`.
fn key_prefix(name: &str) -> String {
    format!("SLACK_CHANNEL_{}", name.to_ascii_uppercase().replace('-', "_"))
}

/// Reads a `KEY=VALUE` file: one setting per line, `#` comments and blank lines skipped, an
/// optional `export ` before the key, and matching quotes around the value removed.
pub fn parse_file(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(value);
            Some((key.trim().to_owned(), value.to_owned()))
        })
        .collect()
}

impl Config {
    /// Resolves the configuration: `env` overrides `file` for each name, and an empty value
    /// counts as absent. Every problem is reported, not just the first.
    pub fn resolve(
        file: &BTreeMap<String, String>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Config, Refused> {
        let value = |name: &str| {
            env(name)
                .or_else(|| file.get(name).cloned())
                .filter(|value| !value.is_empty())
        };
        let mut problems: Vec<String> = Vec::new();
        for (name, replacement) in RETIRED {
            if value(name).is_some() {
                problems.push(format!("{name} is retired; use {replacement}"));
            }
        }
        for name in [BOT_TOKEN, APP_TOKEN, STATE_DIR] {
            if value(name).is_none() {
                problems.push(format!("{name} is missing"));
            }
        }
        let names: Vec<String> = value(CHANNELS)
            .unwrap_or_default()
            .split(',')
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .collect();
        if names.is_empty() {
            problems.push(format!("{CHANNELS} is missing: declare at least one channel"));
        }
        let mut channels: Vec<ChannelConfig> = Vec::new();
        for name in names {
            if !valid_name(&name) {
                problems.push(format!(
                    "channel name {name:?} is invalid: use 1 to 40 lowercase letters, digits and -"
                ));
                continue;
            }
            if channels.iter().any(|channel| channel.name == name) {
                problems.push(format!("channel {name} is declared twice in {CHANNELS}"));
                continue;
            }
            let prefix = key_prefix(&name);
            let id = value(&format!("{prefix}_ID")).unwrap_or_default();
            if id.is_empty() {
                problems.push(format!("channel {name} has no id: set {prefix}_ID"));
            }
            let speakers: Vec<String> = value(&format!("{prefix}_SPEAKERS"))
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|user| !user.is_empty())
                .map(str::to_owned)
                .collect();
            if speakers.is_empty() {
                problems.push(format!("channel {name} has no speakers: set {prefix}_SPEAKERS"));
            }
            channels.push(ChannelConfig { name, id, speakers });
        }
        if !problems.is_empty() {
            return Err(Refused(problems));
        }
        Ok(Config {
            bot_token: value(BOT_TOKEN).unwrap_or_default(),
            app_token: value(APP_TOKEN).unwrap_or_default(),
            channels,
            state_dir: PathBuf::from(value(STATE_DIR).unwrap_or_default()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "# settings\nROC_STATE_DIR=/state\nexport SLACK_BOT_TOKEN=\"xoxb-file\"\n\
        SLACK_APP_TOKEN='xapp-file'\nSLACK_CHANNELS=gonk-chat, fixme\n\
        SLACK_CHANNEL_GONK_CHAT_ID=C1\nSLACK_CHANNEL_GONK_CHAT_SPEAKERS=U1, U3\n\
        SLACK_CHANNEL_FIXME_ID=C2\nSLACK_CHANNEL_FIXME_SPEAKERS=U2\n";

    fn file_with(text: &str) -> BTreeMap<String, String> {
        parse_file(&format!("{GOOD}{text}"))
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn refused(file: &BTreeMap<String, String>) -> Vec<String> {
        Config::resolve(file, no_env).unwrap_err().0
    }

    fn only(problems: Vec<String>) -> String {
        assert_eq!(problems.len(), 1, "{problems:?}");
        problems.into_iter().next().unwrap()
    }

    #[test]
    fn a_good_two_channel_configuration_resolves() {
        let config = Config::resolve(&file_with(""), no_env).unwrap();
        let expected = vec![
            ChannelConfig { name: "gonk-chat".into(), id: "C1".into(), speakers: vec!["U1".into(), "U3".into()] },
            ChannelConfig { name: "fixme".into(), id: "C2".into(), speakers: vec!["U2".into()] },
        ];
        assert_eq!(config.channels, expected);
        assert_eq!(config.bot_token, "xoxb-file");
        assert_eq!(config.app_token, "xapp-file");
        assert_eq!(config.state_dir, PathBuf::from("/state"));
    }

    #[test]
    fn the_environment_overrides_the_file() {
        let env = |name: &str| (name == "SLACK_CHANNEL_FIXME_ID").then(|| "C9".to_owned());
        let config = Config::resolve(&file_with(""), env).unwrap();
        assert_eq!(config.channels[1].id, "C9");
    }

    #[test]
    fn a_missing_app_token_stops_the_start_and_is_named() {
        let mut file = file_with("");
        file.remove(APP_TOKEN);
        assert_eq!(only(refused(&file)), "SLACK_APP_TOKEN is missing");
    }

    #[test]
    fn no_channels_stops_the_start() {
        let mut file = file_with("");
        file.remove(CHANNELS);
        assert!(only(refused(&file)).starts_with("SLACK_CHANNELS is missing"));
    }

    #[test]
    fn a_list_of_only_separators_stops_the_start() {
        let mut file = file_with("");
        file.insert(CHANNELS.to_owned(), " , ".to_owned());
        assert!(only(refused(&file)).starts_with("SLACK_CHANNELS is missing"));
    }

    #[test]
    fn a_bad_channel_name_stops_the_start() {
        for bad in ["Fixme", "fix_me", "fix.me", &"a".repeat(41)] {
            let mut file = file_with("");
            file.insert(CHANNELS.to_owned(), format!("gonk-chat,{bad}"));
            let problem = only(refused(&file));
            assert!(problem.contains("invalid") && problem.contains(bad), "{problem}");
        }
    }

    #[test]
    fn a_repeated_channel_name_stops_the_start() {
        let mut file = file_with("");
        file.insert(CHANNELS.to_owned(), "fixme,gonk-chat,fixme".to_owned());
        assert_eq!(only(refused(&file)), "channel fixme is declared twice in SLACK_CHANNELS");
    }

    #[test]
    fn a_channel_with_no_id_stops_the_start_and_is_named() {
        let mut file = file_with("");
        file.remove("SLACK_CHANNEL_FIXME_ID");
        assert_eq!(only(refused(&file)), "channel fixme has no id: set SLACK_CHANNEL_FIXME_ID");
    }

    #[test]
    fn a_channel_id_of_only_whitespace_stops_the_start_and_is_named() {
        let expected = "channel fixme has no id: set SLACK_CHANNEL_FIXME_ID";
        let file = parse_file(&format!("{GOOD}SLACK_CHANNEL_FIXME_ID=\"   \"\n"));
        assert_eq!(only(refused(&file)), expected);
        let env = |name: &str| (name == "SLACK_CHANNEL_FIXME_ID").then(|| "   ".to_owned());
        assert_eq!(only(Config::resolve(&file_with(""), env).unwrap_err().0), expected);
    }

    #[test]
    fn a_channel_with_no_speakers_stops_the_start_and_is_named() {
        let mut file = file_with("");
        file.insert("SLACK_CHANNEL_FIXME_SPEAKERS".to_owned(), " , ".to_owned());
        let problem = only(refused(&file));
        assert_eq!(problem, "channel fixme has no speakers: set SLACK_CHANNEL_FIXME_SPEAKERS");
    }

    #[test]
    fn each_retired_setting_is_refused_by_name() {
        for name in ["SLACK_CHANNEL", "SLACK_ALLOWED_USERS", "ROC_INBOX_SPACE"] {
            let problem = only(refused(&file_with(&format!("{name}=x\n"))));
            assert!(problem.starts_with(&format!("{name} is retired")), "{problem}");
        }
    }

    #[test]
    fn every_problem_is_named_not_just_the_first() {
        let problems = refused(&BTreeMap::new());
        assert_eq!(problems.len(), 4, "{problems:?}");
    }

    #[test]
    fn debug_never_prints_a_token() {
        let config = Config::resolve(&file_with(""), no_env).unwrap();
        let shown = format!("{config:?}");
        assert!(!shown.contains("xoxb-file"));
        assert!(!shown.contains("xapp-file"));
        assert!(shown.contains("redacted"));
    }
}
