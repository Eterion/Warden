//! `warden route` run as the hooks run it: the built binary, the hook's JSON on stdin, a fixture data folder and
//! fixture account folders (signed out, so no usage request is ever made).

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use warden_core::detect::Env;
use warden_core::hooks::{self, ClaudeShell};
use warden_core::model::Provider;
use warden_core::store::{self, AccountConfig, READINGS_FILE, Readings, SETTINGS_FILE, Settings};
use warden_core::testdir::Dir;
use warden_core::time::now_ms;

fn setup(dir: &Path, checked_at: Option<i64>) -> std::path::PathBuf {
    let data = dir.join("data");
    let account = |id: &str, label: &str, provider| AccountConfig {
        id: id.into(),
        label: label.into(),
        provider,
        folder: dir.join(id),
        enabled: true,
        identity: Some(warden_core::model::Identity {
            email: "second@example.com".into(),
            organization: None,
            organization_id: None,
        }),
    };
    for id in [".claude", "a2"] {
        std::fs::create_dir_all(dir.join(id)).expect("mkdir");
    }
    // Main's Claude Code finds Git Bash (a fixture one, named in its settings), whatever this computer has.
    let bash = dir.join("git").join("bash.exe");
    std::fs::create_dir_all(bash.parent().expect("parent")).expect("mkdir");
    std::fs::write(&bash, "").expect("bash");
    std::fs::write(
        dir.join(".claude").join("settings.json"),
        serde_json::json!({ "env": { "CLAUDE_CODE_GIT_BASH_PATH": bash } }).to_string(),
    )
    .expect("settings");
    let settings = Settings {
        accounts: vec![account("a2", "Second", Provider::Claude)],
        ..Settings::default()
    };
    store::save(&data, SETTINGS_FILE, &settings).expect("save");
    store::save(
        &data,
        READINGS_FILE,
        &Readings {
            checked_at,
            ..Readings::default()
        },
    )
    .expect("save");
    data
}

fn route(data: &Path, account: &str, input: &str) -> (Output, Duration) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_warden"));
    cmd.args(["route", "--account", account, "--data-dir"])
        .arg(data)
        .env("USERPROFILE", data.parent().expect("fixture home"))
        .env("HOME", data.parent().expect("fixture home"))
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let start = Instant::now();
    let mut child = cmd.spawn().expect("warden starts");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write");
    // Like the hook runners: waits for stdout and stderr to close, then for the process.
    let out = child.wait_with_output().expect("warden ends");
    (out, start.elapsed())
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn route_uses_the_same_normalised_reserve_as_the_server() {
    let dir = Dir::new("normalised-reserve");
    let now = now_ms();
    let data = setup(dir.path(), Some(now));
    let mut settings: Settings = store::load(&data, SETTINGS_FILE).expect("settings");
    settings.reserve = 37;
    store::save(&data, SETTINGS_FILE, &settings).expect("settings");
    let mut readings = Readings::default();
    readings.main_login.sign_in.state = warden_core::mainlogin::SignInState::SignedIn;
    readings.accounts.entry("main".into()).or_default().usage = Some(warden_core::model::Usage {
        read_at: now,
        five_hour: None,
        weekly: Some(warden_core::model::Limit {
            used_percent: 64.0,
            resets_at: Some(now + 7 * 24 * 3_600_000),
        }),
        per_model_weekly: Vec::new(),
        extra: None,
    });
    store::save(&data, READINGS_FILE, &readings).expect("readings");
    let (out, _) = route(&data, "main", r#"{"session_id":"normalised-reserve"}"#);
    assert!(
        text(&out).starts_with("Warden: work in this session."),
        "{}",
        text(&out)
    );
}

#[cfg(debug_assertions)]
#[test]
fn debug_home_overrides_a_different_fixture_profile_for_route_and_refresh() {
    let dir = Dir::new("debug-hook-home");
    let data = setup(dir.path(), Some(now_ms()));
    let decoy = dir.path().join("decoy-home");
    std::fs::create_dir_all(&decoy).expect("decoy");
    let mut child = Command::new(env!("CARGO_BIN_EXE_warden"))
        .args(["route", "--account", "main", "--data-dir"])
        .arg(&data)
        .arg("--home")
        .arg(dir.path())
        .env("USERPROFILE", &decoy)
        .env("HOME", &decoy)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("route");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(br#"{"session_id":"debug-home"}"#)
        .expect("input");
    let out = child.wait_with_output().expect("route exits");
    assert!(out.status.success());
    assert!(
        text(&out).contains("CLAUDE_CONFIG_DIR="),
        "fixture home selects Bash, not the decoy's PowerShell"
    );
    let private = dir.path().join(".codex");
    std::fs::create_dir_all(&private).expect("private folder");
    std::fs::write(
        private.join("auth.json"),
        "private fixture; must not be read",
    )
    .expect("private login");
    let mut settings: Settings = store::load(&data, SETTINGS_FILE).expect("settings");
    settings.accounts[0].provider = Provider::Codex;
    settings.accounts[0].folder = private;
    store::save(&data, SETTINGS_FILE, &settings).expect("settings");
    let out = Command::new(env!("CARGO_BIN_EXE_warden"))
        .args(["refresh", "--data-dir"])
        .arg(&data)
        .arg("--home")
        .arg(dir.path())
        .env("USERPROFILE", &decoy)
        .env("HOME", &decoy)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .output()
        .expect("refresh");
    assert!(out.status.success());
    let readings: Readings = store::load(&data, READINGS_FILE).expect("readings");
    assert!(
        !readings.accounts.contains_key("a2"),
        "the explicit home protects its Codex app folder"
    );
}

#[test]
fn tells_each_session_once_and_stays_silent_while_nothing_changes() {
    let dir = Dir::new("route-cli");
    let data = setup(dir.path(), Some(now_ms()));
    let input = r#"{"session_id":"s-1","hook_event_name":"UserPromptSubmit","prompt":"hi"}"#;
    let (first, _) = route(&data, "main", input);
    assert!(first.status.success());
    let t = text(&first);
    assert!(
        t.starts_with(
            "Warden: main is handing work off now, so run each task on Second (Warden's pick)"
        ),
        "{t}"
    );
    assert!(
        t.contains("here, only talk to the user and check results."),
        "{t}"
    );
    assert!(
        t.contains("\nRun a delegated command in the background and read its result when it finishes; a long run in the foreground is cut off.\n- Second (Warden's pick): WARDEN_PARENT_SESSION="),
        "{t}"
    );
    assert!(
        t.contains(&format!(
            "CLAUDE_CONFIG_DIR='{}' claude -p --permission-mode auto \"<task>\"",
            dir.path().join("a2").display()
        )),
        "{t}"
    );

    let (again, _) = route(&data, "main", input);
    assert!(again.status.success());
    assert_eq!(text(&again), "", "nothing changed: silent");

    let (other, _) = route(&data, "a2", r#"{"session_id":"s-2"}"#);
    assert_eq!(text(&other), "", "Warden accounts get no advice");

    // Something changes: the first session is told again.
    let mut s: Settings = store::load(&data, SETTINGS_FILE).expect("load");
    s.accounts[0].enabled = false;
    store::save(&data, SETTINGS_FILE, &s).expect("save");
    let (changed, _) = route(&data, "main", input);
    assert!(
        text(&changed).contains(
            "Warden: no other account can take delegated work now, so work in this session."
        ),
        "{}",
        text(&changed)
    );
    assert!(
        !data.join("refresh.lock").exists(),
        "fresh readings: no refresh"
    );
}

#[test]
fn used_up_delegated_weeks_tell_main_to_keep_work_once_until_the_decision_changes() {
    let dir = Dir::new("used-up-weeks");
    let now = now_ms();
    let data = setup(dir.path(), Some(now));
    let mut readings = Readings::default();
    readings.accounts.entry("a2".into()).or_default().usage = Some(warden_core::model::Usage {
        read_at: now,
        five_hour: None,
        weekly: Some(warden_core::model::Limit {
            used_percent: 100.0,
            resets_at: Some(now + 3_600_000),
        }),
        per_model_weekly: Vec::new(),
        extra: None,
    });
    store::save(&data, READINGS_FILE, &readings).expect("readings");
    let input = r#"{"session_id":"used-up-weeks"}"#;
    let (first, _) = route(&data, "main", input);
    assert!(first.status.success());
    assert!(text(&first).contains(
        "Warden: no other account can take delegated work now, so work in this session."
    ));
    // The used-up account is still listed, with why and its command, so the user can still choose it.
    assert!(
        text(&first).contains(
            "
- Second (skipped: its week is used up): "
        ),
        "{}",
        text(&first)
    );
    assert!(text(&route(&data, "main", input).0).is_empty());
    readings
        .accounts
        .get_mut("a2")
        .expect("a2")
        .usage
        .as_mut()
        .expect("usage")
        .weekly
        .as_mut()
        .expect("week")
        .used_percent = 99.0;
    store::save(&data, READINGS_FILE, &readings).expect("readings");
    assert!(text(&route(&data, "main", input).0).starts_with(
        "Warden: main is handing work off now, so run each task on Second (Warden's pick)"
    ));
    assert!(text(&route(&data, "main", input).0).is_empty());
}

#[test]
fn a_claude_session_is_told_its_commands_for_the_shell_claude_code_uses() {
    let dir = Dir::new("route-shell");
    let data = setup(dir.path(), Some(now_ms()));
    let input = r#"{"session_id":"s-1"}"#;
    let second = dir.path().join("a2");
    let (bash, _) = route(&data, "main", input);
    assert!(
        text(&bash).contains(&format!(
            "CLAUDE_CONFIG_DIR='{}' claude -p --permission-mode auto \"<task>\"",
            second.display()
        )),
        "{}",
        text(&bash)
    );
    // Without the fixture Git Bash, Claude Code uses what this computer has: the session is told again only when
    // that is PowerShell, and then in PowerShell.
    std::fs::remove_file(dir.path().join(".claude").join("settings.json")).expect("remove");
    let (now, _) = route(&data, "main", input);
    match hooks::claude_shell(&dir.path().join(".claude"), &Env::from_process()) {
        ClaudeShell::Bash => assert_eq!(text(&now), "", "same shell: silent"),
        ClaudeShell::PowerShell => {
            assert!(text(&now).contains("$env:CLAUDE_CONFIG_DIR"));
            assert!(text(&now).contains("$env:WARDEN_PARENT_SESSION = 's-1'"));
            assert!(text(&now).contains("finally"));
        }
    }
}

#[test]
fn never_fails_the_message() {
    let dir = Dir::new("route-empty");
    let (o, _) = route(&dir.path().join("nothing-here"), "a1", "not json");
    assert!(o.status.success());
    assert_eq!(text(&o), "");
    assert!(o.stderr.is_empty());
}

#[test]
fn parent_ids_come_only_from_valid_hook_input() {
    let dir = Dir::new("route-parent");
    let data = setup(dir.path(), Some(now_ms()));
    for (input, expected) in [
        (r#"{"session_id":"parent-1_OK"}"#, true),
        (r#"{"session_id":"bad';exit"}"#, false),
        (r#"{"session_id":"../bad"}"#, false),
        ("{}", false),
    ] {
        let (out, _) = route(&data, "main", input);
        assert!(out.status.success());
        assert_eq!(
            text(&out).contains("WARDEN_PARENT_SESSION="),
            expected,
            "{}",
            text(&out)
        );
    }
}

#[test]
fn known_main_usage_changes_its_advice_at_the_reserve_line() {
    let dir = Dir::new("route-reserve");
    let data = setup(dir.path(), Some(now_ms()));
    let now = now_ms();
    let mut readings: Readings = store::load(&data, READINGS_FILE).expect("readings");
    readings.main_login.sign_in.state = warden_core::mainlogin::SignInState::SignedIn;
    readings.accounts.entry("main".into()).or_default().usage = Some(warden_core::model::Usage {
        read_at: now,
        five_hour: None,
        weekly: Some(warden_core::model::Limit {
            used_percent: 10.0,
            resets_at: Some(now + 7 * 24 * 3_600_000),
        }),
        per_model_weekly: vec![],
        extra: None,
    });
    store::save(&data, READINGS_FILE, &readings).expect("save");
    let input = r#"{"session_id":"main-session"}"#;
    let (first, _) = route(&data, "main", input);
    assert!(text(&first).starts_with("Warden: work in this session."));
    let usage = readings
        .accounts
        .get_mut("main")
        .expect("main")
        .usage
        .as_mut()
        .expect("usage");
    usage.weekly.as_mut().expect("week").used_percent = 80.0;
    store::save(&data, READINGS_FILE, &readings).expect("save");
    let (changed, _) = route(&data, "main", input);
    assert!(text(&changed).starts_with(
        "Warden: main is handing work off now, so run each task on Second (Warden's pick)"
    ));
    let (same, _) = route(&data, "main", input);
    assert_eq!(text(&same), "");
}

fn refresh(data: &Path) -> Duration {
    let start = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_warden"))
        .args(["refresh", "--if-stale", "--account", "a1", "--data-dir"])
        .arg(data)
        .env("USERPROFILE", data)
        .env("HOME", data)
        .stdin(Stdio::null())
        .output()
        .expect("warden refresh runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    start.elapsed()
}

#[test]
fn old_readings_are_answered_as_they_are_and_refreshed_by_the_async_handler() {
    let dir = Dir::new("route-stale");
    let data = setup(dir.path(), Some(now_ms() - 60 * 60_000));
    let (o, _) = route(&data, "main", r#"{"session_id":"s-1"}"#);
    assert!(o.status.success());
    assert!(
        text(&o).contains("Warden: main is handing work off now"),
        "{}",
        text(&o)
    );
    assert!(
        !data.join("refresh.lock").exists(),
        "warden route starts nothing"
    );

    refresh(&data);
    let r: Readings = store::load(&data, READINGS_FILE).expect("load");
    let checked = r.checked_at.expect("checked");
    assert!(checked > now_ms() - 60_000);
    // The fixture folders have no login: read as signed out, without any request.
    assert_eq!(
        r.accounts.get("a2").and_then(|a| a.error.clone()),
        Some(warden_core::model::UsageError::SignedOut)
    );
    assert!(!data.join("refresh.lock").exists(), "the lock is released");

    // Fresh now: the next refresh does nothing.
    refresh(&data);
    let r: Readings = store::load(&data, READINGS_FILE).expect("load");
    assert_eq!(r.checked_at, Some(checked));
    // While another refresh runs, none starts.
    let r0: Readings = store::load(&data, READINGS_FILE).expect("load");
    store::save(
        &data,
        READINGS_FILE,
        &Readings {
            checked_at: None,
            ..r0
        },
    )
    .expect("save");
    std::fs::write(data.join("refresh.lock"), "").expect("lock");
    refresh(&data);
    let r: Readings = store::load(&data, READINGS_FILE).expect("load");
    assert_eq!(r.checked_at, None);
}
