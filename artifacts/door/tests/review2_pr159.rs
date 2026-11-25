//! Review 2 of PR 159 (ledger item 449): the door on ikigai-core 0.4 and ikigai-ipc 0.6.
//!
//! The fix for review 1 makes the state directory `0700` only when the door creates it:
//! `DirBuilder::recursive(true).mode(0o700)` sets no mode on a directory that is already there.
//! A door upgraded in place finds the state directory its previous version made, under the
//! umask of that start: `0775` under umask 002. On 0.1.64 the door served `door.sock` there; on
//! 0.6 the bind is refused and the door stops, every restart, so an upgrade takes a working
//! door off the air. The directory is the door's own, so the door can make it private.

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn private_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("roc-door-review2-pr159-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

/// Starts the door on `state` under umask 022 and says whether it serves `door.sock`.
fn serves(state: &Path) -> Result<(), String> {
    let mut door = Command::new("/bin/sh")
        .args(["-c", "umask 022 && exec \"$0\"", env!("CARGO_BIN_EXE_roc-slack-door")])
        .env_clear()
        .env("ROC_STATE_DIR", state)
        .env("SLACK_BOT_TOKEN", "xoxb-review")
        .env("SLACK_APP_TOKEN", "xapp-review")
        .env("SLACK_CHANNELS", "gonk-chat")
        .env("SLACK_CHANNEL_GONK_CHAT_ID", "C1")
        .env("SLACK_CHANNEL_GONK_CHAT_SPEAKERS", "U1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let socket = state.join("door.sock");
    let start = Instant::now();
    let outcome = loop {
        if UnixStream::connect(&socket).is_ok() {
            break Ok(());
        }
        if let Some(status) = door.try_wait().unwrap() {
            break Err(format!("the door stopped ({status}) without serving"));
        }
        if start.elapsed() > Duration::from_secs(10) {
            break Err(format!("the door is running and {} is not served", socket.display()));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let _ = door.kill();
    let output = door.wait_with_output().unwrap();
    outcome.map_err(|why| format!("{why}; stderr: {}", String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn a_door_upgraded_onto_the_state_directory_its_previous_version_made_under_umask_002_serves() {
    // The state directory as the door on ikigai-ipc 0.1.64 left it after a start under umask 002.
    let state = private_dir("upgrade").join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o775)).unwrap();
    let outcome = serves(&state);
    assert!(outcome.is_ok(), "{outcome:?}");
}
