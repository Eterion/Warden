//! Warden's own data, in its data folder: `settings.json` (settings and accounts) and `readings.json` (the last
//! usage reading per account, for the Overview and for `warden route`). Neither ever holds a token.

use crate::identity::{self, Check};
use crate::model::{Identity, IdentityChange, Provider, Usage, UsageError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const SETTINGS_FILE: &str = "settings.json";
pub const READINGS_FILE: &str = "readings.json";
pub const MAIN_ID: &str = "main";
/// Warden's data folder unless `--data-dir` names another: `%APPDATA%\Warden`, shared by the server, the desktop
/// app and `warden route`.
pub fn default_data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .filter(|v| !v.is_empty())
        .map(|d| PathBuf::from(d).join("Warden"))
}

/// Held open, unshared, by the running server: while it is, the server is the only writer of the readings.
pub const SERVER_LOCK_FILE: &str = "server.lock";

/// Proof that this process owns a data folder's server lock; renewal derives its folder from this owner.
pub struct ServerLock {
    _file: fs::File,
    data_dir: PathBuf,
}

impl ServerLock {
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}

/// Opens the lock unshared. Windows releases it on exit; another server or refresh cannot hold it meanwhile.
pub fn hold_server_lock(data_dir: &Path) -> io::Result<ServerLock> {
    fs::create_dir_all(data_dir)?;
    let mut o = fs::OpenOptions::new();
    o.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.share_mode(0);
    }
    Ok(ServerLock {
        _file: o.open(data_dir.join(SERVER_LOCK_FILE))?,
        data_dir: fs::canonicalize(data_dir)?,
    })
}

/// Whether opening the lock failed because someone holds it (Windows' sharing violation).
pub fn is_held(e: &io::Error) -> bool {
    const SHARING_VIOLATION: i32 = 32;
    e.raw_os_error() == Some(SHARING_VIOLATION)
}

/// Whether a server (or a refresh) holds the data folder's lock right now.
pub fn server_running(data_dir: &Path) -> bool {
    match hold_server_lock(data_dir) {
        Ok(_) => false,
        Err(e) => is_held(&e),
    }
}

/// The usage intervals offered in Settings, in minutes.
pub const USAGE_INTERVALS: [u32; 4] = [2, 5, 10, 30];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountConfig {
    pub id: String,
    pub label: String,
    pub provider: Provider,
    pub folder: PathBuf,
    /// Whether this Warden account takes delegated work.
    pub enabled: bool,
    /// Who it was signed in as when added, or last confirmed.
    pub identity: Option<Identity>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub version: u32,
    pub setup_done: bool,
    pub usage_interval_min: u32,
    pub theme: Theme,
    /// Desktop app: closing the window leaves Warden running in the tray instead of quitting.
    pub keep_in_tray: bool,
    /// Warden accounts only; main is the separate fixed record below.
    pub accounts: Vec<AccountConfig>,
    pub next_id: u32,
    /// A fresh namespace after settings are set aside; old hooks can never name a new account.
    pub id_generation: u64,
    pub main: MainConfig,
    pub reserve: u8,
    /// The user's usual work hours; `None` means every hour counts. Main's reserve shrinks with the work hours left
    /// before its weekly reset (`routing`). A value of the wrong shape loads as `None`, never failing the file.
    #[serde(deserialize_with = "lenient_work_hours")]
    pub work_hours: Option<WorkHours>,
    /// How many days before a limit reset expires the Overview warns of it: whole days, [`RESET_WARNING_DAYS`]. A value
    /// of the wrong type or out of range loads as [`DEFAULT_RESET_WARNING_DAYS`], never failing the file.
    #[serde(deserialize_with = "lenient_reset_warning_days")]
    pub reset_warning_days: u8,
}

/// The days before a limit reset expires that Settings offers for its warning.
pub const RESET_WARNING_DAYS: std::ops::RangeInclusive<u8> = 1..=14;
pub const DEFAULT_RESET_WARNING_DAYS: u8 = 3;

fn lenient_reset_warning_days<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value
        .and_then(|v| v.as_u64())
        .and_then(|v| u8::try_from(v).ok())
        .filter(|v| RESET_WARNING_DAYS.contains(v))
        .unwrap_or(DEFAULT_RESET_WARNING_DAYS))
}

fn lenient_work_hours<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<WorkHours>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

/// The days and hours the user usually works: one window, the same on every work day, in local time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkHours {
    /// ISO weekdays, 1 = Monday to 7 = Sunday, ascending, no repeats, at least one.
    pub days: Vec<u8>,
    /// Minutes after local midnight, in steps of 30, `from < to`; no overnight hours.
    pub from: u16,
    pub to: u16,
}

impl WorkHours {
    /// What the setting starts at when first switched on: Monday to Friday, 09:00 to 18:00.
    pub fn first() -> WorkHours {
        WorkHours {
            days: vec![1, 2, 3, 4, 5],
            from: 9 * 60,
            to: 18 * 60,
        }
    }

    pub fn valid(&self) -> bool {
        !self.days.is_empty()
            && self.days.windows(2).all(|w| w[0] < w[1])
            && self.days.iter().all(|d| (1..=7).contains(d))
            && self.from.is_multiple_of(30)
            && self.to.is_multiple_of(30)
            && self.from < self.to
            && self.to <= 24 * 60
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MainConfig {
    pub label: String,
    pub identity: Option<Identity>,
}

impl Default for MainConfig {
    fn default() -> Self {
        Self {
            label: "Main".into(),
            identity: None,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: 2,
            setup_done: false,
            usage_interval_min: 5,
            theme: Theme::System,
            keep_in_tray: false,
            accounts: Vec::new(),
            next_id: 1,
            id_generation: 0,
            main: MainConfig::default(),
            reserve: 25,
            work_hours: None,
            reset_warning_days: DEFAULT_RESET_WARNING_DAYS,
        }
    }
}

impl Settings {
    /// Main's location is always derived from the current environment, never from saved settings.
    pub fn main_account(&self, env: &crate::detect::Env) -> Option<AccountConfig> {
        let folder = env.default_folder(Provider::Claude)?;
        if env.in_codex_app(&folder) {
            return None;
        }
        Some(AccountConfig {
            id: MAIN_ID.into(),
            label: self.main.label.clone(),
            identity: self.main.identity.clone(),
            provider: Provider::Claude,
            folder,
            enabled: true,
        })
    }

    pub fn all_accounts(&self, env: &crate::detect::Env) -> Vec<AccountConfig> {
        self.main_account(env)
            .into_iter()
            .chain(self.accounts.iter().cloned())
            .collect()
    }

    pub fn account_in(&self, env: &crate::detect::Env, id: &str) -> Option<AccountConfig> {
        if id == MAIN_ID {
            self.main_account(env)
        } else {
            self.account(id).cloned()
        }
    }

    /// Apply the same folder boundary in the server and the standalone CLI before any account file is opened.
    pub fn retain_warden_folders(&mut self, env: &crate::detect::Env, data_dir: &Path) {
        self.accounts.retain(|a| {
            a.id != MAIN_ID
                && !env.in_codex_app(&a.folder)
                && env.desktop_app_of(&a.folder) != Some(Provider::Claude)
                && !crate::detect::same_folder(&a.folder, &crate::mainlogin::folder(data_dir))
        });
    }
    /// A new account id, never reused.
    pub fn new_id(&mut self) -> String {
        let id = if self.id_generation == 0 {
            format!("a{}", self.next_id)
        } else {
            format!("a{}-g{}", self.next_id, self.id_generation)
        };
        self.next_id += 1;
        id
    }

    pub fn account(&self, id: &str) -> Option<&AccountConfig> {
        self.accounts.iter().find(|a| a.id == id)
    }

    /// Keep the interval offered by the app and the reserve in range, rounded down to its five-percent steps.
    pub fn normalise(&mut self) {
        if !USAGE_INTERVALS.contains(&self.usage_interval_min) {
            self.usage_interval_min = 5;
        }
        self.reserve = self.reserve.min(50) / 5 * 5;
        // A hand-edited value never breaks delegation: anything invalid means every hour counts.
        if self.work_hours.as_ref().is_some_and(|w| !w.valid()) {
            self.work_hours = None;
        }
        if !RESET_WARNING_DAYS.contains(&self.reset_warning_days) {
            self.reset_warning_days = DEFAULT_RESET_WARNING_DAYS;
        }
        self.version = 2;
        self.accounts.retain(|a| a.id != MAIN_ID);
    }
}

/// What Warden last knows about one account's usage and identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountReading {
    /// The last good reading.
    pub usage: Option<Usage>,
    /// Why the latest attempt gave none.
    pub error: Option<UsageError>,
    /// Set while the folder is signed in as someone else; routing (step 4.2) holds work until it is confirmed.
    pub identity_change: Option<IdentityChange>,
    /// When the folder's current identity was accepted (ms since the epoch). Readings from before it belong to the
    /// previous identity, such as Codex's older session logs read again after a restart, and are never used.
    pub identity_since: Option<i64>,
    /// The account's limit resets as last read (a Claude account's from its usage answer, a Codex account's banked
    /// resets), soonest to expire first; only their short names and expiries.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub resets: Vec<crate::codex_resets::Reset>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Readings {
    pub main_login: crate::mainlogin::MainLogin,
    pub accounts: BTreeMap<String, AccountReading>,
    /// When usage was last read for every account (by the server's interval or `warden route`'s refresh).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<i64>,
}

/// What a fresh identity read changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityUpdate {
    Unchanged,
    /// The account had no identity yet: remember this one in the settings.
    FirstSeen(Identity),
    /// Signed in as someone else; `new`: not seen before (worth a log line).
    Changed {
        new: bool,
    },
}

/// What a usage attempt changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageUpdate {
    /// Older than the reading kept: ignored.
    Ignored,
    Read,
    /// No reading; `new`: a different error from the last one (worth a log line).
    Failed {
        new: bool,
    },
}

impl Readings {
    /// Records the identity read from an account's folder (`None`: signed out, which is not a change of identity).
    pub fn apply_identity(
        &mut self,
        settings: &Settings,
        account: &AccountConfig,
        now: Option<Identity>,
    ) -> IdentityUpdate {
        let check = identity::check(settings, account, now);
        let reading = self.accounts.entry(account.id.clone()).or_default();
        match check {
            Check::Unchanged => {
                if reading.identity_change.take().is_some()
                    && reading.error == Some(UsageError::IdentityChanged)
                {
                    reading.error = None;
                }
                IdentityUpdate::Unchanged
            }
            Check::FirstSeen(ident) => IdentityUpdate::FirstSeen(ident),
            Check::Changed(change) => {
                let new = reading.identity_change.as_ref() != Some(&change);
                reading.identity_change = Some(change);
                IdentityUpdate::Changed { new }
            }
        }
    }

    /// Records a usage attempt: a reading replaces the last one; an error is kept beside the last good reading.
    pub fn apply_usage(&mut self, id: &str, result: Result<Usage, UsageError>) -> UsageUpdate {
        let reading = self.accounts.entry(id.to_owned()).or_default();
        match result {
            Ok(mut u) => {
                let resets = u.resets.take();
                if reading.identity_since.is_some_and(|t| u.read_at < t) {
                    // From before the current identity: someone else's usage.
                    if reading.usage.is_none() && reading.error.is_none() {
                        reading.error = Some(UsageError::NoLogs);
                        return UsageUpdate::Failed { new: false };
                    }
                    return UsageUpdate::Ignored;
                }
                if reading
                    .usage
                    .as_ref()
                    .is_some_and(|old| old.read_at > u.read_at)
                {
                    return UsageUpdate::Ignored;
                }
                reading.usage = Some(u);
                reading.error = None;
                // A Claude answer without a readable list of limit resets keeps the last one.
                if let Some(resets) = resets {
                    reading.resets = resets;
                }
                UsageUpdate::Read
            }
            Err(e) => {
                let new = reading.error.as_ref() != Some(&e);
                reading.error = Some(e);
                UsageUpdate::Failed { new }
            }
        }
    }

    /// Records whether the folder has a login at all: signed out keeps the last reading for display, but the
    /// account gets no work until it is signed in again.
    pub fn set_signed_out(&mut self, id: &str, signed_out: bool) {
        let reading = self.accounts.entry(id.to_owned()).or_default();
        if signed_out {
            reading.error = Some(UsageError::SignedOut);
        } else if reading.error == Some(UsageError::SignedOut) {
            reading.error = None;
        }
    }

    pub fn identity_changed(&self, id: &str) -> bool {
        self.accounts
            .get(id)
            .is_some_and(|r| r.identity_change.is_some())
    }
}

/// Only the folder list from old settings is used for cleanup, never its logins or routing preferences.
pub fn cleanup_accounts(
    bytes: &[u8],
    env: &crate::detect::Env,
    data: &Path,
) -> io::Result<(Vec<AccountConfig>, bool)> {
    let raw: serde_json::Value = serde_json::from_slice(bytes).map_err(io::Error::other)?;
    if !matches!(raw["version"].as_u64(), Some(1 | 2)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported settings version",
        ));
    }
    let mut accounts = Vec::new();
    let mut invalid = !raw["accounts"].is_array();
    for a in raw["accounts"].as_array().into_iter().flatten() {
        let (Some(id), Some(folder), Ok(provider)) = (
            a["id"].as_str(),
            a["folder"].as_str(),
            serde_json::from_value::<Provider>(a["provider"].clone()),
        ) else {
            invalid = true;
            continue;
        };
        // Resolve aliases before considering any file inside the folder. The default Codex tree stays private.
        let folder = crate::detect::real_folder(Path::new(folder.trim().trim_matches('"').trim()));
        if env.in_codex_app(&folder)
            || crate::detect::same_folder(&folder, &crate::mainlogin::folder(data))
        {
            continue;
        }
        accounts.push(AccountConfig {
            id: id.into(),
            label: id.into(),
            provider,
            folder,
            enabled: false,
            identity: None,
        });
    }
    Ok((accounts, invalid))
}

/// Settings reset starts a new id namespace, and attempts cleanup of each old v1 archive once, before any new
/// hooks can be installed. Archives and the cleanup receipt are kept; the Codex app's folder is never opened.
pub fn load_settings(
    data: &Path,
    env: &crate::detect::Env,
    log: impl Fn(&str),
) -> io::Result<Settings> {
    let mut settings: Settings = load(data, SETTINGS_FILE)?;
    let mut archives: Vec<_> = fs::read_dir(data)?
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name == "settings.json.bad"
                || name
                    .strip_prefix("settings.json.bad.")
                    .is_some_and(|n| n.parse::<u64>().is_ok())
        })
        .collect();
    archives.sort_by_key(|e| e.file_name());
    if !data.join(SETTINGS_FILE).exists() {
        settings.id_generation = archives.len() as u64;
        // Persist the namespace even if the app exits before setup is finished.
        save(data, SETTINGS_FILE, &settings)?;
    }
    let receipt = "settings-cleanup.json";
    let mut cleaned: Vec<String> = load(data, receipt)?;
    for archive in archives {
        let name = archive.file_name().to_string_lossy().into_owned();
        if cleaned.contains(&name) {
            continue;
        }
        let bytes = fs::read(archive.path())?;
        let is_v1 = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .is_some_and(|v| v["version"] == 1);
        if !is_v1 {
            continue;
        }
        // A development run with hooks off leaves cleanup pending for a run allowed to write them.
        if env.hooks_off() {
            continue;
        }
        match cleanup_accounts(&bytes, env, data) {
            Ok((accounts, invalid)) => {
                if invalid {
                    log("Some old account folders could not be read for hook cleanup");
                }
                for a in accounts {
                    if let Err(e) = crate::hooks::sync(
                        a.provider,
                        &a.folder,
                        &a.id,
                        data,
                        &crate::hooks::Want::default(),
                    ) {
                        log(&format!(
                            "{}: could not remove old Warden hooks ({})",
                            a.label,
                            e.kind()
                        ));
                    }
                }
            }
            Err(e) => log(&format!(
                "Old settings could not be used for hook cleanup ({})",
                e.kind()
            )),
        }
        cleaned.push(name);
        save(data, receipt, &cleaned)?;
    }
    Ok(settings)
}

/// Loads a JSON file, or the default when it does not exist yet. A file that cannot be parsed is set aside as
/// `<name>.bad` so the user's data is never silently overwritten, and the default is used.
pub fn load<T: DeserializeOwned + Default>(data_dir: &Path, name: &str) -> io::Result<T> {
    let path = data_dir.join(name);
    match fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice(&bytes).and_then(|v| {
            if name == SETTINGS_FILE {
                let raw: serde_json::Value = serde_json::from_slice(&bytes)?;
                if raw.get("version").and_then(serde_json::Value::as_u64) != Some(2) {
                    return Err(<serde_json::Error as serde::de::Error>::custom(
                        "unsupported settings version",
                    ));
                }
            }
            Ok(v)
        }) {
            Ok(v) => Ok(v),
            Err(_) => {
                let mut backup = data_dir.join(format!("{name}.bad"));
                let mut suffix = 1u64;
                while fs::symlink_metadata(&backup).is_ok() {
                    backup = data_dir.join(format!("{name}.bad.{suffix}"));
                    suffix += 1;
                }
                fs::rename(&path, backup)?;
                Ok(T::default())
            }
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e),
    }
}

/// Saves a JSON file atomically.
pub fn save<T: Serialize>(data_dir: &Path, name: &str, value: &T) -> io::Result<()> {
    fs::create_dir_all(data_dir)?;
    let json = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    write_atomic(&data_dir.join(name), &json)
}

/// Writes a file atomically: to a temporary file of its own beside it (unique to this process and write, so
/// concurrent writers never share one), then moved over it.
///
/// The temporary file must be new: a file or link already under its name is never opened, so nothing planted there
/// can redirect the write. A link at `path` itself (a settings file kept elsewhere and linked in) is kept: the file
/// it points to is the one written.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let linked;
    let path = if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        linked = fs::canonicalize(path)?;
        linked.as_path()
    } else {
        path
    };
    let mut attempts = 0;
    let (tmp, mut file) = loop {
        let mut name = path.file_name().unwrap_or_default().to_owned();
        name.push(format!(
            ".{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let tmp = path.with_file_name(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(f) => break (tmp, f),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && attempts < 8 => attempts += 1,
            Err(e) => return Err(e),
        }
    };
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    written
        .and_then(|()| fs::rename(&tmp, path))
        .inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::Dir;

    fn account(id: &str, enabled: bool) -> AccountConfig {
        AccountConfig {
            id: id.into(),
            label: id.into(),
            provider: Provider::Claude,
            folder: PathBuf::from(id),
            enabled,
            identity: None,
        }
    }

    #[test]
    fn saves_and_loads() {
        let dir = Dir::new("data");
        let mut s = Settings::default();
        assert_eq!(
            load::<Settings>(dir.path(), SETTINGS_FILE).expect("loads"),
            s
        );
        s.accounts.push(account("a1", true));
        s.usage_interval_min = 10;
        save(dir.path(), SETTINGS_FILE, &s).expect("saves");
        assert_eq!(
            load::<Settings>(dir.path(), SETTINGS_FILE).expect("loads"),
            s
        );
    }

    #[test]
    fn a_broken_file_is_set_aside() {
        let dir = Dir::new("data");
        fs::write(dir.path().join(SETTINGS_FILE), "{ broken").expect("write");
        assert_eq!(
            load::<Settings>(dir.path(), SETTINGS_FILE).expect("loads"),
            Settings::default()
        );
        assert!(dir.path().join("settings.json.bad").is_file());
    }

    #[test]
    fn writes_replace_the_file_and_leave_no_temporary_files() {
        let dir = Dir::new("atomic");
        let file = dir.path().join("settings.json");
        write_atomic(&file, b"one").expect("first");
        write_atomic(&file, b"two").expect("second");
        assert_eq!(fs::read(&file).expect("read"), b"two");
        let names: Vec<_> = fs::read_dir(dir.path())
            .expect("list")
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, ["settings.json"]);
    }

    /// A settings file linked in from elsewhere stays a link; the file it points to gets the new text. Creating a
    /// link needs Windows' developer mode (or an elevated process): without it, there is nothing to test.
    #[cfg(windows)]
    #[test]
    fn a_linked_file_stays_linked() {
        let dir = Dir::new("atomic-link");
        let real = dir.path().join("kept-elsewhere.json");
        let link = dir.path().join("settings.json");
        fs::write(&real, "{}").expect("real");
        if std::os::windows::fs::symlink_file(&real, &link).is_err() {
            return;
        }
        write_atomic(&link, b"{\"a\":1}").expect("write");
        assert!(
            fs::symlink_metadata(&link)
                .expect("link")
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&real).expect("read"), b"{\"a\":1}");
    }

    #[test]
    fn reserve_and_interval_are_normalised_without_enabling_a_warden_account() {
        let mut s = Settings {
            reserve: 255,
            usage_interval_min: 7,
            accounts: vec![account("a1", false)],
            ..Settings::default()
        };
        s.normalise();
        assert_eq!(s.reserve, 50);
        assert_eq!(s.usage_interval_min, 5);
        assert!(!s.accounts[0].enabled);
        for reserve in 0..=50 {
            s.reserve = reserve;
            s.normalise();
            assert_eq!(s.reserve, reserve / 5 * 5);
        }
    }

    #[test]
    fn settings_v1_is_set_aside_without_migration() {
        let dir = Dir::new("settings-v1");
        let old = br#"{"version":1,"accounts":[{"id":"a1","label":"Old","provider":"claude","folder":"%USERPROFILE%/.claude-work","enabled":true,"identity":null}],"setupDone":true}"#;
        assert!(
            serde_json::from_slice::<Settings>(old).is_ok(),
            "the old document has a valid shape"
        );
        fs::write(dir.path().join(SETTINGS_FILE), old).expect("write");
        assert_eq!(
            load::<Settings>(dir.path(), SETTINGS_FILE).expect("load"),
            Settings::default()
        );
        assert_eq!(
            fs::read(dir.path().join("settings.json.bad")).expect("backup"),
            old
        );
    }

    #[test]
    fn main_is_a_small_record_and_its_folder_is_derived_from_the_environment() {
        let mut settings = Settings::default();
        settings.main.label = "My main".into();
        let dir = Dir::new("main-home");
        let env = crate::detect::Env {
            home: Some(dir.path().to_owned()),
            ..crate::detect::Env::default()
        };
        let main = settings.main_account(&env).expect("main");
        assert_eq!(main.id, MAIN_ID);
        assert_eq!(main.folder, dir.path().join(".claude"));
        assert!(main.enabled);
        assert!(settings.accounts.is_empty());
        let json = serde_json::to_value(&settings).expect("JSON");
        assert_eq!(json["version"], 2);
        assert_eq!(json["main"]["label"], "My main");
        assert_eq!(json["main"].as_object().expect("record").len(), 2);
        assert!(json.get("routing").is_none());
    }

    #[test]
    fn ids_are_not_reused() {
        let mut s = Settings::default();
        assert_eq!(s.new_id(), "a1");
        assert_eq!(s.new_id(), "a2");
    }
    #[test]
    fn unsupported_settings_never_replace_an_earlier_set_aside_file() {
        let dir = crate::testdir::Dir::new("future-settings");
        for (version, backup) in [(3, "settings.json.bad"), (4, "settings.json.bad.1")] {
            let bytes = format!("{{\"version\":{version}}}");
            fs::write(dir.path().join(SETTINGS_FILE), &bytes).expect("future settings");
            let settings: Settings = load(dir.path(), SETTINGS_FILE).expect("set aside");
            assert!(!settings.setup_done);
            assert_eq!(
                fs::read_to_string(dir.path().join(backup)).expect("preserved"),
                bytes
            );
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("settings.json.bad")).expect("first preserved"),
            "{\"version\":3}"
        );
    }

    #[test]
    fn work_hours_round_trip_default_to_none_and_never_break_the_file() {
        let mut s = Settings {
            work_hours: Some(WorkHours::first()),
            ..Settings::default()
        };
        let json = serde_json::to_value(&s).expect("json");
        assert_eq!(
            json["workHours"],
            serde_json::json!({"days": [1, 2, 3, 4, 5], "from": 540, "to": 1080})
        );
        let back: Settings = serde_json::from_value(json).expect("back");
        assert_eq!(back.work_hours, s.work_hours);
        // A version-2 file without it: every hour counts.
        let old: Settings = serde_json::from_str(r#"{"version":2,"reserve":25}"#).expect("old");
        assert_eq!(old.work_hours, None);
        // The wrong shape loads as not set instead of failing the whole file.
        let odd: Settings =
            serde_json::from_str(r#"{"version":2,"workHours":{"days":"weekdays"}}"#).expect("odd");
        assert_eq!(odd.work_hours, None);
        // A value of the right shape but out of bounds is dropped by normalise.
        for bad in [
            WorkHours {
                days: vec![],
                from: 540,
                to: 1080,
            },
            WorkHours {
                days: vec![2, 1],
                from: 540,
                to: 1080,
            },
            WorkHours {
                days: vec![1, 1],
                from: 540,
                to: 1080,
            },
            WorkHours {
                days: vec![8],
                from: 540,
                to: 1080,
            },
            WorkHours {
                days: vec![1],
                from: 545,
                to: 1080,
            },
            WorkHours {
                days: vec![1],
                from: 1080,
                to: 540,
            },
            WorkHours {
                days: vec![1],
                from: 540,
                to: 1470,
            },
        ] {
            s.work_hours = Some(bad);
            s.normalise();
            assert_eq!(s.work_hours, None);
        }
        s.work_hours = Some(WorkHours {
            days: vec![7],
            from: 0,
            to: 1440,
        });
        s.normalise();
        assert!(s.work_hours.is_some());
    }

    #[test]
    fn the_reset_warning_defaults_to_three_days_and_never_breaks_the_file() {
        assert_eq!(Settings::default().reset_warning_days, 3);
        let s = Settings {
            reset_warning_days: 14,
            ..Settings::default()
        };
        let json = serde_json::to_value(&s).expect("json");
        assert_eq!(json["resetWarningDays"], 14);
        let back: Settings = serde_json::from_value(json).expect("back");
        assert_eq!(back.reset_warning_days, 14);
        // Missing, of the wrong type, fractional or out of range: the default, and the rest of the file loads.
        for raw in [
            r#"{"version":2,"reserve":30}"#,
            r#"{"version":2,"reserve":30,"resetWarningDays":null}"#,
            r#"{"version":2,"reserve":30,"resetWarningDays":"5"}"#,
            r#"{"version":2,"reserve":30,"resetWarningDays":2.5}"#,
            r#"{"version":2,"reserve":30,"resetWarningDays":0}"#,
            r#"{"version":2,"reserve":30,"resetWarningDays":15}"#,
            r#"{"version":2,"reserve":30,"resetWarningDays":-1}"#,
            r#"{"version":2,"reserve":30,"resetWarningDays":300}"#,
        ] {
            let s: Settings = serde_json::from_str(raw).expect(raw);
            assert_eq!((s.reset_warning_days, s.reserve), (3, 30), "{raw}");
        }
        for days in 1..=14 {
            let s: Settings =
                serde_json::from_str(&format!(r#"{{"version":2,"resetWarningDays":{days}}}"#))
                    .expect("loads");
            assert_eq!(s.reset_warning_days, days);
        }
        let mut s = Settings {
            reset_warning_days: 40,
            ..Settings::default()
        };
        s.normalise();
        assert_eq!(s.reset_warning_days, 3);
    }

    #[test]
    fn a_usage_reading_brings_its_limit_resets_and_one_without_keeps_the_last() {
        use crate::codex_resets::Reset;
        let usage = |read_at: i64, resets: Option<Vec<Reset>>| Usage {
            read_at,
            five_hour: None,
            weekly: None,
            per_model_weekly: Vec::new(),
            extra: None,
            resets,
        };
        let one = vec![Reset {
            title: Some("Full reset".into()),
            expires_at: Some(5),
        }];
        let mut r = Readings::default();
        assert_eq!(
            r.apply_usage("a", Ok(usage(1, Some(one.clone())))),
            UsageUpdate::Read
        );
        assert_eq!(r.accounts["a"].resets, one);
        // The reading kept carries none of them.
        assert_eq!(r.accounts["a"].usage.as_ref().expect("usage").resets, None);
        // No readable list, a failed read, an older reading: the last list stays.
        r.apply_usage("a", Ok(usage(2, None)));
        r.apply_usage("a", Err(UsageError::LoginExpired { status: Some(401) }));
        r.apply_usage("a", Err(UsageError::IdentityChanged));
        r.apply_usage("a", Ok(usage(0, Some(Vec::new()))));
        assert_eq!(r.accounts["a"].resets, one);
        // An empty list (none left, or not eligible) replaces it.
        r.apply_usage("a", Ok(usage(3, Some(Vec::new()))));
        assert!(r.accounts["a"].resets.is_empty());
    }
}
