//! Where each account's hooks stand is kept in memory and read again only when it can change: after Warden writes
//! the hooks, and when a hook file changes on disk. These tests change the files behind Warden's back, with fixture
//! account folders only, and check the state never shows what was there before.

mod common;

use common::{Fixture, call, eventually, write};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::Path;

fn add(addr: SocketAddr, provider: &str, folder: &Path, label: &str) {
    let body = json!({ "provider": provider, "folder": folder, "label": label }).to_string();
    let reply = call(addr, "POST", "/api/accounts", Some(&body));
    assert_eq!(reply.status, 200, "{}", reply.body);
}

fn state(addr: SocketAddr) -> Value {
    call(addr, "GET", "/api/state", None).json()
}

/// The status hook's setup in a state answer.
fn status_setup(s: &Value, id: &str) -> String {
    s["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|a| a["id"] == id)
        .expect("account")["hooks"]
        .as_str()
        .expect("setup")
        .to_owned()
}

fn becomes(addr: SocketAddr, id: &str, want: &str) {
    eventually(&format!("{id}'s status hook to show {want}"), || {
        status_setup(&state(addr), id) == want
    });
}

fn hooks_on(addr: SocketAddr, id: &str, on: bool) -> Value {
    let body = json!({ "enabled": on }).to_string();
    let reply = call(addr, "PATCH", &format!("/api/accounts/{id}"), Some(&body));
    assert_eq!(reply.status, 200, "{}", reply.body);
    reply.json()
}

const USER_SETTINGS: &str = r#"{ "permissions": { "allow": ["Bash(ls)"] } }"#;

#[test]
fn the_hook_state_follows_edits_files_appearing_and_going() {
    let f = Fixture::new();
    let main = f.claude_folder(".claude-work", "me@example.com", "t1");
    let codex = f.codex_folder(".codex-work", "me@example.com", "t2");
    write(&main.join("settings.json"), USER_SETTINGS);
    let server = f.start();
    let addr = server.addr;
    add(addr, "claude", &main, "Main");
    add(addr, "codex", &codex, "Codex");
    assert_eq!(status_setup(&state(addr), "a1"), "installed");

    // Warden's own install shows in its answer at once.
    assert_eq!(status_setup(&hooks_on(addr, "a1", true), "a1"), "installed");
    assert_eq!(status_setup(&hooks_on(addr, "a2", true), "a2"), "installed");
    let installed = std::fs::read_to_string(main.join("settings.json")).expect("read");

    // Edited by hand: the hook taken out, then put back.
    write(&main.join("settings.json"), USER_SETTINGS);
    becomes(addr, "a1", "notInstalled");
    write(&main.join("settings.json"), &installed);
    becomes(addr, "a1", "installed");

    // The file goes, and comes back (written beside it and renamed over, as editors do).
    std::fs::remove_file(main.join("settings.json")).expect("remove");
    becomes(addr, "a1", "notInstalled");
    write(&main.join("settings.json.tmp"), &installed);
    std::fs::rename(main.join("settings.json.tmp"), main.join("settings.json")).expect("rename");
    becomes(addr, "a1", "installed");

    // Codex: the trust taken out of config.toml, then put back; hooks.json gone and back.
    let config = std::fs::read_to_string(codex.join("config.toml")).expect("read");
    write(&codex.join("config.toml"), "model = \"example\"\n");
    becomes(addr, "a2", "notTrusted");
    write(&codex.join("config.toml"), &config);
    becomes(addr, "a2", "installed");
    let hooks = std::fs::read_to_string(codex.join("hooks.json")).expect("read");
    std::fs::remove_file(codex.join("hooks.json")).expect("remove");
    becomes(addr, "a2", "notInstalled");
    write(&codex.join("hooks.json"), &hooks);
    becomes(addr, "a2", "installed");

    // Other files in the folder change nothing.
    write(&main.join("history.jsonl"), "{}\n");
    assert_eq!(status_setup(&state(addr), "a1"), "installed");

    // Warden's own removal shows at once too.
    assert_eq!(
        status_setup(&hooks_on(addr, "a1", false), "a1"),
        "notInstalled"
    );
    server.stop();
}

#[test]
fn a_waiting_window_hears_of_a_hook_file_changed_by_hand() {
    let f = Fixture::new();
    let main = f.claude_folder(".claude-work", "me@example.com", "t1");
    let server = f.start();
    let addr = server.addr;
    add(addr, "claude", &main, "Main");
    hooks_on(addr, "a1", true);
    let s = state(addr);
    assert_eq!(status_setup(&s, "a1"), "installed");
    let revision = s["revision"].as_u64().expect("revision");
    // Other changes (the usage loop, the watcher seeing Warden's own write) may answer first; the window asks again
    // each time, as the UI does, until an answer carries the edit.
    let poll = std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut since = revision;
        loop {
            let answer = call(addr, "GET", &format!("/api/state/wait?since={since}"), None).json();
            let now = answer["revision"].as_u64().expect("revision");
            assert!(now > since, "answered on a change, not the timeout");
            if status_setup(&answer, "a1") == "notInstalled" {
                return started.elapsed();
            }
            since = now;
        }
    });
    std::thread::sleep(std::time::Duration::from_millis(200));
    write(&main.join("settings.json"), USER_SETTINGS);
    let waited = poll.join().expect("long poll");
    assert!(
        waited < std::time::Duration::from_secs(20),
        "heard of the edit at once, not after a long poll's timeout"
    );
    server.stop();
}

#[test]
fn status_events_leave_the_accounts_as_they_are() {
    let f = Fixture::new();
    let main = f.claude_folder(".claude-work", "me@example.com", "t1");
    let server = f.start();
    let addr = server.addr;
    add(addr, "claude", &main, "Main");
    hooks_on(addr, "a1", true);
    let before = state(addr);
    let event = |name: &str, tool: &str| {
        let body =
            json!({ "session_id": "s1", "hook_event_name": name, "tool_name": tool, "cwd": main });
        assert!(common::hook_event(f.data.path(), "a1", &body));
    };
    event("SessionStart", "");
    event("PreToolUse", "Bash");
    let after = state(addr);
    assert!(after["revision"].as_u64() > before["revision"].as_u64());
    assert_eq!(after["sessions"].as_array().expect("sessions").len(), 1);
    assert_eq!(
        after["accounts"][0]["hooks"],
        before["accounts"][0]["hooks"]
    );
    // A change to the accounts still shows after the events.
    write(&main.join("settings.json"), USER_SETTINGS);
    becomes(addr, "a1", "notInstalled");
    event("PreToolUse", "Edit");
    assert_eq!(status_setup(&state(addr), "a1"), "notInstalled");
    server.stop();
}

/// Retries a file operation Windows may refuse for a moment (a folder just deleted while watched is pending deletion).
fn retry(what: &str, mut f: impl FnMut() -> std::io::Result<()>) {
    eventually(what, || f().is_ok());
}

#[test]
fn an_account_folder_that_goes_and_comes_back_is_followed() {
    let f = Fixture::new();
    let main = f.claude_folder(".claude-work", "me@example.com", "t1");
    let server = f.start();
    let addr = server.addr;
    add(addr, "claude", &main, "Main");
    hooks_on(addr, "a1", true);
    let installed = std::fs::read_to_string(main.join("settings.json")).expect("read");
    let login = std::fs::read(main.join(".credentials.json")).expect("read");

    // The whole folder goes while Warden runs, then comes back with the hook.
    retry("the folder to go", || std::fs::remove_dir_all(&main));
    becomes(addr, "a1", "notInstalled");
    retry("the folder to come back", || std::fs::create_dir(&main));
    retry("the hook file to be written", || {
        std::fs::write(main.join("settings.json"), &installed)
    });
    becomes(addr, "a1", "installed");
    server.stop();

    // Missing when Warden starts: waited for, and followed once it is there.
    retry("the folder to go", || std::fs::remove_dir_all(&main));
    let server = f.start();
    let addr = server.addr;
    assert_eq!(status_setup(&state(addr), "a1"), "notInstalled");
    // The new server listens on another port, so its own hook is written again once the folder is back.
    retry("the folder to come back", || std::fs::create_dir(&main));
    retry("the login to be written", || {
        std::fs::write(main.join(".credentials.json"), &login)
    });
    let ours = hooks_on(addr, "a1", true);
    assert_eq!(status_setup(&ours, "a1"), "installed");
    let installed = std::fs::read_to_string(main.join("settings.json")).expect("read");
    write(&main.join("settings.json"), USER_SETTINGS);
    becomes(addr, "a1", "notInstalled");
    write(&main.join("settings.json"), &installed);
    becomes(addr, "a1", "installed");
    server.stop();
}
