//! The app and standalone refresh use only Warden's own sign-in, through fake transports.
mod common;

use common::{Fixture, set_claude_identity, write};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use warden_core::fallback::Done;
use warden_core::mainlogin::{RenewalError, Renewed};
use warden_core::store;
use warden_server::app::App;

fn app(f: &Fixture) -> App {
    app_with(f, Arc::new(Mutex::new(Vec::new())), Arc::new(|_| {}))
}

/// The app with its sign-ins' `done` callbacks kept, to finish a sign-in when the test says so.
fn app_with(f: &Fixture, dones: Arc<Mutex<Vec<Done>>>, log: warden_server::app::Log) -> App {
    let sign_ins = f.sign_ins.clone();
    App::new(
        f.data.path().to_owned(),
        f.env(),
        (
            f.endpoint.clone(),
            f.sign_in_server.clone(),
            f.resets.clone(),
        ),
        Arc::new(move |p, cli, folder, done| {
            sign_ins
                .lock()
                .expect("lock")
                .push((p, cli.to_owned(), folder.to_owned()));
            dones.lock().expect("lock").push(done);
            Ok(())
        }),
        log,
        Some(f.route_exe.clone()),
        None,
    )
    .expect("app")
}

#[test]
fn a_finished_sign_in_reads_usage_at_once_instead_of_at_the_next_interval() {
    let f = Fixture::new();
    f.claude_folder(".claude", "main@example.com", "app-never-sent");
    write(&f.home.path().join("bin/claude.exe"), "fixture");
    let dones = Arc::new(Mutex::new(Vec::new()));
    let lines = Arc::new(Mutex::new(String::new()));
    let log = {
        let lines = Arc::clone(&lines);
        Arc::new(move |line: &str| {
            lines.lock().expect("lock").push_str(&format!(
                "{line}
"
            ))
        })
    };
    let app = app_with(&f, Arc::clone(&dones), log).shared();
    app.sign_in_main().expect("official sign-in");
    // Starting the sign-in asks for a reading (nothing to read yet); the loop takes that one.
    assert!(app.wait(Duration::ZERO));
    let done = dones.lock().expect("lock").pop().expect("a sign-in");
    done();
    let started = Instant::now();
    assert!(app.wait(Duration::from_secs(30)));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "woken, not timed out"
    );
    assert!(
        lines
            .lock()
            .expect("lock")
            .contains("Main: the sign-in window closed")
    );
}

fn state(app: &App) -> Value {
    serde_json::to_value(app.state()).expect("state")
}

fn renew(f: &Fixture, result: Result<Renewed, RenewalError>) {
    f.sign_in_server
        .answers
        .lock()
        .expect("lock")
        .push_back(result);
}

fn renewed() -> Renewed {
    Renewed {
        access_token: "own-new".to_owned().into(),
        refresh_token: Some("own-next".to_owned().into()),
        expires_in: 28_800,
    }
}

#[test]
fn matching_own_sign_in_renews_in_the_app_and_in_standalone_refresh() {
    let f = Fixture::new();
    f.claude_folder(".claude", "main@example.com", "app-never-sent");
    let own = f.main_sign_in("main@example.com", "own-old", "own-refresh", 0);
    renew(&f, Ok(renewed()));
    let app = app(&f);
    app.read_claude_usage();
    let s = state(&app);
    assert_eq!(
        s["main"]["signIn"],
        json!({"state":"signedIn","email":"main@example.com"})
    );
    assert_eq!(s["main"]["usage"]["weekly"]["usedPercent"], 11.0);
    assert_eq!(
        *f.endpoint.tokens_seen.lock().expect("lock"),
        vec!["own-new"]
    );
    let saved: Value =
        serde_json::from_slice(&std::fs::read(own.join(".credentials.json")).expect("login"))
            .expect("json");
    assert_eq!(saved["claudeAiOauth"]["refreshToken"], "own-next");
    drop(app);
    f.main_sign_in("main@example.com", "own-old", "own-refresh", 0);
    renew(&f, Ok(renewed()));
    let later = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_millis() as i64
        + 1000;
    warden_core::refresh::refresh(
        f.data.path(),
        f.endpoint.as_ref(),
        f.sign_in_server.as_ref(),
        || later,
        Some(f.home.path()),
    )
    .expect("refresh");
    assert_eq!(f.sign_in_server.tokens_seen.lock().expect("lock").len(), 2);
    assert!(
        f.endpoint
            .tokens_seen
            .lock()
            .expect("lock")
            .iter()
            .all(|t| t == "own-new")
    );
    let readings: store::Readings =
        store::load(f.data.path(), store::READINGS_FILE).expect("readings");
    assert_eq!(
        readings.accounts["main"]
            .usage
            .as_ref()
            .expect("usage")
            .read_at,
        later
    );
}

#[test]
fn another_sign_in_and_a_changed_main_never_spend_the_own_login() {
    let f = Fixture::new();
    let main = f.claude_folder(".claude", "main@example.com", "app-never-sent");
    let own = f.main_sign_in("other@example.com", "own-old", "own-refresh", 0);
    let app = app(&f);
    app.read_claude_usage();
    assert_eq!(
        state(&app)["main"]["signIn"],
        json!({"state":"otherAccount","email":"other@example.com"})
    );
    set_claude_identity(&own, "main@example.com");
    renew(&f, Ok(renewed()));
    app.read_claude_usage();
    assert_eq!(state(&app)["main"]["signIn"]["state"], "signedIn");
    set_claude_identity(&main, "changed@example.com");
    app.read_claude_usage();
    assert!(state(&app)["main"]["identityChange"].is_object());
    assert_eq!(state(&app)["main"]["signIn"]["state"], "otherAccount");
    assert_eq!(f.sign_in_server.tokens_seen.lock().expect("lock").len(), 1);
    assert_eq!(f.endpoint.tokens_seen.lock().expect("lock").len(), 1);
}

#[test]
fn refused_renewal_survives_restart_and_waits_for_explicit_fresh_sign_in() {
    for status in [400, 401] {
        let f = Fixture::new();
        f.claude_folder(".claude", "main@example.com", "app-never-sent");
        let own = f.main_sign_in("main@example.com", "own-old", "own-refresh", 0);
        write(&f.home.path().join("bin/claude.exe"), "fixture");
        renew(&f, Err(RenewalError::Refused(status)));
        let lines = Arc::new(Mutex::new(String::new()));
        let log = {
            let lines = Arc::clone(&lines);
            Arc::new(move |line: &str| {
                lines.lock().expect("lock").push_str(&format!(
                    "{line}
"
                ))
            })
        };
        let first = app_with(&f, Arc::new(Mutex::new(Vec::new())), log);
        first.read_claude_usage();
        assert_eq!(state(&first)["main"]["signIn"]["state"], "expired");
        first.read_claude_usage();
        // Said once, when it turned expired; never the login.
        let said = lines.lock().expect("lock").clone();
        assert_eq!(
            said.matches("Warden's sign-in to main was refused on renewal; sign in again")
                .count(),
            1,
            "{said}"
        );
        assert!(
            !said.contains("own-old") && !said.contains("own-refresh"),
            "{said}"
        );
        drop(first);
        let next = app(&f);
        next.read_claude_usage();
        assert_eq!(state(&next)["main"]["signIn"]["state"], "expired");
        let before = std::fs::read(own.join(".credentials.json")).expect("old login");
        next.sign_in_main().expect("official sign-in");
        assert!(warden_core::detect::same_folder(
            &f.sign_ins.lock().expect("lock")[0].2,
            &own
        ));
        next.read_claude_usage();
        assert_eq!(state(&next)["main"]["signIn"]["state"], "signedOut");
        assert_eq!(
            std::fs::read(own.join(".credentials.json")).expect("still old"),
            before
        );
        assert_eq!(f.sign_in_server.tokens_seen.lock().expect("lock").len(), 1);
        f.main_sign_in(
            "main@example.com",
            "official-fresh-access",
            "official-fresh-refresh",
            9_999_999_999_999,
        );
        next.read_claude_usage();
        assert_eq!(state(&next)["main"]["signIn"]["state"], "signedIn");
        assert_eq!(
            *f.endpoint.tokens_seen.lock().expect("lock"),
            vec!["official-fresh-access"]
        );
    }
}

#[test]
fn transport_failure_keeps_the_reading_and_retries_on_the_next_read() {
    let f = Fixture::new();
    f.claude_folder(".claude", "main@example.com", "app-never-sent");
    f.main_sign_in(
        "main@example.com",
        "own-fresh",
        "own-refresh",
        9_999_999_999_999,
    );
    let app = app(&f);
    app.read_claude_usage();
    let before = state(&app)["main"]["usage"].clone();
    f.main_sign_in("main@example.com", "own-expired", "own-refresh", 0);
    renew(
        &f,
        Err(RenewalError::Failed("fixture transport failure".into())),
    );
    app.read_claude_usage();
    assert_eq!(state(&app)["main"]["usage"], before);
    assert_eq!(state(&app)["main"]["signIn"]["state"], "signedIn");
    renew(&f, Ok(renewed()));
    app.read_claude_usage();
    assert_eq!(f.sign_in_server.tokens_seen.lock().expect("lock").len(), 2);
    assert!(state(&app)["main"]["error"].is_null());
}

#[test]
fn main_sign_in_endpoint_prepares_only_wardens_folder() {
    let f = Fixture::new();
    f.claude_folder(".claude", "main@example.com", "app-never-sent");
    write(&f.home.path().join("bin/claude.exe"), "fixture");
    let before = common::folder_bytes(f.home.path());
    let server = f.start();
    let reply = common::call(server.addr, "POST", "/api/main/sign-in", None);
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert!(warden_core::detect::same_folder(
        &f.sign_ins.lock().expect("lock")[0].2,
        &warden_core::mainlogin::folder(f.data.path())
    ));
    server.stop();
    assert_eq!(common::folder_bytes(f.home.path()), before);
}
