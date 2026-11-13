//! The `warden` command-line tool.
//!
//! ```text
//! warden route [--account <id>] [--data-dir <folder>]          the routing hook: where to run work, when it changed
//! warden refresh [--if-stale] [--account <id>] [--data-dir <folder>]   reads every account's usage once
//! warden event --account <id> [--data-dir <folder>]            the status hook: sends the session's event to Warden
//! ```
//!
//! `warden route` reads the hook's input (JSON on stdin, for the session id), Warden's saved settings and readings,
//! and prints the guidance only when it differs from what this session was last told. It answers from the saved
//! readings however old they are, and never fails the user's message: any problem means it prints nothing.
//!
//! The routing hook runs `warden refresh --if-stale` beside it as an async hook (the CLI does not wait for it): when
//! the readings are older than the usage interval and a minute, it reads them again, so the next message is current.
//! Starting the refresh from `warden route` itself would make the CLI wait for it, since a started process keeps the
//! hook's output open.
//!
//! `warden event` reads the hook's event from stdin, keeps only what `warden_core::pipe::HookEvent` keeps (never
//! the prompt, tool input or output, or messages) and sends it to the running core over the per-user pipe. It always
//! exits 0, within a second however the core behaves, at once when Warden is not running, and prints nothing.

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};
use warden_core::claude::AnthropicUsageEndpoint;
use warden_core::hooks::{self, ClaudeShell};
use warden_core::model::Provider;
use warden_core::store::{READINGS_FILE, Readings, SETTINGS_FILE, Settings};
use warden_core::time::now_ms;
use warden_core::{detect, pipe, refresh, routing};

/// What each session was last told, one small file per session (so concurrent sessions never write the same file),
/// so `warden route` stays silent while nothing changes.
const SESSIONS_DIR: &str = "route-sessions";
/// Sessions remembered at most; the oldest are forgotten first.
const MAX_SESSIONS: usize = 200;
/// Sessions last told this long ago are forgotten (and told again).
const SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
/// The most of the hook's input read from stdin.
const MAX_INPUT: u64 = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Route {
        account: Option<String>,
        data_dir: Option<PathBuf>,
        #[cfg(debug_assertions)]
        home: Option<PathBuf>,
    },
    Refresh {
        if_stale: bool,
        data_dir: Option<PathBuf>,
        #[cfg(debug_assertions)]
        home: Option<PathBuf>,
    },
    Unhook {
        data_dir: Option<PathBuf>,
        #[cfg(debug_assertions)]
        home: Option<PathBuf>,
    },
    Event {
        account: String,
        data_dir: Option<PathBuf>,
    },
}

fn parse(args: &[String]) -> Option<Command> {
    let (cmd, rest) = args.split_first()?;
    let mut account = None;
    let mut data_dir = None;
    let mut if_stale = false;
    #[cfg(debug_assertions)]
    let mut home = None;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--account" => account = Some(it.next()?.clone()),
            "--data-dir" => data_dir = Some(PathBuf::from(it.next()?)),
            "--if-stale" if cmd == "refresh" => if_stale = true,
            #[cfg(debug_assertions)]
            "--home" if cmd == "route" || cmd == "refresh" || cmd == "unhook" => {
                home = Some(PathBuf::from(it.next()?))
            }
            _ => return None,
        }
    }
    match cmd.as_str() {
        "route" => Some(Command::Route {
            account,
            data_dir,
            #[cfg(debug_assertions)]
            home,
        }),
        // The account is accepted (the hook names it) but every account is read.
        "refresh" => Some(Command::Refresh {
            if_stale,
            data_dir,
            #[cfg(debug_assertions)]
            home,
        }),
        "unhook" if account.is_none() => Some(Command::Unhook {
            data_dir,
            #[cfg(debug_assertions)]
            home,
        }),
        "event" => Some(Command::Event {
            account: account?,
            data_dir,
        }),
        _ => None,
    }
}

/// Reads a saved file without ever changing it (the server owns them): missing or unreadable gives the default.
fn read_json<T: for<'de> serde::Deserialize<'de> + Default>(dir: &Path, name: &str) -> T {
    std::fs::read(dir.join(name))
        .ok()
        .and_then(|b| {
            if name == SETTINGS_FILE
                && serde_json::from_slice::<serde_json::Value>(&b)
                    .ok()?
                    .get("version")?
                    .as_u64()
                    != Some(2)
            {
                return None;
            }
            serde_json::from_slice(&b).ok()
        })
        .unwrap_or_default()
}

/// The session id from the hook's input (Claude and Codex both send `session_id`).
fn session_id(input: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(input).ok()?;
    let id = v.get("session_id")?.as_str()?;
    (!id.is_empty() && id.len() <= 200).then(|| id.to_owned())
}

fn read_input() -> String {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return String::new();
    }
    let mut input = String::new();
    let _ = stdin.lock().take(MAX_INPUT).read_to_string(&mut input);
    input
}

/// The file that remembers a session: its id as a safe file name (session ids are UUIDs; anything else is hashed).
fn session_file(dir: &Path, session: &str) -> PathBuf {
    let safe = session.len() <= 100
        && session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let name = if safe {
        session.to_owned()
    } else {
        // FNV-1a: stable, and good enough to tell sessions apart.
        let h = session.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
        format!("h{h:016x}")
    };
    dir.join(name)
}

/// Forgets sessions told more than a week ago, and the oldest beyond the cap.
fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    let mut kept: Vec<(SystemTime, PathBuf)> = Vec::new();
    for e in entries.flatten() {
        let Ok(at) = e.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if now.duration_since(at).is_ok_and(|age| age > SESSION_TTL) {
            let _ = std::fs::remove_file(e.path());
        } else {
            kept.push((at, e.path()));
        }
    }
    if kept.len() > MAX_SESSIONS {
        kept.sort();
        for (_, p) in &kept[..kept.len() - MAX_SESSIONS] {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// What `warden route` prints for this session: the guidance, unless the session was already told the same.
/// `claude_shell` is the shell Claude Code runs commands in for the session's account.
fn route_text(
    settings: &Settings,
    readings: &Readings,
    sessions_dir: &Path,
    account: Option<&str>,
    claude_shell: ClaudeShell,
    session: &str,
    now: i64,
) -> Option<String> {
    // A folder signed in as someone other than its account: its sessions get no routing until that is settled.
    // An account Warden no longer has (such as a desktop app's, switched off) gets nothing either.
    if settings.version != 2
        || account != Some(warden_core::store::MAIN_ID)
        || readings.identity_changed(warden_core::store::MAIN_ID)
    {
        return None;
    }
    let g = routing::guidance(
        settings,
        readings,
        account,
        claude_shell,
        Some(session),
        now,
    );
    if g.text.is_empty() {
        return None;
    }
    let fallback = format!("account-{}", account.unwrap_or("none"));
    let file = session_file(
        sessions_dir,
        if session.is_empty() {
            &fallback
        } else {
            session
        },
    );
    if std::fs::read_to_string(&file).is_ok_and(|told| told == g.key) {
        return None;
    }
    if std::fs::create_dir_all(sessions_dir).is_ok() {
        let _ = warden_core::store::write_atomic(&file, g.key.as_bytes());
        prune(sessions_dir);
    }
    Some(g.text)
}

fn route(account: Option<String>, data_dir: &Path, home: Option<&Path>) {
    let input = read_input();
    let mut settings: Settings = read_json(data_dir, SETTINGS_FILE);
    let mut env = detect::Env::from_process();
    if let Some(home) = home {
        env.home = Some(home.to_owned());
    }
    settings.normalise();
    settings.retain_warden_folders(&env, data_dir);
    let readings: Readings = read_json(data_dir, READINGS_FILE);
    let session = session_id(&input).unwrap_or_default();
    // The shell Claude Code runs this session's commands in, decided as for its hooks (from files only).
    let claude_shell = account
        .as_deref()
        .and_then(|id| settings.account_in(&env, id))
        .filter(|a| a.provider == Provider::Claude)
        .map_or_else(ClaudeShell::default, |a| {
            hooks::claude_shell(&a.folder, &env)
        });
    if let Some(text) = route_text(
        &settings,
        &readings,
        &data_dir.join(SESSIONS_DIR),
        account.as_deref(),
        claude_shell,
        &session,
        now_ms(),
    ) {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
}

fn refresh_now(data_dir: &Path, if_stale: bool, home: Option<&Path>) -> ExitCode {
    if if_stale {
        let settings: Settings = read_json(data_dir, SETTINGS_FILE);
        let readings: Readings = read_json(data_dir, READINGS_FILE);
        if !routing::readings_are_stale(&settings, &readings, now_ms()) {
            return ExitCode::SUCCESS;
        }
    }
    // One refresh at a time: overlapping messages start no second one.
    if !refresh::claim(data_dir) {
        return ExitCode::SUCCESS;
    }
    let result = refresh::refresh(
        data_dir,
        &AnthropicUsageEndpoint::new(),
        &warden_core::mainlogin::AnthropicSignInServer::default(),
        now_ms,
        home.or(warden_core::detect::Env::from_process().home.as_deref()),
    );
    refresh::release(data_dir);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("warden refresh: {}", e.kind());
            ExitCode::FAILURE
        }
    }
}

/// The status hook: the event, cut down, to the core. Any problem means the event is dropped. Reading the input
/// and sending it share one deadline ([`pipe::SEND_TIMEOUT`]): an input that never closes or a core that never
/// answers cannot hold the hook up (the process exits, and the thread with it).
fn send_event(account: String, data_dir: &Path) {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let dir = data_dir.to_owned();
    std::thread::spawn(move || {
        if let Some(mut event) = pipe::HookEvent::from_hook_input(std::io::stdin().lock()) {
            event.parent_session_id = std::env::var("WARDEN_PARENT_SESSION")
                .ok()
                .filter(|id| pipe::valid_session_id(id));
            let _ = pipe::send(&dir, &pipe::Message { account, event });
        }
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(pipe::SEND_TIMEOUT);
}

/// Uninstall can run before the desktop process closes. It removes hooks only: no login reads,
/// renewal, readings writes or server lock. The live server reports missing hooks; it does not reinstall them.
/// The uninstaller's command: Warden's hooks out of main and every account, from their backups. A development build
/// takes `--home` too, so the hook checks on a test home never reach the real main folder.
fn unhook(data_dir: &Path, home: Option<&Path>) -> std::io::Result<()> {
    let mut env = detect::Env::from_process();
    if let Some(home) = home {
        // As the server does with a test home: inherited account-folder overrides do not lead out of it.
        env.home = Some(home.to_owned());
        env.claude_config_dir = None;
        env.codex_home = None;
    }
    let mut failure = None;
    let mut accounts = match std::fs::read(data_dir.join(SETTINGS_FILE)) {
        Ok(bytes) => match warden_core::store::cleanup_accounts(&bytes, &env, data_dir) {
            Ok((accounts, invalid)) => {
                if invalid {
                    failure = Some(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid account folder list",
                    ));
                }
                accounts
            }
            Err(e) => {
                failure = Some(e);
                Vec::new()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            failure = Some(e);
            Vec::new()
        }
    };
    // Main needs no settings file. Always attempt it, even after a parse, read or earlier hook failure.
    accounts.extend(Settings::default().main_account(&env));
    for account in accounts {
        if let Err(error) = hooks::sync(
            account.provider,
            &account.folder,
            &account.id,
            data_dir,
            &hooks::Want::default(),
        ) {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = parse(&args) else {
        eprintln!(
            "usage: warden route [--account <id>] [--data-dir <folder>]\n       warden refresh [--if-stale] [--account <id>] [--data-dir <folder>]\n       warden event --account <id> [--data-dir <folder>]\n       warden unhook [--data-dir <folder>]"
        );
        return ExitCode::from(2);
    };
    #[cfg(debug_assertions)]
    let home = match &cmd {
        Command::Route { home, .. }
        | Command::Refresh { home, .. }
        | Command::Unhook { home, .. } => home.clone(),
        Command::Event { .. } => None,
    };
    #[cfg(not(debug_assertions))]
    let home: Option<PathBuf> = None;
    let (Command::Route { data_dir, .. }
    | Command::Refresh { data_dir, .. }
    | Command::Event { data_dir, .. }
    | Command::Unhook { data_dir, .. }) = &cmd;
    let Some(data_dir) = data_dir
        .clone()
        .or_else(warden_core::store::default_data_dir)
    else {
        if matches!(cmd, Command::Unhook { .. }) {
            return ExitCode::FAILURE;
        }
        // No data folder: nothing to say. The hook must never fail the user's message.
        return ExitCode::SUCCESS;
    };
    match cmd {
        Command::Route { account, .. } => {
            route(account, &data_dir, home.as_deref());
            ExitCode::SUCCESS
        }
        Command::Refresh { if_stale, .. } => refresh_now(&data_dir, if_stale, home.as_deref()),
        Command::Unhook { .. } => match unhook(&data_dir, home.as_deref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Warden could not remove its hooks ({}).", e.kind());
                ExitCode::FAILURE
            }
        },
        Command::Event { account, .. } => {
            send_event(account, &data_dir);
            ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use warden_core::store::AccountConfig;
    use warden_core::testdir::Dir;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn parses_commands() {
        assert_eq!(
            parse(&s(&["route"])),
            Some(Command::Route {
                #[cfg(debug_assertions)]
                home: None,
                account: None,
                data_dir: None
            })
        );
        assert_eq!(
            parse(&s(&["route", "--account", "a2", "--data-dir", "D"])),
            Some(Command::Route {
                #[cfg(debug_assertions)]
                home: None,
                account: Some("a2".into()),
                data_dir: Some("D".into())
            })
        );
        assert_eq!(
            parse(&s(&["refresh", "--data-dir", "D"])),
            Some(Command::Refresh {
                #[cfg(debug_assertions)]
                home: None,
                if_stale: false,
                data_dir: Some("D".into())
            })
        );
        assert_eq!(
            parse(&s(&["refresh", "--if-stale", "--account", "a1"])),
            Some(Command::Refresh {
                #[cfg(debug_assertions)]
                home: None,
                if_stale: true,
                data_dir: None
            })
        );
        assert_eq!(
            parse(&s(&["event", "--account", "a1", "--data-dir", "D"])),
            Some(Command::Event {
                account: "a1".into(),
                data_dir: Some("D".into())
            })
        );
        // The status hook names its account.
        assert_eq!(parse(&s(&["event"])), None);
        assert_eq!(parse(&s(&["route", "--if-stale"])), None);
        assert_eq!(parse(&s(&[])), None);
        assert_eq!(parse(&s(&["route", "--account"])), None);
        assert_eq!(parse(&s(&["route", "--what"])), None);
    }

    #[test]
    fn reads_the_session_id() {
        assert_eq!(
            session_id(r#"{"session_id":"abc","prompt":"hi"}"#).as_deref(),
            Some("abc")
        );
        assert_eq!(session_id("not json"), None);
        assert_eq!(session_id(r#"{"session_id":""}"#), None);
    }

    #[test]
    fn unhook_accepts_only_its_data_folder_option() {
        assert_eq!(
            parse(&s(&["unhook"])),
            Some(Command::Unhook {
                data_dir: None,
                #[cfg(debug_assertions)]
                home: None
            })
        );
        assert_eq!(parse(&s(&["unhook", "--account", "main"])), None);
        assert_eq!(parse(&s(&["unhook", "--if-stale"])), None);
        assert_eq!(parse(&s(&["unhook", "--data-dir"])), None);
    }

    #[test]
    fn home_is_only_a_debug_route_refresh_and_unhook_option() {
        for command in ["route", "refresh", "unhook"] {
            let parsed = parse(&s(&[command, "--home", "fixture-home"]));
            #[cfg(debug_assertions)]
            assert!(matches!(
                parsed,
                Some(
                    Command::Route { home: Some(_), .. }
                        | Command::Refresh { home: Some(_), .. }
                        | Command::Unhook { home: Some(_), .. }
                )
            ));
            #[cfg(not(debug_assertions))]
            assert!(parsed.is_none());
        }
        assert!(
            parse(&s(&[
                "event",
                "--account",
                "main",
                "--home",
                "fixture-home"
            ]))
            .is_none()
        );
    }

    fn one_account() -> Settings {
        Settings {
            accounts: vec![AccountConfig {
                id: "a1".into(),
                label: "Main".into(),
                provider: Provider::Claude,
                folder: r"C:\<m>".into(),
                enabled: true,
                identity: None,
            }],
            ..Settings::default()
        }
    }

    #[test]
    fn tells_each_session_once_until_something_changes() {
        let dir = Dir::new("sessions");
        let settings = one_account();
        let readings = Readings::default();
        let d = dir.path();
        let first = route_text(
            &settings,
            &readings,
            d,
            Some("main"),
            ClaudeShell::Bash,
            "s1",
            1_000,
        );
        assert!(first.is_some_and(|t| t.contains("no other account can take delegated work now")));
        assert_eq!(
            route_text(
                &settings,
                &readings,
                d,
                Some("main"),
                ClaudeShell::Bash,
                "s1",
                2_000
            ),
            None
        );
        assert!(
            route_text(
                &settings,
                &readings,
                d,
                Some("main"),
                ClaudeShell::Bash,
                "s2",
                2_000
            )
            .is_some()
        );
        // Odd session ids get a safe file name.
        assert!(
            route_text(
                &settings,
                &readings,
                d,
                Some("main"),
                ClaudeShell::Bash,
                "../x:y",
                2_000
            )
            .is_some()
        );
        assert_eq!(
            route_text(
                &settings,
                &readings,
                d,
                Some("main"),
                ClaudeShell::Bash,
                "../x:y",
                2_000
            ),
            None
        );
        assert!(session_file(d, "../x:y").starts_with(d));
        // Claude Code moving to PowerShell (Git Bash removed): the session is told again, in PowerShell.
        let again = route_text(
            &settings,
            &readings,
            d,
            Some("main"),
            ClaudeShell::PowerShell,
            "s1",
            2_000,
        );
        assert!(again.is_some_and(|t| t.contains("no other account can take delegated work now")));
        assert_eq!(
            route_text(
                &settings,
                &readings,
                d,
                Some("main"),
                ClaudeShell::PowerShell,
                "s1",
                2_000
            ),
            None
        );
        // Nothing set up: nothing to say.
        assert_eq!(
            route_text(
                &Settings::default(),
                &readings,
                d,
                None,
                ClaudeShell::Bash,
                "s3",
                2_000
            ),
            None
        );
    }

    #[test]
    fn a_session_signed_in_as_someone_else_gets_no_routing() {
        let dir = Dir::new("sessions-signed-in");
        let settings = one_account();
        let mut readings = Readings::default();
        readings
            .accounts
            .entry("main".into())
            .or_default()
            .identity_change = Some(warden_core::model::IdentityChange {
            now: None,
            known_account_id: None,
        });
        let told = |r: &Readings| {
            route_text(
                &settings,
                r,
                dir.path(),
                Some("main"),
                ClaudeShell::Bash,
                "s1",
                1_000,
            )
        };
        assert_eq!(told(&readings), None);
        // Settled: the session is told, as it was told nothing before.
        assert!(told(&Readings::default()).is_some());
        // An account Warden no longer has (a desktop app's, switched off) is told nothing.
        assert_eq!(
            route_text(
                &settings,
                &Readings::default(),
                dir.path(),
                Some("gone"),
                ClaudeShell::Bash,
                "s2",
                1_000
            ),
            None
        );
    }

    #[test]
    fn forgets_the_oldest_sessions_beyond_the_cap() {
        let dir = Dir::new("sessions-cap");
        let settings = one_account();
        for i in 0..(MAX_SESSIONS + 10) {
            route_text(
                &settings,
                &Readings::default(),
                dir.path(),
                Some("main"),
                ClaudeShell::Bash,
                &format!("s{i}"),
                0,
            );
        }
        let n = std::fs::read_dir(dir.path()).expect("dir").count();
        assert_eq!(n, MAX_SESSIONS);
    }
}
