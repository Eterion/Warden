//! `warden event` run as the status hook runs it: the built binary, with its input from a pipe. Whatever the input
//! or the core does, it exits 0 within its one-second deadline and prints nothing.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use warden_core::testdir::Dir;

#[test]
fn the_parent_variable_is_forwarded_and_invalid_values_are_dropped() {
    use warden_core::pipe::{Listener, Message};
    for parent in ["parent-123_A", "bad id", "$(unsafe)", ""] {
        let data = Dir::new("event-parent");
        let listener = Listener::create(
            data.path(),
            &warden_server::procs::current_user_sid().expect("SID"),
        )
        .expect("pipe");
        let receiver = std::thread::spawn(move || {
            let mut connection = listener.accept().expect("connection");
            let message: Message = connection.read().expect("message");
            connection.answer();
            message
        });
        let mut child = Command::new(env!("CARGO_BIN_EXE_warden"))
            .args(["event", "--account", "a1", "--data-dir"])
            .arg(data.path())
            .env("WARDEN_PARENT_SESSION", parent)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("starts");
        child.stdin.take().expect("stdin").write_all(br#"{"session_id":"child","hook_event_name":"SessionStart","parent_session_id":"forged","prompt":"private"}"#).expect("input");
        let output = child.wait_with_output().expect("exits");
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        let message = receiver.join().expect("receiver");
        assert_eq!(
            message.event.parent_session_id.as_deref(),
            if parent == "parent-123_A" {
                Some(parent)
            } else {
                None
            }
        );
    }
}

/// Starts `warden event` with a stdin that this test keeps open; `input` is written but the pipe never closes.
fn run_with_open_stdin(data: &std::path::Path, input: &str) -> (bool, String, Duration) {
    let start = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_warden"))
        .args(["event", "--account", "a1", "--data-dir"])
        .arg(data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("warden starts");
    let mut stdin = child.stdin.take().expect("stdin");
    stdin.write_all(input.as_bytes()).expect("write");
    stdin.flush().expect("flush");
    // `stdin` stays open until the hook has exited.
    let deadline = start + Duration::from_secs(5);
    while child.try_wait().expect("wait").is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("warden event did not give up on an input that never closes");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let took = start.elapsed();
    drop(stdin);
    let out = child.wait_with_output().expect("output");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        took,
    )
}

#[test]
fn an_input_that_never_closes_holds_the_hook_up_no_longer_than_its_deadline() {
    let dir = Dir::new("event-open-stdin");
    // Half an event, and the writer never closes the input.
    let (ok, stdout, took) = run_with_open_stdin(dir.path(), r#"{"session_id":"s1","hook_event_"#);
    assert!(ok, "exits 0");
    assert_eq!(stdout, "");
    assert!(took < Duration::from_secs(3), "took {took:?}");
}

#[test]
fn with_no_core_running_the_hook_exits_at_once() {
    let dir = Dir::new("event-no-core");
    let start = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_warden"))
        .args(["event", "--account", "a1", "--data-dir"])
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            c.stdin
                .take()
                .expect("stdin")
                .write_all(br#"{"session_id":"s1","hook_event_name":"Stop","prompt":"x"}"#)?;
            c.wait_with_output()
        })
        .expect("runs");
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert!(
        start.elapsed() < Duration::from_millis(900),
        "{:?}",
        start.elapsed()
    );
}
