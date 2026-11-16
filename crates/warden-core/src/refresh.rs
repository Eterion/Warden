//! Reading every account's usage once, outside the server: what `warden route` starts in the background when the
//! saved readings are old (Warden not running). The server reads on its own interval and never needs this.

use crate::claude::{self, UsageEndpoint};
use crate::codex::{self, LogTail};
use crate::model::{Provider, UsageError};
use crate::store::{self, IdentityUpdate, READINGS_FILE, Readings};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The file that marks a refresh in progress, so overlapping hooks start only one.
pub const LOCK_FILE: &str = "refresh.lock";
/// A refresh that left its mark this long ago has died; another may start.
const LOCK_EXPIRES: Duration = Duration::from_secs(120);

fn lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LOCK_FILE)
}

/// Claims the refresh. `false`: another refresh is running.
pub fn claim(data_dir: &Path) -> bool {
    let path = lock_path(data_dir);
    let create = || OpenOptions::new().write(true).create_new(true).open(&path);
    match create() {
        Ok(_) => true,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let stale = fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age > LOCK_EXPIRES);
            stale && fs::remove_file(&path).is_ok() && create().is_ok()
        }
        Err(_) => false,
    }
}

pub fn release(data_dir: &Path) {
    let _ = fs::remove_file(lock_path(data_dir));
}

/// Reads every account's identity and usage once and saves the readings: Claude from the usage endpoint (one
/// request per account, never asking for the limit resets: routing does not use them, and the last list stays),
/// Codex from the end of its newest session logs. Identity first: a folder signed in as someone else gets no
/// reading. A first-seen identity is left for the server to remember; this never writes the settings.
pub fn refresh(
    data_dir: &Path,
    endpoint: &dyn UsageEndpoint,
    sign_in_server: &dyn crate::mainlogin::SignInServer,
    now: impl Fn() -> i64,
    home: Option<&Path>,
) -> io::Result<()> {
    // The server's lock, held for the whole refresh: a running server reads usage itself and is the only writer of
    // the readings, and a server starting meanwhile waits until this is saved.
    let server = match store::hold_server_lock(data_dir) {
        Ok(lock) => lock,
        Err(e) if store::is_held(&e) => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut readings: Readings = store::load(data_dir, READINGS_FILE)?;
    let env = crate::detect::Env {
        home: home.map(Path::to_owned),
        ..crate::detect::Env::default()
    };
    let mut settings = store::load_settings(data_dir, &env, |_| {})?;
    settings.normalise();
    settings.retain_warden_folders(&env, data_dir);
    let expected = settings.main_account(&env).and_then(|main| {
        let observed = claude::read_identity(&main.folder, home).ok().flatten();
        readings.apply_identity(&settings, &main, observed.clone());
        observed
    });
    let result = crate::mainlogin::read_for_main(
        &server,
        expected.as_ref(),
        readings.identity_changed(crate::store::MAIN_ID),
        &mut readings.main_login,
        // Routing does not use the limit resets: the plain usage request.
        (endpoint, false),
        sign_in_server,
        now(),
    );
    // This process holds the server lock, so it is the only writer of the log too.
    let lines = readings.main_login.take_log_lines();
    if !lines.is_empty() {
        let log = crate::logfile::RollingLog::new(crate::logfile::log_dir(data_dir));
        for line in lines {
            log.write(&line);
        }
    }
    readings.apply_usage(crate::store::MAIN_ID, result);
    for account in &settings.accounts {
        let id = &account.id;
        let who = match account.provider {
            Provider::Claude => claude::read_identity(&account.folder, home),
            Provider::Codex => codex::read_identity_from_login_file(&account.folder),
        };
        // Who the folder is signed in as could not be read: its login might be someone else's, so the last reading
        // stays until the identity can be checked again (as the server does).
        let unchecked = who.is_err() && account.identity.is_some();
        // Unknown (not readable): whatever was known stays. A first-seen identity is the server's to remember.
        if let Ok(who) = who {
            let _: IdentityUpdate = readings.apply_identity(&settings, account, who);
        }
        if readings.identity_changed(id) {
            readings.apply_usage(id, Err(UsageError::IdentityChanged));
            continue;
        }
        let result = match account.provider {
            Provider::Claude if unchecked => Err(UsageError::Failed {
                message: "could not check who the folder is signed in as".into(),
            }),
            Provider::Claude => claude::read_usage(&account.folder, endpoint, now(), false),
            Provider::Codex if !codex::has_login(&account.folder) => Err(UsageError::SignedOut),
            Provider::Codex => match LogTail::new().start(&account.folder) {
                Some(u) => Ok(u),
                None if readings.accounts.get(id).is_none_or(|r| r.usage.is_none()) => {
                    Err(UsageError::NoLogs)
                }
                None => continue,
            },
        };
        readings.apply_usage(id, result);
    }
    readings.checked_at = Some(now());
    store::save(data_dir, READINGS_FILE, &readings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::Secret;
    use crate::store::AccountConfig;
    use crate::store::{SETTINGS_FILE, Settings};
    use crate::testdir::Dir;
    use std::sync::Mutex;

    struct NeverRenew;
    impl crate::mainlogin::SignInServer for NeverRenew {
        fn renew(
            &self,
            _: &Secret,
        ) -> Result<crate::mainlogin::Renewed, crate::mainlogin::RenewalError> {
            panic!("no renewal expected");
        }
    }

    /// Counts calls; never reaches the network. `warden refresh` never asks for the limit resets: routing does not use
    /// them.
    struct Fake(Mutex<u32>);
    impl UsageEndpoint for Fake {
        fn fetch(&self, _token: &Secret, resets: bool) -> Result<crate::model::Usage, UsageError> {
            assert!(!resets, "warden refresh sends the plain usage request");
            *self.0.lock().expect("lock") += 1;
            claude::read_response(
                200,
                r#"{"five_hour":{"utilization":12,"resets_at":null},"seven_day":{"utilization":34,"resets_at":null}}"#
                    .as_bytes(),
            )
        }
    }

    #[test]
    fn the_lock_lets_one_refresh_run() {
        let dir = Dir::new("lock");
        assert!(claim(dir.path()));
        assert!(!claim(dir.path()));
        release(dir.path());
        assert!(claim(dir.path()));
    }

    #[test]
    fn refresh_reads_claude_and_marks_the_time() {
        let dir = Dir::new("refresh");
        let data = dir.path().join("data");
        let signed_in = dir.path().join("claude-a");
        let signed_out = dir.path().join("claude-b");
        fs::create_dir_all(&signed_in).expect("mkdir");
        fs::create_dir_all(&signed_out).expect("mkdir");
        fs::write(
            signed_in.join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"t-123","expiresAt":99999999999999}}"#,
        )
        .expect("write");
        let account = |id: &str, folder: &Path| AccountConfig {
            id: id.into(),
            label: id.into(),
            provider: Provider::Claude,
            folder: folder.to_owned(),
            enabled: true,
            identity: None,
        };
        let settings = Settings {
            accounts: vec![account("a1", &signed_in), account("a2", &signed_out)],
            ..Settings::default()
        };
        store::save(&data, SETTINGS_FILE, &settings).expect("save");
        let fake = Fake(Mutex::new(0));
        refresh(&data, &fake, &NeverRenew, || 1_000, None).expect("refresh");
        let r: Readings = store::load(&data, READINGS_FILE).expect("load");
        assert_eq!(r.checked_at, Some(1_000));
        assert_eq!(
            *fake.0.lock().expect("lock"),
            1,
            "one request, for the signed-in account"
        );
        let a1 = r.accounts.get("a1").expect("a1");
        assert_eq!(
            a1.usage
                .as_ref()
                .and_then(|u| u.weekly.as_ref())
                .map(|l| l.used_percent),
            Some(34.0)
        );
        assert_eq!(
            r.accounts.get("a2").and_then(|a| a.error.clone()),
            Some(UsageError::SignedOut)
        );
    }

    #[test]
    fn a_running_server_is_left_to_read_usage_itself() {
        let dir = Dir::new("refresh-server");
        let _held = store::hold_server_lock(dir.path()).expect("lock");
        assert!(store::server_running(dir.path()));
        refresh(dir.path(), &Fake(Mutex::new(0)), &NeverRenew, || 1, None).expect("refresh");
        assert!(!dir.path().join(READINGS_FILE).exists(), "nothing written");
        drop(_held);
        assert!(!store::server_running(dir.path()));
    }

    #[test]
    fn a_signed_out_codex_folder_is_read_as_signed_out() {
        let dir = Dir::new("refresh-codex-out");
        let data = dir.path().join("data");
        let folder = dir.path().join("codex");
        fs::create_dir_all(&folder).expect("mkdir");
        let settings = Settings {
            accounts: vec![AccountConfig {
                id: "a1".into(),
                label: "a1".into(),
                provider: Provider::Codex,
                folder,
                enabled: true,
                identity: None,
            }],
            ..Settings::default()
        };
        store::save(&data, SETTINGS_FILE, &settings).expect("save");
        refresh(&data, &Fake(Mutex::new(0)), &NeverRenew, || 1, None).expect("refresh");
        let r: Readings = store::load(&data, READINGS_FILE).expect("load");
        assert_eq!(r.accounts["a1"].error, Some(UsageError::SignedOut));
    }

    /// Checks, while the request is out, that the server's lock is held.
    struct LockHeld(std::path::PathBuf);
    impl UsageEndpoint for LockHeld {
        fn fetch(&self, _token: &Secret, resets: bool) -> Result<crate::model::Usage, UsageError> {
            assert!(!resets);
            assert!(
                store::server_running(&self.0),
                "the refresh holds the server's lock"
            );
            claude::read_response(
                200,
                r#"{"five_hour":{"utilization":1,"resets_at":null},"seven_day":{"utilization":2,"resets_at":null}}"#
                    .as_bytes(),
            )
        }
    }

    #[test]
    fn the_refresh_holds_the_servers_lock_while_it_runs() {
        let dir = Dir::new("refresh-lock-held");
        let data = dir.path().join("data");
        let a1 = dir.path().join("claude-a");
        fs::create_dir_all(&a1).expect("mkdir");
        fs::write(
            a1.join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"t-1","expiresAt":99999999999999}}"#,
        )
        .expect("write");
        let settings = Settings {
            accounts: vec![AccountConfig {
                id: "a1".into(),
                label: "a1".into(),
                provider: Provider::Claude,
                folder: a1,
                enabled: true,
                identity: None,
            }],
            ..Settings::default()
        };
        store::save(&data, SETTINGS_FILE, &settings).expect("save");
        refresh(&data, &LockHeld(data.clone()), &NeverRenew, || 5_000, None).expect("refresh");
        assert!(!store::server_running(&data), "released afterwards");
        let r: Readings = store::load(&data, READINGS_FILE).expect("load");
        assert!(r.accounts["a1"].usage.is_some());
    }

    #[test]
    fn a_folder_back_on_its_own_login_takes_work_again() {
        let dir = Dir::new("refresh-identity-back");
        let data = dir.path().join("data");
        let folder = dir.path().join("codex");
        fs::create_dir_all(&folder).expect("mkdir");
        fs::write(
            folder.join("auth.json"),
            crate::codex::tests::login_file(r#"{"email":"me@example.com"}"#, "t-1"),
        )
        .expect("write");
        let me = crate::model::Identity {
            email: "me@example.com".into(),
            organization: None,
            organization_id: None,
        };
        let settings = Settings {
            accounts: vec![AccountConfig {
                id: "a1".into(),
                label: "a1".into(),
                provider: Provider::Codex,
                folder,
                enabled: true,
                identity: Some(me),
            }],
            ..Settings::default()
        };
        store::save(&data, SETTINGS_FILE, &settings).expect("save");
        let mut r = Readings::default();
        r.accounts.entry("a1".into()).or_default().identity_change =
            Some(crate::model::IdentityChange {
                now: None,
                known_account_id: None,
            });
        store::save(&data, READINGS_FILE, &r).expect("save");
        refresh(&data, &Fake(Mutex::new(0)), &NeverRenew, || 5_000, None).expect("refresh");
        let r: Readings = store::load(&data, READINGS_FILE).expect("load");
        assert!(!r.identity_changed("a1"));
    }
}
