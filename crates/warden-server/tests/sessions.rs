//! Sessions from the status hook, driven end to end with made-up events and fixture account folders.

mod common;

use common::{Fixture, call, eventually, hook_event, set_claude_identity, write};
use serde_json::{Value, json};
use std::net::SocketAddr;

fn add(addr: SocketAddr, provider: &str, folder: &std::path::Path, label: &str) -> String {
    let body = json!({ "provider": provider, "folder": folder, "label": label }).to_string();
    let reply = call(addr, "POST", "/api/accounts", Some(&body));
    assert_eq!(reply.status, 200, "{}", reply.body);
    let state = reply.json();
    let accounts = state["accounts"].as_array().expect("accounts");
    accounts.last().expect("added")["id"]
        .as_str()
        .expect("id")
        .to_owned()
}

/// Sends the event over the pipe, as the status hook does; 204 once the server took it, as the HTTP route answered.
fn send(f: &Fixture, account: &str, event: Value) -> u16 {
    if hook_event(f.data.path(), account, &event) {
        204
    } else {
        0
    }
}

fn sessions(addr: SocketAddr) -> Vec<Value> {
    call(addr, "GET", "/api/state", None).json()["sessions"]
        .as_array()
        .expect("sessions")
        .clone()
}

#[test]
fn a_claude_session_shows_live_and_ends_done() {
    let f = Fixture::new();
    let folder = f.claude_folder(".claude-work", "ann@acme.example", "t1");
    let project = f.home.path().join("shop");
    write(
        &project.join(".git/HEAD"),
        "ref: refs/heads/feature/55402-perf-admin\n",
    );
    let server = f.start();
    let addr = server.addr;
    let id = add(addr, "claude", &folder, "Work");

    let ev = |name: &str, extra: Value| {
        let mut e = json!({ "session_id": "s1", "hook_event_name": name, "cwd": project });
        if let (Some(e), Some(x)) = (e.as_object_mut(), extra.as_object()) {
            e.extend(x.clone());
        }
        e
    };
    let revision = call(addr, "GET", "/api/state", None).json()["revision"].clone();
    assert_eq!(send(&f, &id, ev("SessionStart", json!({}))), 204);
    // A new session wakes a waiting window at once.
    let waited = call(
        addr,
        "GET",
        &format!("/api/state/wait?since={revision}"),
        None,
    )
    .json();
    assert_eq!(waited["sessions"][0]["id"], "s1");
    let s = &sessions(addr)[0];
    assert_eq!(s["state"], "idle");
    assert_eq!(s["accountId"], id);
    assert_eq!(s["provider"], "claude");
    assert_eq!(s["project"], "shop");
    assert_eq!(s["name"], "55402 perf admin");
    assert!(s.get("item").is_none());
    assert_eq!(s["source"], "hook");

    send(&f, &id, ev("UserPromptSubmit", json!({ "prompt": "go" })));
    send(&f, &id, ev("PreToolUse", json!({ "tool_name": "Edit" })));
    send(
        &f,
        &id,
        ev(
            "SubagentStart",
            json!({ "agent_id": "x1", "agent_type": "Explore" }),
        ),
    );
    let s = &sessions(addr)[0];
    assert_eq!(s["state"], "working");
    assert_eq!(s["doing"], "Editing files");
    assert_eq!(s["subagents"], json!([{ "type": "Explore" }]));
    assert!(s.get("tool").is_none(), "the tool is not shown on its own");

    let state = call(addr, "GET", "/api/state", None).json();
    let bins = &state["activity"][0]["bins"];
    assert_eq!(state["activity"][0]["accountId"], id);
    assert_eq!(bins[bins.as_array().expect("bins").len() - 1]["working"], 1);

    send(&f, &id, ev("Stop", json!({})));
    assert_eq!(sessions(addr)[0]["state"], "idle");
    send(&f, &id, ev("SessionEnd", json!({ "reason": "exit" })));
    let s = &sessions(addr)[0];
    assert_eq!(s["state"], "done");
    assert!(s["endedAt"].is_number());
    server.stop();
}

#[test]
fn a_session_signed_in_as_someone_else_shows_who_and_counts_for_no_account() {
    let f = Fixture::new();
    let work = f.claude_folder(".claude-work", "work@acme.example", "t1");
    let personal = f.claude_folder(".claude-personal", "me@home.example", "t2");
    let server = f.start();
    let addr = server.addr;
    let work_id = add(addr, "claude", &work, "Work");
    let personal_id = add(addr, "claude", &personal, "Personal");

    // The desktop app's folder is now signed in as the personal account.
    set_claude_identity(&work, "me@home.example");
    send(
        &f,
        &work_id,
        json!({ "session_id": "d1", "hook_event_name": "SessionStart" }),
    );
    send(
        &f,
        &work_id,
        json!({ "session_id": "d1", "hook_event_name": "UserPromptSubmit" }),
    );
    let state = call(addr, "GET", "/api/state", None).json();
    let s = &state["sessions"][0];
    assert_eq!(s["accountId"], work_id, "shown with the folder's account");
    assert_eq!(s["signedInAs"], "me@home.example");
    assert_eq!(s["state"], "working");
    assert_eq!(
        state["accounts"][0]["identityChange"]["knownAccountId"],
        personal_id
    );
    // Counted for neither the old account nor the one signed in.
    for a in state["activity"].as_array().expect("activity") {
        for b in a["bins"].as_array().expect("bins") {
            assert_eq!(b["working"], 0, "{a}");
        }
    }

    // Accepted as the account's own: the running session counts for it from now on, without starting again.
    let reply = call(
        addr,
        "POST",
        &format!("/api/accounts/{work_id}/identity"),
        Some(r#"{"action":"accept"}"#),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    let state = reply.json();
    assert_eq!(state["sessions"][0]["signedInAs"], Value::Null);
    let work = state["activity"]
        .as_array()
        .expect("activity")
        .iter()
        .find(|a| a["accountId"] == work_id.as_str())
        .expect("work's activity");
    assert_eq!(
        work["bins"]
            .as_array()
            .expect("bins")
            .last()
            .expect("a bin")["last"],
        1
    );
    server.stop();
}

#[test]
fn an_identity_change_also_keeps_already_running_main_and_warden_sessions_apart() {
    for main in [true, false] {
        let f = Fixture::new();
        let folder = f.claude_folder(
            if main { ".claude" } else { ".claude-work" },
            "work@example.com",
            "fixture-never-main-usage",
        );
        let server = f.start();
        let id = if main {
            "main".to_owned()
        } else {
            add(server.addr, "claude", &folder, "Work")
        };
        send(
            &f,
            &id,
            json!({"session_id":"running", "hook_event_name":"SessionStart"}),
        );
        send(
            &f,
            &id,
            json!({"session_id":"running", "hook_event_name":"UserPromptSubmit"}),
        );
        assert!(sessions(server.addr)[0]["signedInAs"].is_null());
        set_claude_identity(&folder, "other@example.com");
        call(server.addr, "POST", "/api/usage/refresh", None);
        let s = call(server.addr, "GET", "/api/state", None).json();
        assert_eq!(s["sessions"][0]["signedInAs"], "other@example.com");
        let activity = s["activity"]
            .as_array()
            .expect("activity")
            .iter()
            .find(|a| a["accountId"] == id)
            .expect("account activity");
        assert_eq!(
            activity["bins"]
                .as_array()
                .expect("bins")
                .last()
                .expect("bin")["last"],
            0
        );
        set_claude_identity(&folder, "work@example.com");
        call(server.addr, "POST", "/api/usage/refresh", None);
        assert!(sessions(server.addr)[0]["signedInAs"].is_null());
        server.stop();
    }
}

#[test]
fn a_hook_connection_that_never_sends_holds_up_neither_other_events_nor_stopping() {
    let f = Fixture::new();
    let folder = f.claude_folder(".claude-work", "ann@acme.example", "t1");
    let server = f.start();
    let addr = server.addr;
    let id = add(addr, "claude", &folder, "Work");
    // A few connections to the pipe that never send a line (opened as a file, as any of the user's programs could).
    let pipe = warden_core::pipe::ConnectionInfo::load(f.data.path()).expect("the pipe");
    let mut stuck = Vec::new();
    for _ in 0..3 {
        stuck.push(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&pipe.pipe)
                .expect("connected"),
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let event = json!({ "session_id": "s1", "hook_event_name": "SessionStart" });
    assert_eq!(send(&f, &id, event), 204);
    assert_eq!(sessions(addr).len(), 1);
    let started = std::time::Instant::now();
    server.stop();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(6),
        "stopping waited {:?}",
        started.elapsed()
    );
    drop(stuck);
}

#[test]
fn a_new_session_leaves_the_readings_file_alone_when_nothing_in_it_changed() {
    let f = Fixture::new();
    let folder = f.claude_folder(".claude-work", "ann@acme.example", "t1");
    let server = f.start();
    let addr = server.addr;
    let id = add(addr, "claude", &folder, "Work");
    f.endpoint.settled_calls(&server);
    let file = f.data.path().join("readings.json");
    let modified = || {
        std::fs::metadata(&file)
            .and_then(|m| m.modified())
            .expect("saved")
    };
    let before = modified();
    for i in 0..5 {
        let event = json!({ "session_id": format!("s{i}"), "hook_event_name": "SessionStart" });
        assert_eq!(send(&f, &id, event), 204);
    }
    assert_eq!(sessions(addr).len(), 5);
    assert_eq!(modified(), before, "the same identity is not saved again");

    // A change is still saved.
    set_claude_identity(&folder, "bob@acme.example");
    send(
        &f,
        &id,
        json!({ "session_id": "s9", "hook_event_name": "SessionStart" }),
    );
    assert_ne!(modified(), before);
    server.stop();
}

#[test]
fn events_for_unknown_accounts_or_in_other_shapes_are_dropped_quietly() {
    let f = Fixture::new();
    let folder = f.claude_folder(".claude-work", "ann@acme.example", "t1");
    let server = f.start();
    let addr = server.addr;
    let id = add(addr, "claude", &folder, "Work");
    // Answered (so the hook can exit) and dropped.
    assert_eq!(
        send(
            &f,
            "nobody",
            json!({ "session_id": "s", "hook_event_name": "Stop" })
        ),
        204
    );
    assert_eq!(send(&f, &id, json!({ "hook_event_name": "Stop" })), 204);
    assert_eq!(send(&f, &id, json!({ "session_id": "s" })), 204);
    assert!(sessions(addr).is_empty());
    // A message with any field beyond what the hook keeps is refused, unread.
    let pipe = warden_core::pipe::ConnectionInfo::load(f.data.path()).expect("the pipe is named");
    assert!(pipe.pipe.starts_with(r"\\.\pipe\warden-"), "{}", pipe.pipe);
    assert_eq!(pipe.pid, std::process::id());
    // The hooks have no HTTP route any more.
    assert_eq!(
        call(addr, "POST", "/api/events?account=a1", Some("{}")).status,
        404
    );
    server.stop();
    // Stopped: the connection file is gone and sending fails at once.
    assert!(warden_core::pipe::ConnectionInfo::load(f.data.path()).is_none());
    assert_eq!(
        send(
            &f,
            &id,
            json!({ "session_id": "s", "hook_event_name": "Stop" })
        ),
        0
    );
}

#[test]
fn the_clis_session_lists_fill_in_where_no_hook_reports() {
    let f = Fixture::new();
    let folder = f.claude_folder(".claude-work", "ann@acme.example", "t1");
    write(
        &folder.join("projects/C--shop/listed-1.jsonl"),
        &format!(
            "{}\n",
            json!({ "type": "user", "cwd": f.home.path().join("shop"), "message": "private" })
        ),
    );
    let server = f.start();
    let addr = server.addr;
    // The start-up listing (before any account) is done; adding an account reads usage but lists nothing.
    common::eventually("the start-up reading", || server.app.usage_idle());
    let id = add(addr, "claude", &folder, "Work");
    assert!(
        sessions(addr).is_empty(),
        "read only when asked or at start"
    );

    call(addr, "POST", "/api/usage/refresh", None);
    let s = &sessions(addr)[0];
    assert_eq!(s["id"], "listed-1");
    assert_eq!(s["source"], "list");
    assert_eq!(s["accountId"], id);
    assert_eq!(s["project"], "shop");

    // Once the hook reports the session, the hook's view replaces the listed one.
    send(
        &f,
        &id,
        json!({ "session_id": "listed-1", "hook_event_name": "UserPromptSubmit" }),
    );
    let all = sessions(addr);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0]["source"], "hook");
    assert_eq!(all[0]["state"], "working");
    server.stop();
}

#[test]
fn listed_sessions_show_who_the_folder_is_signed_in_as() {
    let f = Fixture::new();
    let work = f.claude_folder(".claude-work", "work@acme.example", "t1");
    let personal = f.claude_folder(".claude-personal", "me@home.example", "t2");
    let server = f.start();
    let addr = server.addr;
    // The start-up listing is done before the session below is written.
    common::eventually("the start-up reading", || server.app.usage_idle());
    let work_id = add(addr, "claude", &work, "Work");
    add(addr, "claude", &personal, "Personal");

    // The desktop app's folder is now signed in as the personal account, and a session ran there; the refresh
    // finds out who is signed in before it lists the session.
    set_claude_identity(&work, "me@home.example");
    write(
        &work.join("projects/C--shop/listed-1.jsonl"),
        &format!(
            "{}\n",
            json!({ "type": "user", "cwd": f.home.path().join("shop") })
        ),
    );
    call(addr, "POST", "/api/usage/refresh", None);
    let s = &sessions(addr)[0];
    assert_eq!(s["id"], "listed-1");
    assert_eq!(s["accountId"], work_id);
    assert_eq!(s["signedInAs"], "me@home.example");
    server.stop();
}

fn name_of(addr: SocketAddr, id: &str) -> Option<Value> {
    sessions(addr)
        .into_iter()
        .find(|s| s["id"] == id)
        .map(|s| s["name"].clone())
}

#[test]
fn a_claude_session_takes_its_title_also_when_renamed_between_events() {
    let f = Fixture::new();
    let folder = f.claude_folder(".claude-work", "ann@acme.example", "t1");
    let project = f.home.path().join("shop");
    write(
        &project.join(".git/HEAD"),
        "ref: refs/heads/feature/55389-admin\n",
    );
    let transcript = folder.join("projects").join("shop").join("s1.jsonl");
    write(
        &transcript,
        "{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n",
    );
    let server = f.start();
    let addr = server.addr;
    let id = add(addr, "claude", &folder, "Work");
    let start = json!({ "session_id": "s1", "hook_event_name": "SessionStart", "cwd": project, "transcript_path": transcript });
    assert_eq!(send(&f, &id, start), 204);
    // No title yet: the branch in words.
    assert_eq!(name_of(addr, "s1"), Some(json!("55389 admin")));
    // Renamed with no hook event after it: the watcher reads what was added.
    std::fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .and_then(|mut file| {
            std::io::Write::write_all(
                &mut file,
                b"{\"type\":\"custom-title\",\"customTitle\":\"#55402: Payroll export\"}\n",
            )
        })
        .expect("append");
    eventually("the new title", || {
        name_of(addr, "s1") == Some(json!("#55402: Payroll export"))
    });
    server.stop();
}

#[test]
fn a_codex_session_takes_its_thread_name_from_the_index() {
    let f = Fixture::new();
    let folder = f.codex_folder(".codex-work", "ann@acme.example", "t1");
    write(
        &folder.join("session_index.jsonl"),
        "{\"id\":\"c1\",\"thread_name\":\"Fix login\",\"updated_at\":\"2026-11-10T08:00:00Z\"}\n",
    );
    let server = f.start();
    let addr = server.addr;
    let id = add(addr, "codex", &folder, "Codex");
    let start =
        json!({ "session_id": "c1", "hook_event_name": "SessionStart", "cwd": f.home.path() });
    assert_eq!(send(&f, &id, start), 204);
    eventually("the thread name", || {
        name_of(addr, "c1") == Some(json!("Fix login"))
    });
    // Renamed later: the index grows and the watcher reads the new line.
    std::fs::OpenOptions::new()
        .append(true)
        .open(folder.join("session_index.jsonl"))
        .and_then(|mut file| {
            std::io::Write::write_all(
                &mut file,
                b"{\"id\":\"c1\",\"thread_name\":\"Fix login 55410\"}\n",
            )
        })
        .expect("append");
    eventually("the new thread name", || {
        name_of(addr, "c1") == Some(json!("Fix login 55410"))
    });
    server.stop();
}
