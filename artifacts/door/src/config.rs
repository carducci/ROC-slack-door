//! The host's configuration for the door. Values come from a `KEY=VALUE` file the host names,
//! and environment variables of the same names override it. A start with anything missing is
//! refused, naming what is missing. Nothing here is printed: the tokens are redacted in `Debug`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

pub const BOT_TOKEN: &str = "SLACK_BOT_TOKEN";
pub const APP_TOKEN: &str = "SLACK_APP_TOKEN";
pub const CHANNEL: &str = "SLACK_CHANNEL";
pub const ALLOWED_USERS: &str = "SLACK_ALLOWED_USERS";
pub const STATE_DIR: &str = "ROC_STATE_DIR";
pub const INBOX_SPACE: &str = "ROC_INBOX_SPACE";

const REQUIRED: [&str; 6] = [BOT_TOKEN, APP_TOKEN, CHANNEL, ALLOWED_USERS, STATE_DIR, INBOX_SPACE];

/// Everything the door needs from its host.
#[derive(Clone)]
pub struct Config {
    pub bot_token: String,
    pub app_token: String,
    pub channel: String,
    /// The Slack user ids allowed to speak to the door.
    pub allowed_users: Vec<String>,
    pub state_dir: PathBuf,
    pub inbox_space: String,
}

/// The configuration names that were absent or empty.
#[derive(Debug, PartialEq, Eq)]
pub struct Missing(pub Vec<&'static str>);

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("bot_token", &"<redacted>")
            .field("app_token", &"<redacted>")
            .field("channel", &self.channel)
            .field("allowed_users", &self.allowed_users)
            .field("state_dir", &self.state_dir)
            .field("inbox_space", &self.inbox_space)
            .finish()
    }
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
    /// counts as absent. Every missing name is reported, not just the first.
    pub fn resolve(
        file: &BTreeMap<String, String>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Config, Missing> {
        let value = |name: &'static str| {
            env(name)
                .or_else(|| file.get(name).cloned())
                .filter(|value| !value.is_empty())
        };
        let missing: Vec<&'static str> = REQUIRED
            .into_iter()
            .filter(|name| value(*name).is_none())
            .collect();
        if !missing.is_empty() {
            return Err(Missing(missing));
        }
        let allowed_users = value(ALLOWED_USERS)
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|user| !user.is_empty())
            .map(str::to_owned)
            .collect();
        Ok(Config {
            bot_token: value(BOT_TOKEN).unwrap_or_default(),
            app_token: value(APP_TOKEN).unwrap_or_default(),
            channel: value(CHANNEL).unwrap_or_default(),
            allowed_users,
            state_dir: PathBuf::from(value(STATE_DIR).unwrap_or_default()),
            inbox_space: value(INBOX_SPACE).unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_file() -> BTreeMap<String, String> {
        parse_file(
            "# settings\nSLACK_CHANNEL=C1\nSLACK_ALLOWED_USERS=U1, U2\nROC_STATE_DIR=/state\n\
             ROC_INBOX_SPACE=dax\nexport SLACK_BOT_TOKEN=\"xoxb-file\"\nSLACK_APP_TOKEN='xapp-file'\n",
        )
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn a_file_with_every_setting_resolves() {
        let config = Config::resolve(&full_file(), no_env).unwrap();
        assert_eq!(config.channel, "C1");
        assert_eq!(config.allowed_users, ["U1", "U2"]);
        assert_eq!(config.bot_token, "xoxb-file");
        assert_eq!(config.app_token, "xapp-file");
        assert_eq!(config.state_dir, PathBuf::from("/state"));
        assert_eq!(config.inbox_space, "dax");
    }

    #[test]
    fn the_environment_overrides_the_file() {
        let env = |name: &str| (name == CHANNEL).then(|| "C9".to_owned());
        let config = Config::resolve(&full_file(), env).unwrap();
        assert_eq!(config.channel, "C9");
    }

    #[test]
    fn a_missing_app_token_stops_the_start_and_is_named() {
        let mut file = full_file();
        file.remove(APP_TOKEN);
        assert_eq!(Config::resolve(&file, no_env).unwrap_err(), Missing(vec![APP_TOKEN]));
    }

    #[test]
    fn every_missing_setting_is_named_not_just_the_first() {
        let missing = Config::resolve(&BTreeMap::new(), no_env).unwrap_err();
        assert_eq!(missing.0, REQUIRED);
    }

    #[test]
    fn an_empty_value_counts_as_missing() {
        let env = |name: &str| (name == CHANNEL).then(String::new);
        assert_eq!(Config::resolve(&full_file(), env).unwrap_err(), Missing(vec![CHANNEL]));
    }

    #[test]
    fn debug_never_prints_a_token() {
        let config = Config::resolve(&full_file(), no_env).unwrap();
        let shown = format!("{config:?}");
        assert!(!shown.contains("xoxb-file"));
        assert!(!shown.contains("xapp-file"));
        assert!(shown.contains("redacted"));
    }
}
