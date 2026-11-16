//! The server's API driven end to end, with fixture account folders and a stand-in usage endpoint.

mod common;

use common::{
    Fixture, call, codex_token_count, eventually, set_claude_identity, set_codex_identity,
};
use serde_json::{Value, json};
use std::net::SocketAddr;

fn add(addr: SocketAddr, provider: &str, folder: &std::path::Path, label: &str) -> Value {
    let body = json!({ "provider": provider, "folder": folder, "label": label }).to_string();
    let reply = call(addr, "POST", "/api/accounts", Some(&body));
    assert_eq!(reply.status, 200, "{}", reply.body);
    reply.json()
}

fn state(addr: SocketAddr) -> Value {
    call(addr, "GET", "/api/state", None).json()
}

#[test]
fn reserve_is_checked_atomically_and_saved() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    assert_eq!(state(addr)["settings"]["reserve"], 25);
    for reserve in [1, 24, 51, 255] {
        let reply = call(
            addr,
            "PUT",
            "/api/settings",
            Some(&json!({"reserve":reserve,"theme":"dark"}).to_string()),
        );
        assert_eq!(reply.status, 400);
        assert_eq!(state(addr)["settings"]["reserve"], 25);
        assert_eq!(state(addr)["settings"]["theme"], "system");
    }
    for reserve in (0..=50).step_by(5) {
        assert_eq!(
            call(
                addr,
                "PUT",
                "/api/settings",
                Some(&json!({"reserve":reserve}).to_string())
            )
            .json()["settings"]["reserve"],
            reserve
        );
    }
    server.stop();
    let server = f.start();
    assert_eq!(state(server.addr)["settings"]["reserve"], 50);
    server.stop();
}

#[test]
fn work_hours_are_checked_atomically_saved_and_cleared() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    assert_eq!(state(addr)["settings"]["workHours"], Value::Null);
    assert!(state(addr)["delegation"]["handOffAt"].is_u64());
    let refused =
        "Work hours need at least one day, and an end after their start, in steps of 30 minutes.";
    for bad in [
        json!({"days": [], "from": 540, "to": 1080}),
        json!({"days": [2, 1], "from": 540, "to": 1080}),
        json!({"days": [8], "from": 540, "to": 1080}),
        json!({"days": [1], "from": 545, "to": 1080}),
        json!({"days": [1], "from": 1080, "to": 540}),
        json!({"days": [1], "from": 540, "to": 1470}),
        json!({"days": "weekdays"}),
        json!(true),
    ] {
        let reply = call(
            addr,
            "PUT",
            "/api/settings",
            Some(&json!({"workHours": bad, "theme": "dark"}).to_string()),
        );
        assert_eq!(reply.status, 400, "{bad}");
        assert_eq!(reply.json()["error"], refused);
        assert_eq!(state(addr)["settings"]["workHours"], Value::Null);
        assert_eq!(state(addr)["settings"]["theme"], "system");
    }
    let office = json!({"days": [1, 2, 3, 4, 5], "from": 540, "to": 1080});
    let set = call(
        addr,
        "PUT",
        "/api/settings",
        Some(&json!({ "workHours": office }).to_string()),
    );
    assert_eq!(set.json()["settings"]["workHours"], office);
    // Other changes leave them alone; null clears them.
    call(addr, "PUT", "/api/settings", Some(r#"{"reserve": 30}"#));
    assert_eq!(state(addr)["settings"]["workHours"], office);
    server.stop();
    let server = f.start();
    assert_eq!(state(server.addr)["settings"]["workHours"], office);
    let cleared = call(
        server.addr,
        "PUT",
        "/api/settings",
        Some(r#"{"workHours": null}"#),
    );
    assert_eq!(cleared.json()["settings"]["workHours"], Value::Null);
    server.stop();
}

#[test]
fn the_replacement_contract_and_sign_in_actions_keep_main_separate() {
    let f = Fixture::new();
    let folder = f.claude_folder(".claude-work", "work@example.com", "fixture-token");
    common::write(&f.home.path().join("bin/claude.exe"), "fixture CLI");
    let before = std::fs::read(folder.join(".credentials.json")).expect("fixture login");
    let server = f.start();
    let s = add(server.addr, "claude", &folder, "Work");
    for key in [
        "tools",
        "desktopApps",
        "routing",
        "hooksUpdated",
        "claudeTooOld",
    ] {
        assert!(s.get(key).is_none(), "removed {key}");
    }
    assert_eq!(s["main"]["id"], "main");
    assert!(s["problems"].is_array());
    assert!(s["delegation"]["mainWorksItself"].is_boolean());
    for key in ["main", "order", "desktopApp"] {
        assert!(s["accounts"][0].get(key).is_none(), "removed account {key}");
    }
    assert!(s["accounts"][0]["hooks"].is_string());
    assert_eq!(s["accounts"][0]["sameAsMain"], false);
    assert_eq!(
        call(server.addr, "POST", "/api/accounts/a1/sign-in", None).status,
        200
    );
    let signed = f.sign_ins.lock().expect("sign-ins");
    assert_eq!(signed.len(), 1);
    assert!(warden_core::detect::same_folder(&signed[0].2, &folder));
    drop(signed);
    assert_eq!(
        std::fs::read(folder.join(".credentials.json")).expect("unchanged login"),
        before
    );
    server.stop();
}

fn ids(state: &Value) -> Vec<String> {
    state["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .map(|a| a["id"].as_str().expect("id").to_owned())
        .collect()
}

#[test]
fn usage_failures_keep_http_reasons_in_state_and_logs_without_error_bodies() {
    use warden_rules::secrets::{assert_no_token, canary_token};

    let f = Fixture::new();
    let token = canary_token("usage-failure");
    let folder = f.claude_folder(".claude-work", "ann@example.com", &token);
    let credentials = folder.join(".credentials.json");
    let before = std::fs::read(&credentials).expect("fixture login");
    let server = f.start();
    add(server.addr, "claude", &folder, "Work");
    f.endpoint.settled_calls(&server);
    let reading = state(server.addr)["accounts"][0]["usage"].clone();
    assert!(reading.is_object());

    for (status, reason) in [
        (401, "login expired"),
        (403, "missing permission"),
        (429, "rate limited"),
        (500, "server error"),
        (200, "unexpected answer"),
    ] {
        *f.endpoint.answer.lock().expect("lock") = Some((status, token.clone()));
        call(server.addr, "POST", "/api/usage/refresh", None);
        f.endpoint.settled_calls(&server);
        let s = state(server.addr);
        let error = &s["accounts"][0]["usageError"];
        let message = if status == 401 {
            assert_eq!(error["kind"], "loginExpired");
            assert_eq!(error["status"], 401);
            "HTTP 401: login expired, waiting for it to be renewed"
        } else {
            assert_eq!(error["kind"], "failed");
            error["message"].as_str().expect("reason")
        };
        assert!(message.contains(&format!("HTTP {status}")), "{message}");
        assert!(message.contains(reason), "{message}");
        assert_eq!(
            s["accounts"][0]["usage"], reading,
            "keep the last good reading"
        );
        let log = f.log_text();
        assert!(
            log.contains(message),
            "the log keeps the same safe reason: {log}"
        );
        assert_no_token("usage error state", &s.to_string(), &token);
        assert_no_token("usage error log", &log, &token);
        assert_eq!(std::fs::read(&credentials).expect("fixture login"), before);
    }
    struct FailingReader(String);
    impl std::io::Read for FailingReader {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other(self.0.clone()))
        }
    }
    let id = ids(&state(server.addr))[0].clone();
    for status in [401, 403, 429, 500, 200] {
        let result = warden_core::claude::read_response(status, FailingReader(token.clone()));
        assert_no_token("reader result", &format!("{result:?}"), &token);
        server.app.apply_usage(&id, result);
        assert_no_token(
            "reader failure state",
            &state(server.addr).to_string(),
            &token,
        );
        assert_no_token("reader failure log", &f.log_text(), &token);
    }
    assert!(
        f.log_text()
            .contains("HTTP 200: the answer was cut off or not text")
    );
    *f.endpoint.answer.lock().expect("lock") = None;
    call(server.addr, "POST", "/api/usage/refresh", None);
    f.endpoint.settled_calls(&server);
    assert!(state(server.addr)["accounts"][0]["usageError"].is_null());
    server.stop();
}

#[test]
fn first_run_finds_only_warden_account_folders_and_sets_them_up() {
    let f = Fixture::new();
    f.claude_folder(".claude", "main@example.com", "main-not-used");
    f.claude_folder(".claude-second", "second@example.com", "second");
    f.codex_folder(".codex", "private@example.com", "never-used");
    // Warden's own folders come first.
    f.codex_folder(".warden/codex-spare", "spare@example.com", "spare");
    let own = f.home.path().join(".warden").join("codex-spare");
    let server = f.start();
    let addr = server.addr;
    let candidates = call(addr, "GET", "/api/setup/candidates", None).json();
    let list = candidates.as_array().expect("candidates");
    assert_eq!(list.len(), 2, "{list:?}");
    assert_eq!(list[0]["label"], "Codex spare");
    assert_eq!(list[0]["folder"], own.to_string_lossy().as_ref());
    assert_eq!(list[1]["label"], "Claude second");
    assert!(list[1].get("desktopApp").is_none());
    let chosen: Vec<Value> = list
        .iter()
        .map(|c| json!({"provider":c["provider"],"folder":c["folder"],"label":c["label"]}))
        .collect();
    let reply = call(
        addr,
        "POST",
        "/api/setup",
        Some(&json!({"accounts":chosen}).to_string()),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    let state = reply.json();
    assert_eq!(state["setupDone"], true);
    assert!(state["accounts"][0].get("main").is_none());
    assert_eq!(state["main"]["id"], "main");
    assert_eq!(
        state["main"]["folder"],
        f.home.path().join(".claude").to_string_lossy().as_ref()
    );
    assert_eq!(
        call(addr, "GET", "/api/setup/candidates", None).json()[0]["known"],
        true
    );
    server.stop();
}

#[test]
fn main_is_fixed_and_warden_accounts_are_independently_switched_and_removed() {
    let f = Fixture::new();
    let folder = f.claude_folder(".claude-work", "work@example.com", "fixture");
    let server = f.start();
    let addr = server.addr;
    add(addr, "claude", &folder, "Work");
    for (method, path, body) in [
        ("PATCH", "/api/accounts/main", Some(r#"{"enabled":false}"#)),
        ("DELETE", "/api/accounts/main", None),
        ("PATCH", "/api/accounts/a1", Some(r#"{"main":true}"#)),
    ] {
        assert_eq!(call(addr, method, path, body).status, 400);
    }
    assert_eq!(
        call(addr, "PATCH", "/api/main", Some(r#"{"label":"My main"}"#)).status,
        200
    );
    assert_eq!(
        call(
            addr,
            "PATCH",
            "/api/accounts/a1",
            Some(r#"{"enabled":false}"#)
        )
        .json()["accounts"][0]["enabled"],
        false
    );
    assert_eq!(
        call(
            addr,
            "PATCH",
            "/api/accounts/a1",
            Some(r#"{"enabled":true}"#)
        )
        .status,
        200
    );
    assert_eq!(call(addr, "DELETE", "/api/accounts/a1", None).status, 200);
    assert!(folder.is_dir());
    server.stop();
    let server = f.start();
    let state = state(server.addr);
    assert_eq!(state["main"]["label"], "My main");
    assert_eq!(ids(&state).len(), 0);
    server.stop();
}

fn add_status(addr: SocketAddr, provider: &str, folder: &std::path::Path) -> u16 {
    let body = json!({ "provider": provider, "folder": folder, "label": "x" }).to_string();
    call(addr, "POST", "/api/accounts", Some(&body)).status
}

#[test]
fn adding_refuses_folders_without_a_login() {
    let f = Fixture::new();
    let empty = f.home.path().join(".claude-empty");
    std::fs::create_dir_all(&empty).expect("mkdir");
    let server = f.start();
    assert_eq!(add_status(server.addr, "claude", &empty), 400);
    assert_eq!(
        add_status(server.addr, "claude", &f.home.path().join("missing")),
        400
    );
    assert_eq!(
        add_status(server.addr, "claude", std::path::Path::new("relative")),
        400
    );
    server.stop();
}

/// The other ways to write `folder` that this computer allows: written differently, through `..`, its 8.3 short
/// name, a junction and (where Windows lets this user make one) a link to it. Each with what it is.
fn other_ways_to_write(home: &std::path::Path, folder: &std::path::Path) -> Vec<(String, String)> {
    let text = folder.to_string_lossy().into_owned();
    let name = folder.file_name().expect("name").to_string_lossy();
    let mut ways = vec![
        ("trailing separator".into(), format!(r"{text}\")),
        ("forward slashes".into(), text.replace('\\', "/")),
        ("quoted".into(), format!(r#""{text}""#)),
        ("quoted with spaces".into(), format!(r#"  "{text}"  "#)),
        ("other case".into(), text.to_uppercase()),
        ("through ..".into(), format!(r"{text}-work\..\{name}")),
        ("through .".into(), format!(r"{}\.\{name}", home.display())),
    ];
    let real = std::fs::canonicalize(folder).expect("real folder");
    let short = (1..=4)
        .map(|n| {
            home.join(format!(
                "{}~{n}",
                name.trim_start_matches('.').to_uppercase()
            ))
        })
        .find(|p| std::fs::canonicalize(p).is_ok_and(|r| r == real));
    match short {
        Some(p) => ways.push(("8.3 short name".into(), p.to_string_lossy().into_owned())),
        None => eprintln!("no 8.3 short name on this volume; that way is not tested here"),
    }
    let links = home.join("links");
    std::fs::create_dir_all(&links).expect("mkdir");
    let junction = links.join("desktop-junction");
    junction::create(folder, &junction).expect("junction");
    ways.push(("junction".into(), junction.to_string_lossy().into_owned()));
    let link = links.join("desktop-link");
    match std::os::windows::fs::symlink_dir(folder, &link) {
        Ok(()) => ways.push(("link".into(), link.to_string_lossy().into_owned())),
        Err(e) => eprintln!("no link made ({e}); that way is not tested here"),
    }
    ways
}

#[test]
fn the_warden_folder_itself_is_never_an_account_but_a_folder_in_it_is() {
    let f = Fixture::new();
    let own = f.claude_folder(".warden", "own@example.com", "fixture-own");
    let inside = f.claude_folder(".warden/claude-work", "work@example.com", "fixture-work");
    let mut ways = vec![("as made".into(), own.to_string_lossy().into_owned())];
    ways.extend(other_ways_to_write(f.home.path(), &own));
    let server = f.start();
    for (way, folder) in ways {
        for provider in ["claude", "codex"] {
            let body = json!({"provider":provider,"folder":folder,"label":"Own"});
            let reply = call(
                server.addr,
                "POST",
                "/api/accounts",
                Some(&body.to_string()),
            );
            assert_eq!(reply.status, 400, "{way}: {}", reply.body);
            assert_eq!(
                reply.json()["error"],
                "That is Warden's folder for its accounts; choose a folder inside it."
            );
        }
    }
    let setup = json!({"accounts":[{"provider":"claude","folder":own,"label":"Own"}]});
    let reply = call(server.addr, "POST", "/api/setup", Some(&setup.to_string()));
    assert_eq!(reply.status, 400, "{}", reply.body);
    assert!(ids(&state(server.addr)).is_empty());
    let body = json!({"provider":"claude","folder":inside,"label":"Work"});
    let reply = call(
        server.addr,
        "POST",
        "/api/accounts",
        Some(&body.to_string()),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    server.stop();
}

#[test]
fn desktop_default_folders_are_refused_under_every_spelling_and_provider() {
    for (name, status) in [(".claude", 409), (".codex", 400)] {
        let f = Fixture::new();
        let folder = if name == ".claude" {
            f.claude_folder(name, "main@example.com", "fixture")
        } else {
            f.codex_folder(name, "private@example.com", "fixture")
        };
        let mut ways = vec![("as found".into(), folder.to_string_lossy().into_owned())];
        ways.extend(other_ways_to_write(f.home.path(), &folder));
        let server = f.start();
        for (way, folder) in ways {
            for provider in ["claude", "codex"] {
                for _confirmed in [false, true] {
                    let body = json!({"provider":provider,"folder":folder,"label":"Blocked"});
                    let reply = call(
                        server.addr,
                        "POST",
                        "/api/accounts",
                        Some(&body.to_string()),
                    );
                    assert_eq!(reply.status, status, "{way}: {}", reply.body);
                    if name == ".codex" {
                        assert_eq!(
                            reply.json()["error"],
                            "That is the Codex app's folder; Warden leaves it alone."
                        );
                    }
                }
            }
        }
        assert!(ids(&state(server.addr)).is_empty());
        server.stop();
    }
}

#[test]
fn setup_refuses_a_codex_desktop_folder_before_adding_any_accounts() {
    let f = Fixture::new();
    let work = f.claude_folder(".claude-work", "work@example.com", "work");
    let codex = f.codex_folder(".codex", "private@example.com", "private");
    let server = f.start();
    let mut ways = vec![("as found".into(), codex.to_string_lossy().into_owned())];
    ways.extend(other_ways_to_write(f.home.path(), &codex));
    for (way, folder) in ways {
        let body = json!({"accounts":[{"provider":"claude","folder":work,"label":"Work"},{"provider":"codex","folder":folder,"label":"Private"}]});
        let reply = call(server.addr, "POST", "/api/setup", Some(&body.to_string()));
        assert_eq!(reply.status, 400, "{way}: {}", reply.body);
    }
    assert!(ids(&state(server.addr)).is_empty());
    assert_eq!(state(server.addr)["setupDone"], false);
    server.stop();
}

#[test]
fn a_changed_identity_is_flagged_kept_apart_and_can_be_accepted() {
    let f = Fixture::new();
    let work = f.claude_folder(".claude-work", "work@acme.example", "tw");
    let personal = f.claude_folder(".claude-personal", "me@home.example", "tp");
    let server = f.start();
    let addr = server.addr;
    add(addr, "claude", &work, "Work");
    let s = add(addr, "claude", &personal, "Personal");
    let [iw, ip] = <[String; 2]>::try_from(ids(&s)).expect("two");
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    assert_eq!(s["accounts"][0]["usage"]["weekly"]["usedPercent"], 11.0);
    let calls_before = f.endpoint.settled_calls(&server);

    // The desktop app signs in as the personal account in the default folder.
    set_claude_identity(&work, "me@home.example");
    *f.endpoint.answer.lock().expect("lock") = Some((200, CLAUDE_OTHER.into()));
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    let w = &s["accounts"][0];
    assert_eq!(
        w["identity"]["email"], "work@acme.example",
        "the remembered identity stays"
    );
    assert_eq!(w["identityChange"]["now"]["email"], "me@home.example");
    assert_eq!(w["identityChange"]["knownAccountId"], ip.as_str());
    assert_eq!(w["usageError"]["kind"], "identityChanged");
    assert_eq!(
        w["usage"]["weekly"]["usedPercent"], 11.0,
        "the old reading is kept, not mixed"
    );
    assert!(
        !f.endpoint
            .tokens_since(calls_before)
            .contains(&"tw".to_owned()),
        "the changed folder's login was not used"
    );
    // Saved for routing (step 4.2) to hold work.
    let saved: Value = serde_json::from_str(
        &std::fs::read_to_string(f.data.path().join("readings.json")).expect("readings"),
    )
    .expect("json");
    assert!(saved["accounts"][&iw]["identityChange"].is_object());

    // Accepted: the account is now who the folder is signed in as.
    let s = call(
        addr,
        "POST",
        &format!("/api/accounts/{iw}/identity"),
        Some(r#"{"action": "accept"}"#),
    )
    .json();
    assert_eq!(s["accounts"][0]["identity"]["email"], "me@home.example");
    assert_eq!(s["accounts"][0]["identityChange"], Value::Null);
    server.stop();
}

const CLAUDE_OTHER: &str =
    r#"{"five_hour": null, "seven_day": {"utilization": 77.0, "resets_at": null}}"#;

#[test]
fn failures_keep_the_last_reading() {
    let f = Fixture::new();
    let folder = f.claude_folder(".claude-work", "me@acme.example", "t");
    let server = f.start();
    let addr = server.addr;
    add(addr, "claude", &folder, "Main");
    call(addr, "POST", "/api/usage/refresh", None);

    *f.endpoint.answer.lock().expect("lock") = Some((503, "busy".into()));
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    assert_eq!(s["accounts"][0]["usageError"]["kind"], "failed");
    assert_eq!(s["accounts"][0]["usage"]["fiveHour"]["usedPercent"], 61.0);

    *f.endpoint.answer.lock().expect("lock") = Some((401, String::new()));
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    assert_eq!(s["accounts"][0]["usageError"]["kind"], "loginExpired");
    assert_eq!(s["accounts"][0]["usageError"]["status"], 401);
    assert_eq!(s["accounts"][0]["usage"]["fiveHour"]["usedPercent"], 61.0);
    assert!(s["lastRead"].is_number());
    server.stop();
}

#[test]
fn an_unreadable_identity_keeps_the_last_reading() {
    let f = Fixture::new();
    let folder = f.claude_folder(".claude-work", "me@acme.example", "t");
    let server = f.start();
    let addr = server.addr;
    add(addr, "claude", &folder, "Main");
    call(addr, "POST", "/api/usage/refresh", None);
    let calls_before = f.endpoint.settled_calls(&server);

    // The settings file is half written: whose login the folder holds cannot be told.
    common::write(
        &common::claude_identity_file(&folder),
        "{\"oauthAccount\": {",
    );
    *f.endpoint.answer.lock().expect("lock") = Some((200, CLAUDE_OTHER.into()));
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    assert_eq!(s["accounts"][0]["usageError"]["kind"], "failed");
    assert_eq!(s["accounts"][0]["usage"]["fiveHour"]["usedPercent"], 61.0);
    assert_eq!(
        f.endpoint.tokens_since(calls_before),
        Vec::<String>::new(),
        "the folder's login was not used"
    );

    set_claude_identity(&folder, "me@acme.example");
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    assert_eq!(s["accounts"][0]["usageError"], Value::Null);
    assert_eq!(s["accounts"][0]["usage"]["weekly"]["usedPercent"], 77.0);
    server.stop();
}

#[test]
fn codex_usage_follows_the_logs_as_they_grow() {
    let f = Fixture::new();
    let codex = f.codex_folder(".codex-work", "me@acme.example", "t");
    let log = codex.join("sessions/2026/11/07/rollout-a.jsonl");
    common::write(
        &log,
        &format!(
            "{}\n",
            codex_token_count("2026-11-07T09:00:00Z", 10.0, 20.0)
        ),
    );
    let server = f.start();
    let addr = server.addr;
    add(addr, "codex", &codex, "Codex");
    let five = |s: &Value| s["accounts"][0]["usage"]["fiveHour"]["usedPercent"].as_f64();
    eventually("the first reading", || five(&state(addr)) == Some(10.0));

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .expect("open");
    std::io::Write::write_all(
        &mut file,
        format!(
            "{}\n",
            codex_token_count("2026-11-07T09:05:00Z", 15.0, 21.0)
        )
        .as_bytes(),
    )
    .expect("append");
    drop(file);
    eventually("the appended reading", || five(&state(addr)) == Some(15.0));

    // A new session file in a new day folder.
    let next = codex.join("sessions/2026/11/08/rollout-b.jsonl");
    common::write(
        &next,
        &format!("{}\n", codex_token_count("2026-11-08T08:00:00Z", 3.0, 22.0)),
    );
    eventually("the new file's reading", || five(&state(addr)) == Some(3.0));

    // Signing in as someone else: flagged, and the logs are kept apart.
    set_codex_identity(&codex, "other@else.example", "t2");
    eventually("the identity change", || {
        state(addr)["accounts"][0]["identityChange"].is_object()
    });
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&next)
        .expect("open");
    std::io::Write::write_all(
        &mut file,
        format!(
            "{}\n",
            codex_token_count("2026-11-08T09:00:00Z", 90.0, 90.0)
        )
        .as_bytes(),
    )
    .expect("append");
    drop(file);
    eventually("the held reading", || {
        state(addr)["accounts"][0]["usageError"]["kind"] == "identityChanged"
    });
    assert_eq!(five(&state(addr)), Some(3.0));

    // Accepted, then Warden restarts: the logs written before the change are someone else's and stay out.
    let id = ids(&state(addr)).remove(0);
    let reply = call(
        addr,
        "POST",
        &format!("/api/accounts/{id}/identity"),
        Some(r#"{"action": "accept"}"#),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    server.stop();
    let server = f.start();
    let addr = server.addr;
    eventually("the restarted server's first look at the logs", || {
        state(addr)["accounts"][0]["usageError"]["kind"] == "noLogs"
    });
    let s = state(addr);
    assert_eq!(s["accounts"][0]["identity"]["email"], "other@else.example");
    assert_eq!(five(&s), None, "no reading from the previous identity");
    server.stop();
}

#[test]
fn a_new_account_gets_a_folder_and_a_sign_in() {
    let f = Fixture::new();
    common::write(&f.home.path().join("bin/codex.cmd"), "@echo off");
    let server = f.start();
    let addr = server.addr;
    let reply = call(
        addr,
        "POST",
        "/api/accounts/new",
        Some(r#"{"provider": "claude", "label": "Claude 3"}"#),
    );
    assert_eq!(reply.status, 400, "no claude CLI: {}", reply.body);
    assert!(
        reply.json()["error"]
            .as_str()
            .is_some_and(|e| e.contains("Settings"))
    );

    // Created under `~/.warden`, which does not exist yet.
    assert!(!f.home.path().join(".warden").exists());
    let reply = call(
        addr,
        "POST",
        "/api/accounts/new",
        Some(r#"{"provider": "codex", "label": "Codex 3"}"#),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    let folder = f.home.path().join(".warden").join("codex-3");
    assert!(folder.is_dir());
    assert!(!f.home.path().join(".codex-3").exists());
    let sign_ins = f.sign_ins.lock().expect("lock").clone();
    assert_eq!(sign_ins.len(), 1);
    assert_eq!(sign_ins[0].1, f.home.path().join("bin/codex.cmd"));
    assert_eq!(sign_ins[0].2, folder);
    assert_eq!(reply.json()["accounts"][0]["label"], "Codex 3");
    server.stop();
}

#[test]
fn only_a_new_read_interval_reads_usage_again() {
    let f = Fixture::new();
    let folder = f.claude_folder(".claude-work", "work@example.com", "fixture-work");
    let server = f.start();
    let addr = server.addr;
    let body = json!({"provider": "claude", "folder": folder, "label": "Work"});
    assert_eq!(
        call(addr, "POST", "/api/accounts", Some(&body.to_string())).status,
        200
    );
    let before = f.endpoint.settled_calls(&server);
    // Theme, reserve, work hours: no reading; the decision is made from the saved readings.
    for body in [
        r#"{"theme": "dark"}"#,
        r#"{"reserve": 30}"#,
        r#"{"workHours": {"days": [1, 2, 3, 4, 5], "from": 540, "to": 1080}}"#,
        r#"{"usageIntervalMin": 5}"#,
    ] {
        assert_eq!(
            call(addr, "PUT", "/api/settings", Some(body)).status,
            200,
            "{body}"
        );
    }
    assert_eq!(
        f.endpoint.settled_calls(&server),
        before,
        "the same interval again reads nothing either"
    );
    // A new interval: the usage loop reads once and then waits the new one.
    assert_eq!(
        call(
            addr,
            "PUT",
            "/api/settings",
            Some(r#"{"usageIntervalMin": 10}"#)
        )
        .status,
        200
    );
    assert!(f.endpoint.settled_calls(&server) > before);
    server.stop();
}

#[test]
fn settings_are_checked_and_saved() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    let s = call(
        addr,
        "PUT",
        "/api/settings",
        Some(r#"{"usageIntervalMin": 10, "theme": "dark"}"#),
    )
    .json();
    assert_eq!(s["settings"]["usageIntervalMin"], 10);
    assert_eq!(s["settings"]["theme"], "dark");
    assert_eq!(
        call(
            addr,
            "PUT",
            "/api/settings",
            Some(r#"{"usageIntervalMin": 1}"#)
        )
        .status,
        400
    );
    assert_eq!(
        call(addr, "PUT", "/api/settings", Some(r#"{"color": "red"}"#)).status,
        400
    );
    assert_eq!(call(addr, "GET", "/api/nothing", None).status, 404);
    assert_eq!(
        call(addr, "POST", "/api/setup/restart", None).json()["setupDone"],
        false
    );
    server.stop();
}

#[test]
fn a_waiting_request_answers_when_something_changes() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    // Let the start-up reading finish, so only the change below moves the revision.
    eventually("the start-up reading", || server.app.usage_idle());
    let revision = state(addr)["revision"].as_u64().expect("revision");
    let waiter = std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let s = call(
            addr,
            "GET",
            &format!("/api/state/wait?since={revision}"),
            None,
        )
        .json();
        (s, started.elapsed())
    });
    std::thread::sleep(std::time::Duration::from_millis(200));
    call(addr, "PUT", "/api/settings", Some(r#"{"theme": "light"}"#));
    let (s, waited) = waiter.join().expect("waiter");
    assert_eq!(s["settings"]["theme"], "light");
    assert!(s["revision"].as_u64() > Some(revision));
    assert!(
        waited < std::time::Duration::from_secs(10),
        "answered on the change, not the timeout"
    );
    server.stop();
}

/// Stands in for the desktop app's start with Windows registration.
#[derive(Default)]
struct FakeDesktop {
    registered: std::sync::Mutex<bool>,
    fail: std::sync::atomic::AtomicBool,
    /// How many times the registration was read (the registry, in the desktop app).
    reads: std::sync::atomic::AtomicUsize,
    /// The folders "Open log folder" asked Explorer to open.
    opened: std::sync::Mutex<Vec<std::path::PathBuf>>,
    /// How many times "Install and restart" reached the desktop app.
    installs: std::sync::atomic::AtomicUsize,
}

impl warden_server::app::Desktop for FakeDesktop {
    fn start_with_windows(&self) -> bool {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *self.registered.lock().expect("lock")
    }

    fn set_start_with_windows(&self, on: bool) -> std::io::Result<()> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(std::io::Error::other("refused"));
        }
        *self.registered.lock().expect("lock") = on;
        Ok(())
    }

    fn open_folder(&self, folder: &std::path::Path) -> std::io::Result<()> {
        self.opened.lock().expect("lock").push(folder.to_owned());
        Ok(())
    }

    fn install_update(&self) -> std::io::Result<()> {
        self.installs
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn an_update_shows_in_the_state_and_installs_only_when_ready_in_the_desktop_app() {
    use warden_server::app::UpdateView;
    let f = Fixture::new();
    // The browser version has no update and installs none.
    let server = f.start();
    assert_eq!(state(server.addr)["update"], Value::Null);
    assert_eq!(
        call(server.addr, "POST", "/api/update/install", None).status,
        400
    );
    server.stop();

    let desktop = std::sync::Arc::new(FakeDesktop::default());
    let server = f.start_with(Some(desktop.clone()));
    let addr = server.addr;
    let installs = || desktop.installs.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(state(addr)["update"], Value::Null);
    assert_eq!(call(addr, "POST", "/api/update/install", None).status, 409);
    // Downloading: its version and progress; still nothing to install.
    let before = state(addr)["revision"].as_u64().expect("revision");
    let downloading = UpdateView {
        version: "0.2.0".into(),
        downloaded: Some(40),
        ready: false,
    };
    server.app.set_update(Some(downloading.clone()));
    let s = state(addr);
    assert_eq!(
        s["update"],
        json!({"version": "0.2.0", "downloaded": 40, "ready": false})
    );
    assert!(s["revision"].as_u64().expect("revision") > before);
    // The same report again changes nothing, so the window is not woken for it.
    let revision = s["revision"].as_u64();
    server.app.set_update(Some(downloading));
    assert_eq!(state(addr)["revision"].as_u64(), revision);
    assert_eq!(call(addr, "POST", "/api/update/install", None).status, 409);
    assert_eq!(installs(), 0);
    // Ready: "Install and restart" reaches the desktop app.
    server.app.set_update(Some(UpdateView {
        version: "0.2.0".into(),
        downloaded: Some(100),
        ready: true,
    }));
    assert_eq!(state(addr)["update"]["ready"], true);
    assert_eq!(call(addr, "POST", "/api/update/install", None).status, 200);
    assert_eq!(installs(), 1);
    server.app.set_update(None);
    assert_eq!(state(addr)["update"], Value::Null);
    server.stop();
}

#[test]
fn the_log_folder_opens_in_the_desktop_app_and_its_path_is_in_the_state() {
    let f = Fixture::new();
    let logs = f.data.path().join("logs");
    let server = f.start();
    let addr = server.addr;
    assert_eq!(
        state(addr)["logDir"].as_str().map(std::path::PathBuf::from),
        Some(logs.clone())
    );
    // The browser version only shows the path.
    assert_eq!(call(addr, "POST", "/api/logs/open", None).status, 400);
    server.stop();

    let desktop = std::sync::Arc::new(FakeDesktop::default());
    let server = f.start_with(Some(desktop.clone()));
    assert_eq!(
        call(server.addr, "POST", "/api/logs/open", None).status,
        200
    );
    assert_eq!(*desktop.opened.lock().expect("lock"), vec![logs.clone()]);
    server.stop();
    // The server's log lines went to today's file there.
    let files: Vec<_> = std::fs::read_dir(&logs).expect("logs").collect();
    assert_eq!(files.len(), 1, "one file a day");
}

#[test]
fn the_browser_version_has_no_desktop_settings() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    let s = state(addr);
    assert_eq!(s["desktop"], Value::Null);
    assert_eq!(s["settings"]["keepInTray"], false);
    for body in [r#"{"keepInTray": true}"#, r#"{"startWithWindows": true}"#] {
        assert_eq!(call(addr, "PUT", "/api/settings", Some(body)).status, 400);
    }
    assert_eq!(state(addr)["settings"]["keepInTray"], false);
    server.stop();
}

#[test]
fn the_desktop_app_keeps_its_tray_and_start_with_windows_settings() {
    let f = Fixture::new();
    let desktop = std::sync::Arc::new(FakeDesktop::default());
    let server = f.start_with(Some(desktop.clone()));
    let addr = server.addr;
    assert_eq!(state(addr)["desktop"]["startWithWindows"], false);
    // Read once at start, not on every answer.
    let reads = || desktop.reads.load(std::sync::atomic::Ordering::SeqCst);
    let at_start = reads();
    for _ in 0..3 {
        state(addr);
    }
    assert_eq!(
        reads(),
        at_start,
        "the state answers do not read the registry"
    );

    let s = call(
        addr,
        "PUT",
        "/api/settings",
        Some(r#"{"keepInTray": true, "startWithWindows": true}"#),
    )
    .json();
    assert_eq!(s["settings"]["keepInTray"], true);
    assert_eq!(s["desktop"]["startWithWindows"], true);
    assert_eq!(reads(), at_start + 1, "read back once after the change");
    assert!(server.app.keep_in_tray());
    let saved = std::fs::read_to_string(f.data.path().join("settings.json")).expect("saved");
    assert!(saved.contains(r#""keepInTray": true"#), "{saved}");

    let s = call(
        addr,
        "PUT",
        "/api/settings",
        Some(r#"{"startWithWindows": false}"#),
    )
    .json();
    assert_eq!(s["desktop"]["startWithWindows"], false);
    assert!(!*desktop.registered.lock().expect("lock"));

    // A refused registration changes nothing, not even the other settings sent with it.
    desktop
        .fail
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let reply = call(
        addr,
        "PUT",
        "/api/settings",
        Some(r#"{"keepInTray": false, "startWithWindows": true}"#),
    );
    assert_eq!(reply.status, 500);
    let s = state(addr);
    assert_eq!(s["desktop"]["startWithWindows"], false);
    assert_eq!(s["settings"]["keepInTray"], true);
    server.stop();
}

#[test]
fn identity_alerts_and_changes_reach_the_desktop_app() {
    let f = Fixture::new();
    let server = f.start_with(Some(std::sync::Arc::new(FakeDesktop::default())));
    let addr = server.addr;
    let folder = f.claude_folder(".claude-work", "ann@example.com", "tok-1");
    add(addr, "claude", &folder, "Work");
    assert!(server.app.identity_alerts().is_empty());

    let since = server.app.revision();
    let app = std::sync::Arc::clone(&server.app);
    let waiter = std::thread::spawn(move || app.wait_for_change(since));
    set_claude_identity(&folder, "bob@example.com");
    call(addr, "POST", "/api/usage/refresh", None);
    assert!(waiter.join().expect("waiter").is_some_and(|r| r > since));
    let alerts = server.app.identity_alerts();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].label, "Work");
    assert_eq!(alerts[0].now.as_deref(), Some("bob@example.com"));

    let app = std::sync::Arc::clone(&server.app);
    let since = server.app.revision();
    let waiter = std::thread::spawn(move || app.wait_for_change(since));
    server.app.stop();
    assert_eq!(
        waiter.join().expect("waiter"),
        None,
        "stops waiting when the server stops"
    );
    server.stop();
}

#[test]
fn a_codex_login_changed_while_warden_was_stopped_keeps_its_logs_apart() {
    let f = Fixture::new();
    let codex = f.codex_folder(".codex-work", "me@acme.example", "t");
    let log = codex.join("sessions/2026/11/07/rollout-a.jsonl");
    common::write(
        &log,
        &format!(
            "{}\n",
            codex_token_count("2026-11-07T09:00:00Z", 10.0, 20.0)
        ),
    );
    let server = f.start();
    let addr = server.addr;
    add(addr, "codex", &codex, "Codex");
    let five = |s: &Value| s["accounts"][0]["usage"]["fiveHour"]["usedPercent"].as_f64();
    eventually("the first reading", || five(&state(addr)) == Some(10.0));
    server.stop();

    // While Warden is stopped, the folder signs in as someone else, who then works in it.
    set_codex_identity(&codex, "other@else.example", "t2");
    common::write(
        &codex.join("sessions/2026/11/08/rollout-b.jsonl"),
        &format!(
            "{}\n",
            codex_token_count("2026-11-08T08:00:00Z", 90.0, 90.0)
        ),
    );
    let server = f.start();
    let addr = server.addr;
    eventually("the identity change", || {
        state(addr)["accounts"][0]["identityChange"].is_object()
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        five(&state(addr)),
        Some(10.0),
        "the other identity's log is not read into the account"
    );
    server.stop();
}

#[test]
fn wardens_sign_in_folder_is_never_a_candidate_or_account() {
    let f = Fixture::new();
    let own = f.data.path().join("main-sign-in");
    std::fs::create_dir_all(&own).expect("folder");
    common::write(
        &own.join(".credentials.json"),
        r#"{"claudeAiOauth":{"accessToken":"fixture"}}"#,
    );
    common::set_claude_identity(&own, "main@example.com");
    let server = f.start();
    assert_eq!(add_status(server.addr, "claude", &own), 400);
    assert!(
        call(server.addr, "GET", "/api/setup/candidates", None)
            .json()
            .as_array()
            .expect("candidates")
            .is_empty()
    );
    server.stop();
}

#[test]
fn main_and_warden_identity_files_are_watched_without_reading_main_credentials() {
    let f = Fixture::new();
    let main = f.claude_folder(".claude", "main@example.com", "must-not-be-sent");
    let work = f.claude_folder(".claude-work", "work@example.com", "work");
    let server = f.start();
    add(server.addr, "claude", &work, "Work");
    call(server.addr, "POST", "/api/usage/refresh", None);
    assert_eq!(
        state(server.addr)["main"]["identity"]["email"],
        "main@example.com"
    );
    set_claude_identity(&main, "other@example.com");
    set_claude_identity(&work, "other@example.com");
    eventually("main identity change", || {
        state(server.addr)["main"]["identityChange"]["now"]["email"] == "other@example.com"
    });
    eventually("warden identity change", || {
        state(server.addr)["accounts"][0]["identityChange"]["now"]["email"] == "other@example.com"
    });
    assert!(
        !f.endpoint
            .tokens_seen
            .lock()
            .expect("tokens")
            .iter()
            .any(|t| t == "must-not-be-sent")
    );
    server.stop();
}

#[test]
fn codex_plan_changes_update_saved_labels_without_changing_identity() {
    let f = Fixture::new();
    let folder = f.codex_folder(".codex-work", "ann@example.com", "fixture-only");
    let server = f.start();
    let addr = server.addr;
    let s = add(addr, "codex", &folder, "Codex");
    assert_eq!(s["accounts"][0]["identity"]["organization"], Value::Null);
    common::set_codex_plan(&folder, "ann@example.com", "fixture-only", Some("business"));
    eventually("the new plan label", || {
        state(addr)["accounts"][0]["identity"]["organization"] == "Business"
    });
    let s = state(addr);
    assert_eq!(s["accounts"][0]["identity"]["organization"], "Business");
    assert_eq!(s["accounts"][0]["identityChange"], Value::Null);
    let saved: Value = serde_json::from_str(
        &std::fs::read_to_string(f.data.path().join("settings.json")).expect("settings"),
    )
    .expect("json");
    assert_eq!(saved["accounts"][0]["identity"]["organization"], "Business");
    server.stop();
}

#[test]
fn a_codex_read_without_a_workspace_id_preserves_the_saved_id() {
    let f = Fixture::new();
    let folder = f.codex_folder(".codex-work", "ann@example.com", "fixture-only");
    let server = f.start();
    let s = add(server.addr, "codex", &folder, "Codex");
    let id = ids(&s).remove(0);
    let mut fresh = warden_core::codex::read_identity_from_login_file(&folder)
        .expect("reads")
        .expect("identity");
    fresh.organization_id = None;
    fresh.organization = Some("Plus".into());
    assert!(!server.app.apply_identity(&id, Ok(Some(fresh.clone()))));
    let saved: Value = serde_json::from_str(
        &std::fs::read_to_string(f.data.path().join("settings.json")).expect("settings"),
    )
    .expect("json");
    assert_eq!(
        saved["accounts"][0]["identity"]["organizationId"],
        "ws-ann@example.com"
    );
    assert_eq!(
        state(server.addr)["accounts"][0]["identityChange"],
        Value::Null
    );
    fresh.organization_id = Some("different-workspace".into());
    assert!(
        server.app.apply_identity(&id, Ok(Some(fresh))),
        "saved workspace still detects a different seat"
    );
    server.stop();
}

#[test]
fn both_default_claude_sign_in_files_are_watched_in_priority_order() {
    let f = Fixture::new();
    let folder = f.claude_folder(".claude", "ann@example.com", "fixture-only");
    let primary = f.home.path().join(".claude.json");
    let inner = folder.join(".claude.json");
    common::write(&primary, "{}");
    common::write(
        &inner,
        r#"{"oauthAccount":{"emailAddress":"inner@example.com"}}"#,
    );
    let server = f.start();
    call(server.addr, "POST", "/api/usage/refresh", None);
    let s = state(server.addr);
    assert_eq!(s["main"]["identity"]["email"], "inner@example.com");
    for (file, email) in [
        (&inner, "inner-next@example.com"),
        (&primary, "home@example.com"),
    ] {
        common::write(
            file,
            &json!({"oauthAccount":{"emailAddress":email}}).to_string(),
        );
        eventually("either default identity file changes", || {
            state(server.addr)["main"]["identityChange"]["now"]["email"] == email
        });
    }
    common::write(&primary, "{}");
    eventually(
        "home sign-out reveals the explicit config-dir sign-in",
        || state(server.addr)["main"]["identityChange"]["now"]["email"] == "inner-next@example.com",
    );
    server.stop();
}

#[test]
fn codex_resets_are_read_kept_through_failures_and_hidden_for_someone_else() {
    let f = Fixture::new();
    let folder = f.codex_folder(".codex-work", "me@acme.example", "codex-token");
    *f.resets.answer.lock().expect("lock") = Some((200, common::RESETS_ANSWER.into()));
    let server = f.start();
    let addr = server.addr;
    add(addr, "codex", &folder, "Codex");
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    assert_eq!(
        s["accounts"][0]["resets"],
        json!([{
            "title": "Full reset (Weekly + 5 hr)",
            "expiresAt": warden_core::time::parse_rfc3339_ms("2026-12-10T22:00:00Z"),
        }])
    );
    let saved = std::fs::read_to_string(f.data.path().join("readings.json")).expect("readings");
    assert!(saved.contains("Full reset (Weekly + 5 hr)"));
    assert!(!saved.contains("credit-1") && !saved.contains("Ready to redeem"));

    // A failed read keeps the last list and is logged once.
    *f.resets.answer.lock().expect("lock") = Some((503, "busy".into()));
    call(addr, "POST", "/api/usage/refresh", None);
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    assert_eq!(
        s["accounts"][0]["resets"][0]["title"],
        "Full reset (Weekly + 5 hr)"
    );
    assert_eq!(
        f.log_text()
            .matches("Codex: no banked resets reading (HTTP 503: server error)")
            .count(),
        1,
        "{}",
        f.log_text()
    );

    // Signed out: nothing is asked.
    *f.resets.answer.lock().expect("lock") = Some((200, common::RESETS_ANSWER.into()));
    std::fs::remove_file(folder.join("auth.json")).expect("sign out");
    let asked = f.resets.seen.lock().expect("lock").len();
    call(addr, "POST", "/api/usage/refresh", None);
    assert_eq!(f.resets.seen.lock().expect("lock").len(), asked);

    // Signed in as someone else: the old list is not shown and the other login is never sent.
    set_codex_identity(&folder, "other@acme.example", "other-token");
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    assert!(s["accounts"][0]["identityChange"].is_object());
    assert_eq!(s["accounts"][0]["resets"], json!([]));
    assert!(
        f.resets
            .seen
            .lock()
            .expect("lock")
            .iter()
            .all(|(token, _)| token == "codex-token")
    );
    server.stop();
}

/// A usage answer with the account's limit resets: two full resets in one grant and a 5-hour one.
const CLAUDE_RESETS_ANSWER: &str = r#"{
    "five_hour": {"utilization": 20.0, "resets_at": null},
    "seven_day": {"utilization": 30.0, "resets_at": null},
    "cedar_ember": {"eligible": true, "ineligible_reason": null, "grants": [
        {"id": "grant-1", "label": "One full limit reset (November 2026)", "resets_total": 2, "resets_left": 2,
         "ends_at": "2026-12-10T22:00:00Z", "clears": ["five_hour", "seven_day"], "gives_back_weekly": false},
        {"id": "grant-2", "label": "A session reset", "resets_total": 1, "resets_left": 1,
         "ends_at": "2026-12-01T08:00:00Z", "clears": ["five_hour"], "type": "session"}
    ], "event_props": {"campaign": "fixture-campaign"}}
}"#;

#[test]
fn claude_limit_resets_come_with_usage_for_main_and_warden_accounts() {
    let f = Fixture::new();
    f.main_sign_in(
        "fixture-main@example.com",
        "own-access",
        "own-refresh",
        9_999_999_999_999,
    );
    let folder = f.claude_folder(".claude-work", "work@example.com", "work-token");
    *f.endpoint.answer.lock().expect("lock") = Some((200, CLAUDE_RESETS_ANSWER.into()));
    let server = f.start();
    let addr = server.addr;
    add(addr, "claude", &folder, "Work");
    let at = |s: &str| warden_core::time::parse_rfc3339_ms(s);
    let expected = json!([
        {"title": "5-hour reset", "expiresAt": at("2026-12-01T08:00:00Z")},
        {"title": "Full reset", "expiresAt": at("2026-12-10T22:00:00Z")},
        {"title": "Full reset", "expiresAt": at("2026-12-10T22:00:00Z")},
    ]);
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    assert_eq!(s["main"]["usage"]["weekly"]["usedPercent"], 30.0);
    assert_eq!(s["main"]["resets"], expected);
    assert_eq!(s["accounts"][0]["resets"], expected);
    // Only the short names and expiries are kept, and never inside the usage reading.
    let saved = std::fs::read_to_string(f.data.path().join("readings.json")).expect("readings");
    for gone in ["grant-1", "November", "fixture-campaign", "A session reset"] {
        assert!(!saved.contains(gone), "{gone} in {saved}");
    }
    assert!(s["main"]["usage"].get("resets").is_none());

    // An answer without the block, or a failed read: the last list stays.
    *f.endpoint.answer.lock().expect("lock") = Some((200, common::CLAUDE_ANSWER.into()));
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    assert_eq!(s["accounts"][0]["resets"], expected);
    *f.endpoint.answer.lock().expect("lock") = Some((503, "busy".into()));
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    assert_eq!(s["main"]["resets"], expected);
    assert_eq!(s["accounts"][0]["resets"], expected);

    // Signed in as someone else: not shown, and kept for when the change is undone.
    set_claude_identity(&folder, "other@example.com");
    *f.endpoint.answer.lock().expect("lock") = Some((200, CLAUDE_RESETS_ANSWER.into()));
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    assert!(s["accounts"][0]["identityChange"].is_object());
    assert_eq!(s["accounts"][0]["resets"], json!([]));
    assert_eq!(s["main"]["resets"], expected);
    set_claude_identity(&folder, "work@example.com");
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    assert_eq!(s["accounts"][0]["resets"], expected);

    // Not eligible: none.
    *f.endpoint.answer.lock().expect("lock") = Some((
        200,
        r#"{"seven_day": {"utilization": 1}, "cedar_ember": {"eligible": false, "ineligible_reason": "surface", "grants": []}}"#.into(),
    ));
    let s = call(addr, "POST", "/api/usage/refresh", None).json();
    assert_eq!(s["main"]["resets"], json!([]));
    assert_eq!(s["accounts"][0]["resets"], json!([]));
    server.stop();
}

#[test]
fn the_reset_warning_is_checked_and_saved() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    assert_eq!(state(addr)["settings"]["resetWarningDays"], 3);
    for bad in [json!(0), json!(15), json!(2.5), json!("5"), json!(-1)] {
        let reply = call(
            addr,
            "PUT",
            "/api/settings",
            Some(&json!({"resetWarningDays": bad, "theme": "dark"}).to_string()),
        );
        assert_eq!(reply.status, 400, "{bad}");
        assert_eq!(state(addr)["settings"]["resetWarningDays"], 3);
        assert_eq!(state(addr)["settings"]["theme"], "system");
    }
    for days in [1, 14, 7] {
        let set = call(
            addr,
            "PUT",
            "/api/settings",
            Some(&json!({ "resetWarningDays": days }).to_string()),
        );
        assert_eq!(set.status, 200, "{}", set.body);
        assert_eq!(set.json()["settings"]["resetWarningDays"], days);
    }
    server.stop();
    let server = f.start();
    assert_eq!(state(server.addr)["settings"]["resetWarningDays"], 7);
    server.stop();
}

#[test]
fn showing_quiet_sessions_is_checked_and_saved() {
    let f = Fixture::new();
    let server = f.start();
    let addr = server.addr;
    assert_eq!(state(addr)["settings"]["showQuietSessions"], true);
    for bad in [json!("false"), json!(0), json!({}), json!(null)] {
        let reply = call(
            addr,
            "PUT",
            "/api/settings",
            Some(&json!({"showQuietSessions": bad, "theme": "dark"}).to_string()),
        );
        assert_eq!(reply.status, 400, "{bad}");
        assert_eq!(state(addr)["settings"]["showQuietSessions"], true);
        assert_eq!(state(addr)["settings"]["theme"], "system");
    }
    let set = call(
        addr,
        "PUT",
        "/api/settings",
        Some(&json!({ "showQuietSessions": false }).to_string()),
    );
    assert_eq!(set.status, 200, "{}", set.body);
    assert_eq!(set.json()["settings"]["showQuietSessions"], false);
    server.stop();
    let server = f.start();
    assert_eq!(state(server.addr)["settings"]["showQuietSessions"], false);
    server.stop();
}

/// What the stand-in endpoints were asked since `from`: per Claude token whether the request asked for the limit
/// resets, and how many Codex resets requests were sent.
fn asked_since(f: &Fixture, from: (usize, usize)) -> (Vec<(String, bool)>, usize) {
    let tokens = f.endpoint.tokens_seen.lock().expect("lock").clone();
    let asked = f.endpoint.resets_asked.lock().expect("lock").clone();
    assert_eq!(tokens.len(), asked.len());
    let claude = tokens.into_iter().zip(asked).skip(from.0).collect();
    let codex = f.resets.seen.lock().expect("lock").len() - from.1;
    (claude, codex)
}

fn asked_so_far(f: &Fixture) -> (usize, usize) {
    (
        f.endpoint.tokens_seen.lock().expect("lock").len(),
        f.resets.seen.lock().expect("lock").len(),
    )
}

#[test]
fn limit_resets_are_read_every_few_hours_on_the_interval_and_always_on_refresh() {
    let f = Fixture::new();
    f.main_sign_in(
        "fixture-main@example.com",
        "own-access",
        "own-refresh",
        9_999_999_999_999,
    );
    let claude = f.claude_folder(".claude-work", "work@example.com", "work-token");
    let codex = f.codex_folder(".codex-work", "me@acme.example", "codex-token");
    *f.endpoint.answer.lock().expect("lock") = Some((200, CLAUDE_RESETS_ANSWER.into()));
    *f.resets.answer.lock().expect("lock") = Some((200, common::RESETS_ANSWER.into()));
    let server = f.start();
    let addr = server.addr;
    add(addr, "claude", &claude, "Work");
    add(addr, "codex", &codex, "Codex");
    f.endpoint.settled_calls(&server);

    // The first interval readings (at start, and on adding each account) asked every account for its resets.
    let (first, codex_requests) = asked_since(&f, (0, 0));
    for token in ["own-access", "work-token"] {
        assert!(
            first.iter().any(|(t, asked)| t == token && *asked),
            "{token}: {first:?}"
        );
    }
    assert_eq!(codex_requests, 1);
    let s = state(addr);
    assert_eq!(s["main"]["resets"].as_array().map(Vec::len), Some(3));
    assert_eq!(s["accounts"][0]["resets"].as_array().map(Vec::len), Some(3));
    assert_eq!(s["accounts"][1]["resets"].as_array().map(Vec::len), Some(1));

    // The next interval reading, right after: the plain usage request, no Codex resets request, and the lists stay.
    let from = asked_so_far(&f);
    server.app.read_claude_usage();
    assert_eq!(
        asked_since(&f, from),
        (
            vec![("own-access".into(), false), ("work-token".into(), false)],
            0
        )
    );
    let s = state(addr);
    assert_eq!(s["main"]["resets"].as_array().map(Vec::len), Some(3));
    assert_eq!(s["accounts"][0]["resets"].as_array().map(Vec::len), Some(3));
    assert_eq!(s["accounts"][1]["resets"].as_array().map(Vec::len), Some(1));

    // Refreshed by hand: every account is asked again, main included.
    let from = asked_so_far(&f);
    call(addr, "POST", "/api/usage/refresh", None);
    assert_eq!(
        asked_since(&f, from),
        (
            vec![("own-access".into(), true), ("work-token".into(), true)],
            1
        )
    );
    let from = asked_so_far(&f);
    server.app.read_claude_usage();
    assert_eq!(
        asked_since(&f, from),
        (
            vec![("own-access".into(), false), ("work-token".into(), false)],
            0
        )
    );
    server.stop();
}

#[test]
fn a_failed_resets_read_is_asked_again_on_the_next_interval_reading() {
    let f = Fixture::new();
    f.main_sign_in(
        "fixture-main@example.com",
        "own-access",
        "own-refresh",
        9_999_999_999_999,
    );
    let claude = f.claude_folder(".claude-work", "work@example.com", "work-token");
    let codex = f.codex_folder(".codex-work", "me@acme.example", "codex-token");
    // Claude answers without a readable block (the usage alone), Codex's backend fails.
    *f.endpoint.answer.lock().expect("lock") = Some((200, common::CLAUDE_ANSWER.into()));
    *f.resets.answer.lock().expect("lock") = Some((503, "busy".into()));
    let server = f.start();
    let addr = server.addr;
    add(addr, "claude", &claude, "Work");
    add(addr, "codex", &codex, "Codex");
    f.endpoint.settled_calls(&server);

    let every = |codex| {
        (
            vec![("own-access".into(), true), ("work-token".into(), true)],
            codex,
        )
    };
    for _ in 0..2 {
        let from = asked_so_far(&f);
        server.app.read_claude_usage();
        assert_eq!(asked_since(&f, from), every(1));
    }
    // A failed Claude request does not count either.
    *f.endpoint.answer.lock().expect("lock") = Some((503, "busy".into()));
    let from = asked_so_far(&f);
    server.app.read_claude_usage();
    assert_eq!(asked_since(&f, from), every(1));
    // The failure was logged once, as before.
    assert_eq!(
        f.log_text()
            .matches("Codex: no banked resets reading (HTTP 503: server error)")
            .count(),
        1,
        "{}",
        f.log_text()
    );

    // Read at last: counted, so the next interval reading does not ask. An ineligible answer counts too.
    *f.endpoint.answer.lock().expect("lock") = Some((
        200,
        r#"{"seven_day": {"utilization": 1}, "cedar_ember": {"eligible": false, "grants": []}}"#
            .into(),
    ));
    *f.resets.answer.lock().expect("lock") = Some((200, common::RESETS_ANSWER.into()));
    let from = asked_so_far(&f);
    server.app.read_claude_usage();
    assert_eq!(asked_since(&f, from), every(1));
    let from = asked_so_far(&f);
    server.app.read_claude_usage();
    assert_eq!(
        asked_since(&f, from),
        (
            vec![("own-access".into(), false), ("work-token".into(), false)],
            0
        )
    );
    assert_eq!(
        state(addr)["accounts"][1]["resets"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    server.stop();
}
