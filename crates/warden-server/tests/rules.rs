//! Warden's own rules as tests (`warden-plan.md`, Review, layer 1), using the `warden-rules` harness, applied to the
//! real server started in this process with fixture account folders and a stand-in usage endpoint. Never delete or
//! weaken one of these tests to make the suite pass.

mod common;

use common::{Fixture, call, codex_token_count, cookie, eventually, raw};
use std::net::SocketAddr;
use warden_core::policy::{ALLOWED_OUTBOUND_HOSTS, BIND_IP};
use warden_rules::hook_data::{assert_not_kept, canary_text};
use warden_rules::secrets::{assert_no_token, canary_refresh_token, canary_token};
use warden_rules::{hosts, net};

#[test]
fn the_codex_apps_folder_is_left_alone() {
    use std::sync::Arc;
    use warden_core::model::Provider;
    use warden_server::app::App;
    let f = Fixture::new();
    let codex = f.codex_folder(
        ".codex",
        "private@example.com",
        &canary_token("private-codex"),
    );
    common::write(
        &codex.join("sessions/nested/session.jsonl"),
        "private fixture session\n",
    );
    common::write(&codex.join("hooks.json"), r#"{"hooks":{"Stop":[]}}"#);
    common::write(&codex.join("config.toml"), "model = 'private'\n");
    let before = common::folder_bytes(&codex);
    let work = f.claude_folder(".claude-work", "work@example.com", "work-fixture");
    // Even a hand-edited v2 file cannot smuggle a protected folder into readers or watchers.
    let mut settings = warden_core::store::Settings {
        setup_done: true,
        ..Default::default()
    };
    settings.accounts.push(warden_core::store::AccountConfig {
        id: "private".into(),
        label: "Private".into(),
        provider: Provider::Codex,
        folder: codex.clone(),
        enabled: true,
        identity: None,
    });
    warden_core::store::save(f.data.path(), warden_core::store::SETTINGS_FILE, &settings)
        .expect("settings");
    let mut env = f.env();
    env.codex_home = Some(codex.clone());
    let found = warden_core::detect::account_folders(&env);
    assert!(found.iter().all(|(_, folder)| !env.in_codex_app(folder)));
    let app = App::new(
        f.data.path().to_owned(),
        env,
        (
            f.endpoint.clone(),
            f.sign_in_server.clone(),
            f.resets.clone(),
        ),
        Arc::new(|_, _, _, _| panic!("no sign-in")),
        Arc::new(|_| {}),
        Some(f.route_exe.clone()),
        None,
    )
    .expect("app");
    for path in [
        codex.clone(),
        codex.join("sessions"),
        f.home.path().join(".codex-work/../.codex"),
    ] {
        for provider in [Provider::Claude, Provider::Codex] {
            assert_eq!(
                app.add_existing(provider, &path.to_string_lossy(), "Private")
                    .expect_err("refused")
                    .0,
                400
            );
        }
    }
    app.finish_setup(
        &[(
            Provider::Claude,
            work.to_string_lossy().into_owned(),
            "Work".into(),
        )],
        false,
    )
    .expect("setup");
    app.read_claude_usage();
    app.repair_account_hooks("a1").expect("status hook");
    app.repair_account_hooks("main").expect("repair");
    // `~/.warden` as a junction to the Codex app's folder (or main's): never scanned, and no new account is made in it.
    common::write(&f.home.path().join("bin/codex.cmd"), "@echo off");
    let warden = f.home.path().join(".warden");
    let decoy = f.claude_folder(".claude/claude-decoy", "decoy@example.com", "decoy");
    for target in [codex.clone(), f.home.path().join(".claude")] {
        junction::create(&target, &warden).expect("junction");
        let found = warden_core::detect::account_folders(&f.env());
        assert!(
            found
                .iter()
                .all(|(_, folder)| !folder.starts_with(&warden) && folder != &decoy),
            "{target:?}: {found:?}"
        );
        let refused = app.add_new(Provider::Codex, "Spare").expect_err("refused");
        assert!(matches!(refused.0, 400 | 409), "{target:?}: {}", refused.1);
        junction::delete(&warden).expect("unjunction");
        std::fs::remove_dir(&warden).expect("remove junction");
    }
    assert!(!f.home.path().join(".claude/codex-spare").exists());
    assert_eq!(common::folder_bytes(&codex), before);
}

#[test]
fn only_the_usage_endpoint_the_sign_in_server_codex_resets_and_github_releases_are_allowed() {
    hosts::assert_only_known_hosts(ALLOWED_OUTBOUND_HOSTS);
    hosts::assert_request_allowed(
        "https://github.com/Eterion/Warden/releases/latest/download/latest.json",
        ALLOWED_OUTBOUND_HOSTS,
    );
    hosts::assert_request_allowed(
        "https://platform.claude.com/v1/oauth/token",
        ALLOWED_OUTBOUND_HOSTS,
    );
    hosts::assert_request_allowed(warden_core::claude::USAGE_URL, ALLOWED_OUTBOUND_HOSTS);
    hosts::assert_request_allowed(
        warden_core::codex_resets::RESETS_URL,
        ALLOWED_OUTBOUND_HOSTS,
    );
}

/// The policy constant only; `server_listens_on_loopback_only` checks the address the server really listens on.
#[test]
fn server_binds_to_loopback_only() {
    net::assert_loopback_v4(SocketAddr::new(BIND_IP, 0));
}

#[test]
fn server_listens_on_loopback_only() {
    let f = Fixture::new();
    let server = f.start();
    net::assert_loopback_v4(server.addr);
    server.stop();
}

#[test]
fn server_checks_the_host_header() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    let cookie = cookie(&server);
    net::assert_host_header_checked(addr.port(), |host| {
        let mut headers: Vec<(&str, &str)> = host.map(|h| vec![("Host", h)]).unwrap_or_default();
        headers.push(("Cookie", &cookie));
        raw(addr, "GET", "/api/state", &headers, None).status == 200
    });
    server.stop();
}

#[test]
fn server_checks_the_origin() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    let host = format!("127.0.0.1:{}", addr.port());
    let cookie = cookie(&server);
    net::assert_origin_checked(addr.port(), &[], |origin| {
        let mut headers = vec![("Host", host.as_str()), ("Cookie", &cookie)];
        if let Some(o) = origin {
            headers.push(("Origin", o));
        }
        // A request that changes something, as another web page would send it.
        raw(
            addr,
            "PUT",
            "/api/settings",
            &headers,
            Some(r#"{"theme":"dark"}"#),
        )
        .status
            == 200
    });
    server.stop();
}

/// A web page's GET without an Origin (an image or a script pointed at the server) is refused by its
/// `Sec-Fetch-Site`, the long poll among them, so no other page can hold the request workers.
#[test]
fn server_checks_the_fetch_site() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    let host = format!("127.0.0.1:{}", addr.port());
    let cookie = cookie(&server);
    let get = |path: &str, site: Option<&str>| {
        let mut headers = vec![("Host", host.as_str()), ("Cookie", cookie.as_str())];
        if let Some(s) = site {
            headers.push(("Sec-Fetch-Site", s));
        }
        raw(addr, "GET", path, &headers, None).status == 200
    };
    net::assert_fetch_site_checked(|site| get("/api/state", site));
    // The long poll: only the refusals, which answer at once (an accepted one would wait out its 50 seconds).
    for (site, accepted) in net::fetch_site_cases() {
        if !accepted {
            assert!(
                !get("/api/state/wait?since=999999999", site),
                "rule broken: the long poll answered Sec-Fetch-Site {site:?}"
            );
        }
    }
    server.stop();
}

/// Only requests with this run's token in their cookie are answered: the API, the long poll and the page alike.
#[test]
fn server_checks_the_token() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    let host = format!("127.0.0.1:{}", addr.port());
    let send = |method: &str, path: &str, cookie: Option<&str>, body: Option<&str>| {
        let mut headers = vec![("Host", host.as_str())];
        if let Some(c) = cookie {
            headers.push(("Cookie", c));
        }
        raw(addr, method, path, &headers, body).status
    };
    let (name, value) = (server.token.cookie_name(), server.token.value());
    net::assert_token_checked(name, value, |c| send("GET", "/api/state", c, None) == 200);
    net::assert_token_checked(name, value, |c| {
        send("PUT", "/api/settings", c, Some(r#"{"theme":"dark"}"#)) == 200
    });
    // The page and the long poll: only the refusals (the page is not built here; the long poll would wait).
    for (cookie, accepted) in net::token_cases(name, value) {
        if !accepted {
            for (method, path) in [("GET", "/"), ("GET", "/api/state/wait?since=999999999")] {
                assert_eq!(
                    send(method, path, cookie.as_deref(), None),
                    403,
                    "rule broken: {method} {path} answered Cookie {cookie:?}"
                );
            }
        }
    }
    // The token in the address works only for the page, and only as itself.
    for path in [
        format!("/api/state?token={value}"),
        "/?token=wrong".to_owned(),
        "/?token=".to_owned(),
    ] {
        assert_eq!(send("GET", &path, None, None), 403, "{path}");
    }
    server.stop();
}

/// The address the server prints trades the token for a cookie and takes it out of the address bar.
#[test]
fn the_printed_address_trades_the_token_for_a_cookie() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    assert_eq!(
        server.url(),
        format!("http://{addr}/?token={}", server.token.value())
    );
    let host = format!("127.0.0.1:{}", addr.port());
    let path = format!("/settings?x=1&token={}", server.token.value());
    let reply = raw(addr, "GET", &path, &[("Host", &host)], None);
    assert_eq!(reply.status, 303);
    assert_eq!(reply.header("Location"), Some("/settings?x=1"));
    assert_eq!(
        reply.header("Set-Cookie"),
        Some(format!("{}; Path=/; HttpOnly; SameSite=Strict", cookie(&server)).as_str())
    );
    // Opened again with the cookie already set: the token still leaves the address bar.
    let again = raw(
        addr,
        "GET",
        &path,
        &[("Host", &host), ("Cookie", &cookie(&server))],
        None,
    );
    assert_eq!(again.status, 303);
    assert_eq!(again.header("Location"), Some("/settings?x=1"));
    server.stop();
}

/// Every answer carries the strict Content-Security-Policy, and the server's log never shows the token.
#[test]
fn server_sends_a_strict_csp() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    let host = format!("127.0.0.1:{}", addr.port());
    let replies = [
        ("GET /api/state", call(addr, "GET", "/api/state", None)),
        ("GET /api/nothing", call(addr, "GET", "/api/nothing", None)),
        ("GET /", call(addr, "GET", "/", None)),
        // Refused: no token, another page, the hooks' old route.
        (
            "no token",
            raw(addr, "GET", "/api/state", &[("Host", &host)], None),
        ),
        (
            "another Origin",
            raw(
                addr,
                "GET",
                "/api/state",
                &[("Host", &host), ("Origin", "https://example.com")],
                None,
            ),
        ),
        (
            "POST /api/events",
            call(addr, "POST", "/api/events?account=nobody", Some("{}")),
        ),
    ];
    for (what, reply) in replies {
        let csp = reply.header("Content-Security-Policy").expect(what);
        warden_rules::csp::assert_strict(csp, &[]);
    }
    let token = server.token.value().to_owned();
    server.stop();
    assert!(!f.log_text().contains(&token), "the log shows the token");
}

#[test]
fn tokens_never_reach_logs_or_responses() {
    let f = Fixture::new();
    let old_access = canary_token("own-old");
    let old_refresh = canary_refresh_token("own-old");
    let new_access = canary_token("own-new");
    let new_refresh = canary_refresh_token("own-new");
    f.claude_folder(".claude", "main@example.com", "main-app-never-sent");
    let own = f.main_sign_in("main@example.com", &old_access, &old_refresh, 0);
    f.sign_in_server
        .answers
        .lock()
        .expect("lock")
        .push_back(Ok(warden_core::mainlogin::Renewed {
            access_token: new_access.clone().into(),
            refresh_token: Some(new_refresh.clone().into()),
            expires_in: 28_800,
        }));
    let claude_token = canary_token("claude");
    let codex_token = canary_token("codex");
    let claude = f.claude_folder(".claude-work", "ann@acme.example", &claude_token);
    let codex = f.codex_folder(".codex-work", "ann@acme.example", &codex_token);
    common::write(
        &codex.join("sessions/2026/11/07/rollout-1.jsonl"),
        &format!(
            "{}\n",
            codex_token_count("2026-11-07T10:00:00Z", 12.0, 31.0)
        ),
    );
    *f.resets.answer.lock().expect("lock") = Some((200, common::RESETS_ANSWER.to_owned()));
    let server = f.start();
    let addr = server.addr;

    let mut answers = Vec::new();
    answers.push((
        "GET /api/setup/candidates",
        call(addr, "GET", "/api/setup/candidates", None),
    ));
    let add = |provider: &str, folder: &std::path::Path, label: &str| {
        let body = serde_json::json!({ "provider": provider, "folder": folder, "label": label })
            .to_string();
        call(addr, "POST", "/api/accounts", Some(&body))
    };
    answers.push((
        "POST /api/accounts (Claude)",
        add("claude", &claude, "Work"),
    ));
    answers.push(("POST /api/accounts (Codex)", add("codex", &codex, "Codex")));
    answers.push((
        "POST /api/usage/refresh",
        call(addr, "POST", "/api/usage/refresh", None),
    ));
    eventually("the Codex reading", || {
        call(addr, "GET", "/api/state", None).json()["accounts"][1]["usage"].is_object()
    });
    eventually("the Codex account's resets", || {
        call(addr, "GET", "/api/state", None).json()["accounts"][1]["resets"][0].is_object()
    });
    answers.push(("GET /api/state", call(addr, "GET", "/api/state", None)));
    server.stop();

    assert_eq!(
        *f.sign_in_server.tokens_seen.lock().expect("lock"),
        vec![old_refresh.clone()]
    );
    let own_saved = std::fs::read_to_string(own.join(".credentials.json")).expect("own login");
    assert!(own_saved.contains(&new_access) && own_saved.contains(&new_refresh));
    // The token was read, and sent only to the usage endpoint.
    assert!(
        f.endpoint
            .tokens_seen
            .lock()
            .expect("lock")
            .iter()
            .any(|t| t == &claude_token),
        "the usage endpoint was called with the account's token"
    );
    // The Codex access token was read, and sent only to ChatGPT's backend with its account id.
    let resets_seen = f.resets.seen.lock().expect("lock").clone();
    assert!(!resets_seen.is_empty());
    assert!(
        resets_seen
            .iter()
            .all(|(token, account)| token == &codex_token && account == "ws-1"),
        "only the Codex account's own access token goes to the resets endpoint"
    );
    assert!(
        !f.endpoint
            .tokens_seen
            .lock()
            .expect("lock")
            .contains(&codex_token)
    );
    let state = answers.last().expect("state").1.json();
    assert_eq!(
        state["accounts"][1]["resets"][0]["title"],
        "Full reset (Weekly + 5 hr)"
    );
    assert_eq!(
        state["accounts"][0]["usage"]["fiveHour"]["usedPercent"],
        61.0
    );
    assert_eq!(
        state["accounts"][1]["identity"]["email"],
        "ann@acme.example"
    );

    for token in [
        &claude_token,
        &codex_token,
        &old_access,
        &old_refresh,
        &new_access,
        &new_refresh,
    ] {
        // The renewal is logged (when it holds until), never the login.
        assert!(
            f.log_text()
                .contains("renewed Warden's sign-in to main; it holds until "),
            "{}",
            f.log_text()
        );
        assert_no_token("server log", &f.log_text(), token);
        assert_no_token("log file", &log_files(&f), token);
        for (place, reply) in &answers {
            assert_eq!(reply.status, 200, "{place}: {}", reply.body);
            assert_no_token(place, &reply.body, token);
        }
        for (file, bytes) in common::folder_bytes(f.data.path()) {
            if file == std::path::Path::new("main-sign-in/.credentials.json") {
                continue;
            }
            assert_no_token(
                &file.to_string_lossy(),
                &String::from_utf8_lossy(&bytes),
                token,
            );
        }
        for file in ["settings.json", "readings.json"] {
            let saved = std::fs::read_to_string(f.data.path().join(file)).expect("saved file");
            assert_no_token(file, &saved, token);
        }
    }
}

#[test]
fn account_logins_are_never_written() {
    use std::sync::Arc;
    use warden_core::model::Provider;
    use warden_server::app::App;
    let f = Fixture::new();
    let app_token = canary_token("app");
    let expired_token = canary_token("expired");
    let own_refresh = canary_refresh_token("own");
    let main = f.claude_folder(".claude", "main@example.com", &app_token);
    let fresh = f.claude_folder(".claude-fresh", "fresh@example.com", &canary_token("fresh"));
    let expired = f.claude_folder(".claude-expired", "expired@example.com", &expired_token);
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(expired.join(".credentials.json")).expect("login"))
            .expect("json");
    value["claudeAiOauth"]["expiresAt"] = 0.into();
    common::write(&expired.join(".credentials.json"), &value.to_string());
    let before: Vec<_> = [&main, &fresh, &expired]
        .iter()
        .map(|folder| std::fs::read(folder.join(".credentials.json")).expect("before"))
        .collect();
    f.main_sign_in("main@example.com", &canary_token("own"), &own_refresh, 0);
    f.sign_in_server
        .answers
        .lock()
        .expect("lock")
        .push_back(Ok(warden_core::mainlogin::Renewed {
            access_token: canary_token("own-new").into(),
            refresh_token: Some(canary_refresh_token("own-new").into()),
            expires_in: 28_800,
        }));
    let app = App::new(
        f.data.path().to_owned(),
        f.env(),
        (
            f.endpoint.clone(),
            f.sign_in_server.clone(),
            f.resets.clone(),
        ),
        Arc::new(|_, _, _, _| panic!("no sign-in")),
        Arc::new(|_| {}),
        Some(f.route_exe.clone()),
        None,
    )
    .expect("app");
    for (folder, label) in [(&fresh, "Fresh"), (&expired, "Expired")] {
        app.add_existing(Provider::Claude, &folder.to_string_lossy(), label)
            .expect("account");
    }
    app.read_claude_usage();
    app.read_claude_usage();
    for (folder, bytes) in [&main, &fresh, &expired].iter().zip(before) {
        assert_eq!(
            std::fs::read(folder.join(".credentials.json")).expect("after"),
            bytes
        );
    }
    assert_eq!(
        *f.sign_in_server.tokens_seen.lock().expect("lock"),
        vec![own_refresh]
    );
    assert!(
        !f.endpoint
            .tokens_seen
            .lock()
            .expect("lock")
            .contains(&app_token)
    );
    assert!(
        !f.endpoint
            .tokens_seen
            .lock()
            .expect("lock")
            .contains(&expired_token)
    );
}

/// Everything in the data folder's rolling log files; there is at least one, holding the server's lines.
fn log_files(f: &Fixture) -> String {
    let dir = f.data.path().join("logs");
    let mut text = String::new();
    for e in std::fs::read_dir(&dir).expect("the log folder") {
        text.push_str(&std::fs::read_to_string(e.expect("entry").path()).expect("a log file"));
    }
    assert!(
        text.contains("Warden "),
        "the log file holds the server's lines"
    );
    text
}

#[test]
fn prompts_and_tool_inputs_never_kept() {
    let f = Fixture::new();
    let claude = f.claude_folder(".claude-work", "ann@acme.example", "t1");
    let server = f.start();
    let addr = server.addr;
    let body =
        serde_json::json!({ "provider": "claude", "folder": claude, "label": "Work" }).to_string();
    assert_eq!(call(addr, "POST", "/api/accounts", Some(&body)).status, 200);
    let id = call(addr, "GET", "/api/state", None).json()["accounts"][0]["id"]
        .as_str()
        .expect("id")
        .to_owned();

    let prompt = canary_text("prompt");
    let input = canary_text("tool input");
    let events = [
        serde_json::json!({ "session_id": "s1", "hook_event_name": "SessionStart", "cwd": f.home.path() }),
        serde_json::json!({ "session_id": "s1", "hook_event_name": "UserPromptSubmit", "prompt": prompt }),
        serde_json::json!({ "session_id": "s1", "hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": { "command": input } }),
        serde_json::json!({ "session_id": "s1", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": { "command": input } }),
        serde_json::json!({ "session_id": "s1", "hook_event_name": "Notification", "notification_type": "permission_prompt", "message": prompt }),
    ];
    let mut answers = Vec::new();
    for e in events {
        // What the hook sends: `warden event` keeps only the contract's fields.
        let mut kept = warden_core::pipe::HookEvent::from_hook_input(e.to_string().as_bytes())
            .expect("an event");
        kept.parent_session_id = Some("main-session".into());
        let message = warden_core::pipe::Message {
            account: id.clone(),
            event: kept,
        };
        let sent = String::from_utf8(message.to_line()).expect("utf-8");
        assert_eq!(
            warden_core::pipe::Message::read(sent.as_bytes())
                .expect("allowed fields")
                .event
                .parent_session_id
                .as_deref(),
            Some("main-session")
        );
        answers.push(sent);
        assert!(warden_core::pipe::send(f.data.path(), &message).is_ok());
    }
    // And the server refuses a message that carries more, even from the user's own process.
    let leaky = format!(
        r#"{{"account":"{id}","event":{{"session_id":"s2","hook_event_name":"UserPromptSubmit","parent_session_id":"main-session","prompt":"{prompt}"}}}}"#
    );
    assert!(warden_core::pipe::Message::read(leaky.as_bytes()).is_err());
    let state = call(addr, "GET", "/api/state", None);
    assert_eq!(
        state.json()["sessions"][0]["doing"],
        "Asks to run a command"
    );
    answers.push(state.body);
    server.stop();

    for canary in [&prompt, &input] {
        assert_not_kept("server log", &f.log_text(), canary);
        assert_not_kept("log file", &log_files(&f), canary);
        for answer in &answers {
            assert_not_kept("what the hook sent, or an API answer", answer, canary);
        }
        for file in ["settings.json", "readings.json"] {
            let saved = std::fs::read_to_string(f.data.path().join(file)).expect("saved file");
            assert_not_kept(file, &saved, canary);
        }
    }
}

#[test]
fn transcripts_read_only_for_their_title() {
    let f = Fixture::new();
    let claude = f.claude_folder(".claude-work", "ann@acme.example", "t1");
    let server = f.start();
    let addr = server.addr;
    let body =
        serde_json::json!({ "provider": "claude", "folder": claude, "label": "Work" }).to_string();
    assert_eq!(call(addr, "POST", "/api/accounts", Some(&body)).status, 200);
    let id = call(addr, "GET", "/api/state", None).json()["accounts"][0]["id"]
        .as_str()
        .expect("id")
        .to_owned();

    // A transcript holds the conversation; only its title record may be read from it.
    let message = canary_text("transcript message");
    let projects = claude.join("projects").join("p");
    std::fs::create_dir_all(&projects).expect("mkdir");
    let transcript = projects.join("s1.jsonl");
    let lines = [
        serde_json::json!({ "type": "user", "message": { "role": "user", "content": message } }),
        serde_json::json!({ "type": "custom-title", "customTitle": "Payroll export", "sessionId": "s1" }),
        serde_json::json!({ "type": "assistant", "message": { "content": message } }),
        // Claude Code's generated title never replaces the session's own.
        serde_json::json!({ "type": "ai-title", "aiTitle": "Generated name", "sessionId": "s1" }),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&transcript, text).expect("transcript");
    // Without a custom title, the generated one names the session; nothing else in its record or the transcript is
    // kept.
    let generated = projects.join("s3.jsonl");
    let lines = [
        serde_json::json!({ "type": "user", "message": { "role": "user", "content": message } }),
        serde_json::json!({ "type": "ai-title", "aiTitle": "Fix the login", "sessionId": "s3", "note": message }),
        serde_json::json!({ "type": "assistant", "message": { "content": message } }),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&generated, text).expect("transcript");
    // A transcript path outside the account's folder is not read at all.
    let outside = f.home.path().join("s2.jsonl");
    std::fs::write(
        &outside,
        "{\"type\":\"custom-title\",\"customTitle\":\"Outside\"}\n",
    )
    .expect("outside");

    let events = [
        serde_json::json!({ "session_id": "s1", "hook_event_name": "SessionStart", "cwd": f.home.path(), "transcript_path": transcript }),
        serde_json::json!({ "session_id": "s1", "hook_event_name": "Stop", "transcript_path": transcript }),
        serde_json::json!({ "session_id": "s2", "hook_event_name": "SessionStart", "cwd": f.home.path(), "transcript_path": outside }),
        serde_json::json!({ "session_id": "s3", "hook_event_name": "SessionStart", "cwd": f.home.path(), "transcript_path": generated }),
    ];
    let mut answers = Vec::new();
    for e in events {
        // As the status hook sends it, over the pipe.
        assert!(common::hook_event(f.data.path(), &id, &e));
    }
    let state = call(addr, "GET", "/api/state", None);
    let name = |sid: &str| {
        state.json()["sessions"]
            .as_array()
            .expect("sessions")
            .iter()
            .find(|s| s["id"] == sid)
            .map(|s| s["name"].clone())
            .expect("session")
    };
    assert_eq!(name("s1"), "Payroll export");
    assert_eq!(name("s3"), "Fix the login");
    assert_ne!(name("s2"), "Outside");
    answers.push(state.body.clone());
    server.stop();

    assert_not_kept("server log", &f.log_text(), &message);
    for answer in &answers {
        assert_not_kept("an API answer", answer, &message);
    }
    for file in ["settings.json", "readings.json"] {
        let saved = std::fs::read_to_string(f.data.path().join(file)).expect("saved file");
        assert_not_kept(file, &saved, &message);
    }
}
