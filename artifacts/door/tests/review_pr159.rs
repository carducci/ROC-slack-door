//! Review 1 of PR 159 (ledger item 449): the door on ikigai-core 0.4 and ikigai-ipc 0.6.
//!
//! ikigai-ipc 0.6 refuses to bind a socket in a directory group or others can write (its ledger
//! item 3634). The door makes its own state directory with `create_dir_all`, whose mode is the
//! umask's: under umask 002, the default for a user with a private group, that is `0775`. On
//! 0.1.64 the door served `door.sock` there; on 0.6 the bind is refused, `main` logs it from the
//! serving thread and runs on, having already printed `serving …/door.sock`. The process stays up
//! with no socket, so `Restart=on-failure` never fires, and every mount of the door fails.
//!
//! The door is started for real, with tokens Slack refuses: its Socket Mode thread logs
//! `invalid_auth` (or cannot connect at all, offline) and the door runs on, as it does in service.

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn private_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("roc-door-review-pr159-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

/// The door started on `state`: it serves `door.sock` there, or it stops. What it must not do is
/// run on with nothing to mount.
fn serves_or_stops(state: &std::path::Path, umask: &str) -> Result<&'static str, String> {
    let mut door = Command::new("/bin/sh")
        .args(["-c", &format!("umask {umask} && exec \"$0\""), env!("CARGO_BIN_EXE_roc-slack-door")])
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
            break Ok("served");
        }
        if let Some(status) = door.try_wait().unwrap() {
            break if status.success() { Err("the door exited 0 without serving".to_owned()) } else { Ok("stopped") };
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
fn a_door_started_under_umask_002_serves_the_socket_in_the_state_directory_it_made_or_stops() {
    // The door makes the state directory itself, as on a first start.
    let state = private_dir("umask-002").join("state");
    let outcome = serves_or_stops(&state, "002");
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[test]
fn a_door_started_under_umask_022_serves_its_socket() {
    // The control: the same door, the same first start, under umask 022.
    let state = private_dir("umask-022").join("state");
    assert_eq!(serves_or_stops(&state, "022"), Ok("served"));
}
