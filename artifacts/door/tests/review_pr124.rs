//! Review 1 of PR 124 (ledger item 40, task 1.2 of `channel-files`): a channel's file settings.
//!
//! A refusal naming an undetectable file type prints the type as written, through `Display`, to
//! the door's standard error, one line per problem. A type holding a newline or an escape writes
//! a second line that reads as the door's own, or drives the terminal. The channel-name check
//! already quotes what it echoes (`{name:?}`); the file-type check does not.

#[path = "../src/config.rs"]
#[allow(dead_code)]
mod config;

use config::{parse_file, Config};

const GOOD: &str = "ROC_STATE_DIR=/state\nSLACK_BOT_TOKEN=xoxb-file\nSLACK_APP_TOKEN=xapp-file\n\
    SLACK_CHANNELS=gonk-chat, fixme\nSLACK_CHANNEL_GONK_CHAT_ID=C1\nSLACK_CHANNEL_GONK_CHAT_SPEAKERS=U1\n\
    SLACK_CHANNEL_FIXME_ID=C2\nSLACK_CHANNEL_FIXME_SPEAKERS=U2\nSLACK_CHANNEL_FIXME_FILE_MAX_BYTES=10\n";

#[test]
fn a_refusal_naming_a_file_type_holds_no_control_character() {
    let file = parse_file(GOOD);
    for hostile in [
        "image/png\nroc-slack-door: refusing to start: forged",
        "image/png\r",
        "image/png\u{1b}[2J",
    ] {
        let env = |key: &str| (key == "SLACK_CHANNEL_FIXME_FILE_TYPES").then(|| hostile.to_owned());
        if let Err(refused) = Config::resolve(&file, env) {
            for problem in &refused.0 {
                assert!(!problem.chars().any(char::is_control), "a refusal prints a control character: {problem:?}");
            }
        }
    }
}
