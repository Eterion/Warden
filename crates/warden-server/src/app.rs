//! The server's state and everything it does with accounts: reading usage, checking identity, changing settings.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::Duration;
use warden_core::claude::{self, UsageEndpoint};
use warden_core::codex::{self, LogTail};
use warden_core::codex_resets::{self, Reset, ResetsEndpoint};
use warden_core::detect::{self, Env, Tool};
use warden_core::fallback;
use warden_core::hooks::{self, HookTool, Setup, Setups, Want};
use warden_core::logfile::{self, RollingLog};
use warden_core::mainlogin::{self, SignInServer};
use warden_core::model::{Identity, IdentityChange, Provider, Usage, UsageError};
use warden_core::pipe::Message;
use warden_core::routing;
use warden_core::secret::Secret;
use warden_core::sessions::{self, Activity, Event, EventKind, Session, Sessions};
use warden_core::store::{
    self, AccountConfig, AccountReading, IdentityUpdate, READINGS_FILE, Readings, SETTINGS_FILE,
    Settings, Theme, UsageUpdate, WorkHours,
};
use warden_core::time::now_ms;
use warden_core::titles;

/// Writes one line to the server's log (and, through `App::new`, to the data folder's rolling log file). Never given a
/// token, prompt, tool input or message.
pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

/// Starts the official sign-in for an account folder (in the real server, `fallback::start_sign_in`), and calls the
/// last argument once the sign-in's CLI has exited.
pub type SignIn =
    Arc<dyn Fn(Provider, &Path, &Path, fallback::Done) -> io::Result<()> + Send + Sync>;

/// An error answered to the UI: an HTTP status and a plain sentence.
#[derive(Debug)]
pub struct ApiError(pub u16, pub String);

impl ApiError {
    pub fn bad(msg: impl Into<String>) -> Self {
        ApiError(400, msg.into())
    }
    pub fn not_found() -> Self {
        ApiError(404, "There is no such account.".into())
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// What only the desktop app can do. The demo server has none, and its settings are refused there.
pub trait Desktop: Send + Sync {
    /// Whether Warden is registered to start when the user signs in to Windows.
    fn start_with_windows(&self) -> bool;
    /// Registers or removes the start with Windows.
    fn set_start_with_windows(&self, on: bool) -> io::Result<()>;
    /// Opens a folder in Explorer ("Open log folder").
    fn open_folder(&self, folder: &Path) -> io::Result<()>;
    /// Installs the downloaded update and starts Warden again ("Install and restart"); Warden closes for it.
    fn install_update(&self) -> io::Result<()>;
}

/// A newer Warden the desktop app found: downloading (`downloaded` the percent done, `None` before it starts or
/// while the size is not known) or `ready` to install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    pub version: String,
    pub downloaded: Option<u8>,
    pub ready: bool,
}

/// A change to the settings; fields left out stay as they are.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    pub reserve: Option<u8>,
    pub usage_interval_min: Option<u32>,
    pub theme: Option<Theme>,
    /// Desktop app only.
    pub keep_in_tray: Option<bool>,
    /// Desktop app only.
    pub start_with_windows: Option<bool>,
    /// The work hours: an object sets them, `null` clears them (every hour counts), absent leaves them.
    #[serde(default, deserialize_with = "present")]
    pub work_hours: Option<serde_json::Value>,
    /// How many days before a limit reset expires the Overview warns of it.
    pub reset_warning_days: Option<u8>,
    /// Whether the Overview's sessions table shows ended and not-known sessions. `null` is refused, as any other
    /// value that is not a boolean.
    #[serde(default, deserialize_with = "strict_bool")]
    pub show_quiet_sessions: Option<bool>,
    /// The accounts whose limit resets the Overview's list leaves out: ids of accounts Warden knows (main is `main`).
    /// `null` is refused, as anything but a list of strings.
    #[serde(default, deserialize_with = "strict_ids")]
    pub hidden_reset_accounts: Option<Vec<String>>,
    /// Whether the Overview's limit resets list shows only each account's soonest reset. `null` is refused, as any
    /// other value that is not a boolean.
    #[serde(default, deserialize_with = "strict_bool")]
    pub soonest_reset_only: Option<bool>,
}

/// A list of ids that, when there, must be a list of strings; absent leaves the setting.
fn strict_ids<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    Vec::<String>::deserialize(d).map(Some)
}

/// A boolean that, when there, must be `true` or `false`; absent leaves the setting.
fn strict_bool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    bool::deserialize(d).map(Some)
}

/// Whether `id` names an account Warden knows: main (`main`) or one of its own.
fn known_account(s: &store::Settings, id: &str) -> bool {
    id == "main" || s.accounts.iter().any(|a| a.id == id)
}

/// A field that is there, even as `null`, is `Some`: so `null` can clear a setting while absent leaves it.
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<serde_json::Value>, D::Error> {
    serde_json::Value::deserialize(d).map(Some)
}

const WORK_HOURS_REFUSED: &str =
    "Work hours need at least one day, and an end after their start, in steps of 30 minutes.";

/// An account whose folder is signed in as someone else, for the desktop app's notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityAlert {
    pub account_id: String,
    pub label: String,
    /// The email the folder is signed in as now; `None` when signed out.
    pub now: Option<String>,
}

struct Inner {
    settings: Settings,
    readings: Readings,
    tools: Vec<Tool>,
    main_signed_in: bool,
    last_read: Option<i64>,
    /// Bumped on every change the UI shows, so a waiting UI request can answer.
    revision: u64,
    /// Bumped whenever the set of Codex folders changes, so the watcher starts again.
    codex_generation: u64,
    /// Bumped whenever the set of account folders changes, so the hook file watcher starts again.
    folders_generation: u64,
    /// Where each account's hooks stand, read from its files only when they can have changed: after Warden writes
    /// them, and when the hook file watcher sees one change. Every state answer takes it from here.
    hook_setups: HashMap<String, Setups>,
    /// The shell Claude Code runs each Claude account's commands in, read with its hook setup: the preview tells
    /// main's commands for it.
    claude_shells: HashMap<String, hooks::ClaudeShell>,
    /// The accounts part of the state, built once per change to the accounts (`touch`); a change to the sessions
    /// alone (`touch_sessions`) reuses it.
    accounts_view: Option<Arc<Vec<AccountView>>>,
    /// Live sessions and two hours of activity per account, from the status hook. In memory only, capped; only which
    /// main session started each delegated run is kept on disk (`session-parents.json`).
    sessions: Sessions,
    /// Accounts whose status hook has reported since Warden started; the others get the session-list fallback.
    hooked: HashSet<String>,
    /// The sessions' CLI processes Warden waits on for their exit.
    watched: HashSet<u32>,
    /// Whether Warden starts with Windows (desktop app only): read from the registry at start and after each change
    /// through the settings, not on every state answer.
    start_with_windows: Option<bool>,
    /// A newer Warden the desktop app found, as its updater reports it.
    update: Option<UpdateView>,
    /// A usage reading was asked for (an account added, the read interval changed) and the usage loop has not started it
    /// yet. Kept as a flag, so a wake-up while a reading runs is not lost.
    usage_wanted: bool,
    /// The usage loop is reading, between two waits.
    usage_busy: bool,
    /// Why each Codex account's banked resets could not be read last time, so a failure is logged once.
    resets_failed: HashMap<String, String>,
    /// When each account's limit resets (Claude's, or Codex's banked resets) were last read with success, since Warden
    /// started: a reading on the usage interval asks for them again only once they are due
    /// ([`codex_resets::resets_due`]).
    resets_read: HashMap<String, i64>,
    stop: bool,
}

pub struct App {
    inner: Mutex<Inner>,
    /// Wakes the usage loop (refresh, interval change, stop).
    wake: Condvar,
    /// Wakes UI requests waiting for a change.
    changed: Condvar,
    /// One usage reading at a time.
    reading: Mutex<()>,
    /// The app itself once shared ([`App::shared`]), for work that finishes later (a sign-in's window closing).
    me: std::sync::OnceLock<Weak<App>>,
    /// Mutations and hook writes for an account share this lock, through their read-back and removal.
    hook_changes: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    pub data_dir: PathBuf,
    pub env: Env,
    endpoint: Arc<dyn UsageEndpoint>,
    sign_in_server: Arc<dyn SignInServer>,
    resets: Arc<dyn ResetsEndpoint>,
    sign_in: SignIn,
    pub log: Log,
    /// The `warden` binary the hooks run; `None` when it was not found.
    route_exe: Option<PathBuf>,
    /// Held while the server runs, so `warden refresh` leaves the readings to it.
    _lock: store::ServerLock,
    /// Wakes the Codex watcher when the Codex accounts change or the server stops.
    codex_wake: Mutex<Option<CodexSender>>,
    /// Wakes the hook file watcher when the accounts change or the server stops.
    hooks_wake: Mutex<Option<CodexSender>>,
    /// One reading of hook files at a time, so a reading made before a change never replaces one made after it.
    hook_reading: Mutex<()>,
    /// One change to Start with Windows at a time: the registry write, its read-back and the kept answer together, so
    /// an earlier read-back never replaces a later one.
    start_with_windows_change: Mutex<()>,
    /// Where each Claude session's transcript was last read for its title.
    transcripts: Mutex<titles::Transcripts>,
    desktop: Option<Arc<dyn Desktop>>,
}

type CodexSender = std::sync::mpsc::Sender<notify::Result<notify::Event>>;

/// Wakes a watcher waiting on its channel.
fn poke(wake: &Mutex<Option<CodexSender>>) {
    if let Some(tx) = wake
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
    {
        let _ = tx.send(Err(notify::Error::generic("accounts changed")));
    }
}

/// The UI's view of an identity: no ids.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityView {
    email: String,
    organization: Option<String>,
}

impl From<&Identity> for IdentityView {
    fn from(i: &Identity) -> Self {
        IdentityView {
            email: i.email.clone(),
            organization: i.organization.clone(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityChangeView {
    now: Option<IdentityView>,
    known_account_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    id: String,
    label: String,
    provider: Provider,
    folder: String,
    enabled: bool,
    identity: Option<IdentityView>,
    identity_change: Option<IdentityChangeView>,
    usage: Option<Usage>,
    usage_error: Option<UsageError>,
    /// The account's limit resets (Claude's, or Codex's banked resets), soonest to expire first.
    resets: Vec<Reset>,
    hooks: Setup,
    same_as_main: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    reserve: u8,
    usage_interval_min: u32,
    theme: Theme,
    keep_in_tray: bool,
    work_hours: Option<WorkHours>,
    reset_warning_days: u8,
    show_quiet_sessions: bool,
    /// Only ids of accounts Warden knows now: one removed since is left out.
    hidden_reset_accounts: Vec<String>,
    soonest_reset_only: bool,
}

/// Present only in the desktop app.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopView {
    start_with_windows: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateView {
    hooks_off: bool,
    version: &'static str,
    setup_done: bool,
    data_dir: String,
    log_dir: String,
    settings: SettingsView,
    main: Option<MainView>,
    accounts: Shared<Vec<AccountView>>,
    problems: Vec<Problem>,
    last_read: Option<i64>,
    revision: u64,
    home_dir: Option<String>,
    delegation: DelegationView,
    /// The `warden` binary the hooks run; `null` when it was not found.
    route_tool: Option<String>,
    /// Sessions to show, in no order (the UI sorts them).
    sessions: Vec<Session>,
    activity: Vec<ActivityView>,
    desktop: Option<DesktopView>,
    /// A newer Warden, found by the desktop app; `null` when there is nothing newer and in the demo server.
    update: Option<UpdateView>,
}

#[derive(Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Problem {
    ClaudeAppMissing,
    ClaudeAppSignedOut,
    CliMissing {
        provider: Provider,
        name: String,
    },
    CliTooOld {
        provider: Provider,
        name: String,
        version: String,
        needed: &'static str,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegatedView {
    account_id: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationView {
    main_works_itself: bool,
    delegated: Option<DelegatedView>,
    main_reason: String,
    delegated_reason: String,
    hand_off_at: u8,
}
impl From<routing::Decision> for DelegationView {
    fn from(d: routing::Decision) -> Self {
        Self {
            main_works_itself: d.main_works_itself,
            delegated: d.delegated.map(|t| DelegatedView {
                account_id: t.account_id,
            }),
            main_reason: d.main_reason,
            delegated_reason: d.delegated_reason,
            hand_off_at: d.hand_off_at,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MainView {
    #[serde(flatten)]
    account: AccountView,
    sign_in: mainlogin::SignIn,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityView {
    account_id: String,
    bins: Activity,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    provider: Provider,
    folder: String,
    label: String,
    identity: Option<IdentityView>,
    signed_in: bool,
    known: bool,
}

/// A part of the state built once and shared by every answer until it changes.
pub struct Shared<T>(Arc<T>);

impl<T: Serialize> Serialize for Shared<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

fn show(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

impl App {
    pub fn new(
        data_dir: PathBuf,
        env: Env,
        (endpoint, sign_in_server, resets): (
            Arc<dyn UsageEndpoint>,
            Arc<dyn SignInServer>,
            Arc<dyn ResetsEndpoint>,
        ),
        sign_in: SignIn,
        log: Log,
        route_exe: Option<PathBuf>,
        desktop: Option<Arc<dyn Desktop>>,
    ) -> io::Result<App> {
        std::fs::create_dir_all(&data_dir)?;
        let lock = wait_for_lock(&data_dir)?;
        // Every line also goes to the rolling log file, written only while this server holds the data folder.
        let file = RollingLog::new(logfile::log_dir(&data_dir));
        let log: Log = Arc::new(move |line: &str| {
            log(line);
            file.write(line);
        });
        log(&format!("Warden {} started", env!("CARGO_PKG_VERSION")));
        let mut settings = store::load_settings(&data_dir, &env, |line| log(line))?;
        settings.normalise();
        settings.retain_warden_folders(&env, &data_dir);
        let mut readings: Readings = store::load(&data_dir, READINGS_FILE)?;
        if !settings.setup_done && settings.accounts.is_empty() {
            // Old readings do not belong to the new account namespace. Keep the own sign-in's refusal latch.
            readings.accounts.clear();
            readings.checked_at = None;
        }
        let main_signed_in = settings.main_account(&env).is_some_and(|a| {
            claude::read_identity(&a.folder, env.home.as_deref())
                .ok()
                .flatten()
                .is_some()
        });
        let tools = detect::tools(&env);
        let app = App {
            inner: Mutex::new(Inner {
                settings,
                readings,
                tools,
                main_signed_in,
                last_read: None,
                codex_generation: 0,
                folders_generation: 0,
                hook_setups: HashMap::new(),
                claude_shells: HashMap::new(),
                accounts_view: None,
                sessions: {
                    let mut s = Sessions::default();
                    s.restore_parents(
                        store::load(&data_dir, sessions::PARENT_LINKS_FILE).unwrap_or_default(),
                        now_ms(),
                    );
                    s
                },
                hooked: HashSet::new(),
                watched: HashSet::new(),
                start_with_windows: desktop.as_ref().map(|d| d.start_with_windows()),
                update: None,
                usage_wanted: false,
                // The loop starts with a reading.
                usage_busy: true,
                resets_failed: HashMap::new(),
                resets_read: HashMap::new(),
                revision: 0,
                stop: false,
            }),
            wake: Condvar::new(),
            changed: Condvar::new(),
            reading: Mutex::new(()),
            me: std::sync::OnceLock::new(),
            hook_changes: Mutex::new(HashMap::new()),
            data_dir,
            env,
            endpoint,
            sign_in_server,
            resets,
            sign_in,
            log,
            route_exe,
            _lock: lock,
            codex_wake: Mutex::new(None),
            hooks_wake: Mutex::new(None),
            hook_reading: Mutex::new(()),
            start_with_windows_change: Mutex::new(()),
            transcripts: Mutex::new(titles::Transcripts::default()),
            desktop,
        };
        // The sessions the CLIs' own lists hold, before the hooks' pipe opens and before the first usage reading: the
        // first answer already has every session Warden can know about at start, each under its main session again
        // (the saved parent links). No hook has reported yet, so every account is listed; a folder signed in as
        // someone else counts as the last reading found until the first reading checks it again.
        app.list_sessions();
        // A main session idle for longer than the lists' window is not in them: each one a shown run's saved link
        // names is looked up by its id, so the run sits under it.
        let to_find = app.lock().sessions.parents_to_find(now_ms());
        app.find_parents(to_find);
        Ok(app)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Marks a change the UI shows and wakes requests waiting for one.
    fn touch(&self, inner: &mut Inner) {
        inner.accounts_view = None;
        self.touch_sessions(inner);
    }

    /// Marks a change to the sessions alone (the status hook's events, the listed sessions): the accounts part of
    /// the state stays as it was built.
    fn touch_sessions(&self, inner: &mut Inner) {
        inner.revision += 1;
        self.changed.notify_all();
    }

    /// The state once it differs from `since`, or after `timeout` either way (a long poll: the UI asks again).
    pub fn state_after(&self, since: u64, timeout: Duration) -> StateView {
        let inner = self.lock();
        let _ = self
            .changed
            .wait_timeout_while(inner, timeout, |i| i.revision == since && !i.stop)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.state()
    }

    fn save_settings(&self, inner: &mut Inner) -> ApiResult<()> {
        self.touch(inner);
        store::save(&self.data_dir, SETTINGS_FILE, &inner.settings).map_err(|e| {
            (self.log)(&format!("could not save settings: {e}"));
            ApiError(500, "Warden could not save its settings.".into())
        })
    }

    fn save_readings(&self, inner: &mut Inner) {
        self.touch(inner);
        if let Err(e) = store::save(&self.data_dir, READINGS_FILE, &inner.readings) {
            (self.log)(&format!("could not save readings: {e}"));
        }
    }

    pub fn state(&self) -> StateView {
        // Hook setups not read yet (an account just added, or the first answer) are read first, outside the lock.
        let missing: Vec<String> = {
            let inner = self.lock();
            inner
                .settings
                .all_accounts(&self.env)
                .into_iter()
                .filter(|a| !inner.hook_setups.contains_key(&a.id))
                .map(|a| a.id.clone())
                .collect()
        };
        for id in missing {
            self.refresh_hook_setup(&id);
        }
        let mut inner = self.lock();
        let accounts = match &inner.accounts_view {
            Some(v) => Arc::clone(v),
            None => {
                let v = Arc::new(self.accounts_view(&inner));
                inner.accounts_view = Some(Arc::clone(&v));
                v
            }
        };
        let s = &inner.settings;
        StateView {
            hooks_off: self.env.hooks_off(),
            version: env!("CARGO_PKG_VERSION"),
            setup_done: s.setup_done,
            data_dir: show(&self.data_dir),
            log_dir: show(&logfile::log_dir(&self.data_dir)),
            settings: SettingsView {
                reserve: s.reserve,
                usage_interval_min: s.usage_interval_min,
                theme: s.theme,
                keep_in_tray: s.keep_in_tray,
                work_hours: s.work_hours.clone(),
                reset_warning_days: s.reset_warning_days,
                show_quiet_sessions: s.show_quiet_sessions,
                hidden_reset_accounts: s
                    .hidden_reset_accounts
                    .iter()
                    .filter(|id| known_account(s, id))
                    .cloned()
                    .collect(),
                soonest_reset_only: s.soonest_reset_only,
            },
            main: s.main_account(&self.env).as_ref().map(|a| MainView {
                account: self.account_view(&inner, a),
                sign_in: inner.readings.main_login.sign_in.clone(),
            }),
            accounts: Shared(accounts),
            problems: self.problems(&inner),
            last_read: inner.last_read,
            revision: inner.revision,
            home_dir: self.env.home.as_deref().map(show),
            delegation: routing::decide(s, &inner.readings, now_ms()).into(),
            route_tool: self.route_exe.as_deref().map(show),
            sessions: inner.sessions.shown_with_parents(now_ms()),
            activity: inner
                .sessions
                .activity()
                .iter()
                .map(|(id, a)| ActivityView {
                    account_id: id.clone(),
                    bins: a.clone(),
                })
                .collect(),
            desktop: inner.start_with_windows.map(|on| DesktopView {
                start_with_windows: on,
            }),
            update: inner.update.clone(),
        }
    }

    fn problems(&self, inner: &Inner) -> Vec<Problem> {
        let mut out = Vec::new();
        if !inner.tools.iter().any(|t| {
            t.provider == Provider::Claude && t.kind == detect::ToolKind::App && t.path.is_some()
        }) {
            out.push(Problem::ClaudeAppMissing);
        }
        if !inner.main_signed_in {
            out.push(Problem::ClaudeAppSignedOut);
        }
        for provider in [Provider::Claude, Provider::Codex] {
            let tool = inner
                .tools
                .iter()
                .find(|t| t.provider == provider && t.kind == detect::ToolKind::Cli);
            if tool.is_none_or(|t| t.path.is_none()) {
                out.push(Problem::CliMissing {
                    provider,
                    name: provider.name().to_owned(),
                });
            }
        }
        if let Some(version) = claude_too_old(&inner.tools) {
            out.push(Problem::CliTooOld {
                provider: Provider::Claude,
                name: "Claude".into(),
                version,
                needed: hooks::MIN_CLAUDE_VERSION,
            });
        }
        out
    }

    /// The accounts as the UI shows them.
    fn accounts_view(&self, inner: &Inner) -> Vec<AccountView> {
        inner
            .settings
            .accounts
            .iter()
            .map(|a| self.account_view(inner, a))
            .collect()
    }
    fn account_view(&self, inner: &Inner, a: &AccountConfig) -> AccountView {
        let r = inner.readings.accounts.get(&a.id);
        let setup = inner
            .hook_setups
            .get(&a.id)
            .copied()
            .unwrap_or_else(|| hooks::setup(a.provider, &a.folder, &a.id, &self.ours()));
        let main = a.id == store::MAIN_ID;
        AccountView {
            id: a.id.clone(),
            label: a.label.clone(),
            provider: a.provider,
            folder: show(&a.folder),
            enabled: a.enabled,
            identity: a.identity.as_ref().map(Into::into),
            identity_change: r.and_then(|r| r.identity_change.as_ref()).map(|c| {
                IdentityChangeView {
                    now: c.now.as_ref().map(Into::into),
                    known_account_id: c.known_account_id.clone(),
                }
            }),
            usage: r.and_then(|r| r.usage.clone()),
            usage_error: r.and_then(|r| r.error.clone()),
            // Resets read while signed in as the account: not shown while the folder is someone else's.
            resets: r
                .filter(|r| r.identity_change.is_none())
                .map(|r| r.resets.clone())
                .unwrap_or_default(),
            same_as_main: !main && routing::same_as_main(&inner.settings, a),
            hooks: if main && setup.routing != Setup::Installed {
                setup.routing
            } else {
                setup.status
            },
        }
    }

    /// What this Warden writes as an account's hooks now: a hook running another warden.exe or data folder, or in
    /// another form, is not in place.
    fn ours(&self) -> Want {
        Want {
            tool: self.hook_tool(),
            route: true,
            status: true,
        }
    }

    /// What the hooks run: this Warden's `warden.exe` with its data folder; `None` when `warden.exe` was not found.
    fn hook_tool(&self) -> Option<HookTool> {
        self.route_exe.clone().map(|exe| HookTool {
            exe,
            data_dir: self.data_dir.clone(),
            #[cfg(debug_assertions)]
            home: self.env.home.clone(),
        })
    }

    /// Reads where an account's hooks stand from its files and keeps it; marks a change the UI shows.
    pub fn refresh_hook_setup(&self, id: &str) {
        let _one = self
            .hook_reading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some((provider, folder)) = self
            .lock()
            .settings
            .account_in(&self.env, id)
            .map(|a| (a.provider, a.folder.clone()))
        else {
            return;
        };
        let shell = self.claude_shell(provider, &folder);
        let setup = hooks::setup(provider, &folder, id, &self.ours());
        let mut inner = self.lock();
        if inner.settings.account_in(&self.env, id).is_none() {
            return;
        }
        let shell_changed = inner
            .claude_shells
            .insert(id.to_owned(), shell)
            .is_some_and(|was| was != shell);
        match inner.hook_setups.insert(id.to_owned(), setup) {
            Some(was) if was == setup && !shell_changed => {}
            Some(_) => self.touch(&mut inner),
            // Read for the first time: no answer showed it yet, so only the accounts part is built again.
            None => inner.accounts_view = None,
        }
    }

    /// Waits until anything the UI shows changes, and returns the new revision; `None` once the server stops.
    /// The desktop app follows the state with it, without polling.
    pub fn wait_for_change(&self, since: u64) -> Option<u64> {
        let inner = self.lock();
        let inner = self
            .changed
            .wait_while(inner, |i| i.revision == since && !i.stop)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (!inner.stop).then_some(inner.revision)
    }

    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    pub fn keep_in_tray(&self) -> bool {
        self.lock().settings.keep_in_tray
    }

    /// The accounts whose folder is signed in as someone else right now.
    pub fn identity_alerts(&self) -> Vec<IdentityAlert> {
        let inner = self.lock();
        inner
            .settings
            .all_accounts(&self.env)
            .into_iter()
            .filter_map(|a| {
                let change = inner
                    .readings
                    .accounts
                    .get(&a.id)?
                    .identity_change
                    .as_ref()?;
                Some(IdentityAlert {
                    account_id: a.id.clone(),
                    label: a.label.clone(),
                    now: change.now.as_ref().map(|i| i.email.clone()),
                })
            })
            .collect()
    }

    // ---- Usage and identity -------------------------------------------------------------------------------

    /// The usage reading on its interval: every Claude account's usage now, checking identity first, and each account's
    /// limit resets (Claude's with the usage, Codex's banked resets) only where they are due
    /// ([`codex_resets::resets_due`]). Network calls happen without the lock.
    pub fn read_claude_usage(&self) {
        self.read_usage_and_resets(false);
    }

    /// The usage refreshed by hand (`POST /api/usage/refresh`): as [`App::read_claude_usage`], asking every account for
    /// its limit resets.
    pub fn refresh_usage(&self) {
        self.read_usage_and_resets(true);
    }

    /// Whether this reading asks the account for its limit resets: always when refreshed by hand, otherwise when due.
    fn resets_wanted(&self, id: &str, by_hand: bool) -> bool {
        by_hand || codex_resets::resets_due(self.lock().resets_read.get(id).copied(), now_ms())
    }

    fn read_usage_and_resets(&self, by_hand: bool) {
        let _one = self
            .reading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.read_main_usage(by_hand);
        let accounts: Vec<AccountConfig> = self.lock().settings.accounts.clone();
        for account in accounts.iter() {
            self.read_account_usage(account, by_hand);
        }
        let mut inner = self.lock();
        let now = now_ms();
        inner.last_read = Some(now);
        // Tells `warden route` the readings are current, so it does not refresh them itself.
        inner.readings.checked_at = Some(now);
        self.save_readings(&mut inner);
    }

    /// Main's usage (and its limit resets when wanted) with Warden's own sign-in, checking who `~/.claude` is signed in
    /// as first. The caller holds `reading`: only one renewal of the sign-in at a time.
    fn read_main_usage(&self, by_hand: bool) {
        let main = self.lock().settings.main_account(&self.env);
        let expected = main.and_then(|main| {
            let observed = claude::read_identity(&main.folder, self.env.home.as_deref());
            let actual = observed.as_ref().ok().cloned().flatten();
            self.apply_identity(store::MAIN_ID, observed);
            actual
        });
        let mut login = self.lock().readings.main_login.clone();
        let main_changed = self.lock().readings.identity_changed(store::MAIN_ID);
        let result = mainlogin::read_for_main(
            &self._lock,
            expected.as_ref(),
            main_changed,
            &mut login,
            (
                self.endpoint.as_ref(),
                self.resets_wanted(store::MAIN_ID, by_hand),
            ),
            self.sign_in_server.as_ref(),
            now_ms(),
        );
        for line in login.take_log_lines() {
            (self.log)(&line);
        }
        self.lock().readings.main_login = login;
        self.apply_usage(store::MAIN_ID, result);
    }

    /// A Warden account's usage and limit resets: a Claude account's usage (with its resets when wanted), checking who
    /// the folder is signed in as first; a Codex account's sign-in and, when wanted, its banked resets (its usage comes
    /// from its logs).
    fn read_account_usage(&self, account: &AccountConfig, by_hand: bool) {
        if account.provider == Provider::Claude {
            let now = claude::read_identity(&account.folder, self.env.home.as_deref());
            // Who the folder is signed in as could not be read: its login might be someone else's, so the
            // last reading stays until the identity can be checked again.
            let unchecked = now.is_err() && account.identity.is_some();
            let changed = self.apply_identity(&account.id, now);
            let result = if changed {
                Err(UsageError::IdentityChanged)
            } else if unchecked {
                Err(UsageError::Failed {
                    message: "could not check who the folder is signed in as".into(),
                })
            } else {
                claude::read_usage(
                    &account.folder,
                    self.endpoint.as_ref(),
                    now_ms(),
                    self.resets_wanted(&account.id, by_hand),
                )
            };
            self.apply_usage(&account.id, result);
        } else {
            // One reading of the login file for who it is signed in as and the resets request's access, so the
            // token sent is the one of the identity checked here.
            let (now, access) = match codex::read_login_with_access(&account.folder) {
                Ok(login) => (Ok(login.identity), login.access),
                Err(e) => (Err(e), None),
            };
            let signed_in = matches!(now, Ok(Some(_)));
            self.apply_identity(&account.id, now);
            // Only while the folder is signed in as the account itself (a switched-off account is not shown), and
            // only when they are due or the usage is refreshed by hand.
            if account.enabled
                && signed_in
                && self.resets_wanted(&account.id, by_hand)
                && let Some(access) = access
            {
                self.read_codex_resets(&account.id, access);
            }
        }
    }

    // ---- Limit resets ---------------------------------------------------------------------------------------

    /// Uses one of the account's limit resets after the user confirmed it in the app (`POST
    /// /api/accounts/{id}/resets/use`): only a reset on its list now, and only while its folder is signed in as the
    /// account itself. Main's goes with Warden's own sign-in (renewed as for a usage reading), a Warden account's with
    /// the login in its own folder (never renewed). Once the provider says it reset the limits, the reset leaves the
    /// list at once and the account's usage and resets are read again, as a refresh by hand reads them. Logged without
    /// a token; the provider's refusal is answered `409` in plain words.
    pub fn use_reset(&self, id: &str, reset_id: &str) -> ApiResult<()> {
        // One at a time with the usage readings: Warden's own sign-in is renewed by one request at a time.
        let _one = self
            .reading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (account, title) = {
            let inner = self.lock();
            let account = inner
                .settings
                .account_in(&self.env, id)
                .ok_or_else(ApiError::not_found)?;
            if self.env.in_codex_app(&account.folder) {
                return Err(ApiError::bad("Warden never uses the Codex app's folder."));
            }
            if inner.readings.identity_changed(id) {
                return Err(signed_in_as_someone_else());
            }
            let reset = inner
                .readings
                .accounts
                .get(id)
                .and_then(|r| r.resets.iter().find(|r| r.id.as_deref() == Some(reset_id)))
                .ok_or_else(|| {
                    ApiError(409, "This reset is no longer on the account's list.".into())
                })?;
            // Checked against the clock now: a confirmation left open across its expiry sends nothing.
            if reset.expires_at.is_some_and(|at| at <= now_ms()) {
                return Err(ApiError(409, "This reset has expired.".into()));
            }
            (
                account,
                reset.title.clone().unwrap_or_else(|| "reset".into()),
            )
        };
        let request_id = new_request_id()?;
        let result = if id == store::MAIN_ID {
            self.use_main_reset(&account, reset_id, &request_id)?
        } else if account.provider == Provider::Claude {
            self.use_claude_reset(&account, reset_id, &request_id)?
        } else {
            self.use_codex_reset(&account, reset_id, &request_id)?
        };
        let label = account.label.clone();
        if let Err(refused) = result {
            (self.log)(&format!("{label}: reset refused ({})", refused.reason));
            let mut inner = self.lock();
            self.save_readings(&mut inner);
            return Err(ApiError(409, refused.message));
        }
        (self.log)(&format!("{label}: reset used ({title})"));
        {
            let mut inner = self.lock();
            inner.readings.use_reset(id, reset_id);
            self.save_readings(&mut inner);
        }
        // Read again as a refresh by hand reads it: its usage and its limit resets.
        if id == store::MAIN_ID {
            self.read_main_usage(true);
        } else {
            self.read_account_usage(&account, true);
        }
        let mut inner = self.lock();
        self.save_readings(&mut inner);
        Ok(())
    }

    /// Checks who `~/.claude` is signed in as, then claims the reset with Warden's own sign-in to main. `Err`: the folder
    /// is signed in as someone else.
    fn use_main_reset(
        &self,
        main: &AccountConfig,
        reset_id: &str,
        request_id: &str,
    ) -> ApiResult<Result<(), codex_resets::UseRefused>> {
        let observed = claude::read_identity(&main.folder, self.env.home.as_deref());
        let actual = observed.as_ref().ok().cloned().flatten();
        if self.apply_identity(store::MAIN_ID, observed) {
            return Err(signed_in_as_someone_else());
        }
        let mut login = self.lock().readings.main_login.clone();
        let main_changed = self.lock().readings.identity_changed(store::MAIN_ID);
        let result = mainlogin::use_reset_for_main(
            &self._lock,
            actual.as_ref(),
            main_changed,
            &mut login,
            (self.endpoint.as_ref(), reset_id, request_id),
            self.sign_in_server.as_ref(),
            now_ms(),
        );
        for line in login.take_log_lines() {
            (self.log)(&line);
        }
        self.lock().readings.main_login = login;
        Ok(result.unwrap_or_else(|e| Err(claude::login_refusal(&e))))
    }

    /// Checks who a Claude account's folder is signed in as, then claims the reset with the login in its folder, for the
    /// organization it is signed in to. `Err`: the folder is signed in as someone else.
    fn use_claude_reset(
        &self,
        account: &AccountConfig,
        reset_id: &str,
        request_id: &str,
    ) -> ApiResult<Result<(), codex_resets::UseRefused>> {
        let observed = claude::read_identity(&account.folder, self.env.home.as_deref());
        let unchecked = observed.is_err();
        let actual = observed.as_ref().ok().cloned().flatten();
        if self.apply_identity(&account.id, observed) {
            return Err(signed_in_as_someone_else());
        }
        if unchecked {
            return Err(identity_unchecked());
        }
        // Only while the folder is signed in as the account itself: without an identity it is signed out, and nothing
        // is sent.
        let Some(actual) = actual else {
            return Ok(Err(claude::login_refusal(&UsageError::SignedOut)));
        };
        let Some(organization) = actual.organization_id else {
            return Ok(Err(codex_resets::UseRefused::new(
                "no organization id",
                "Warden does not know the account's organization yet.",
            )));
        };
        Ok(claude::use_reset(
            &account.folder,
            self.endpoint.as_ref(),
            now_ms(),
            (&organization, reset_id, request_id),
        ))
    }

    /// Reads a Codex account's login file once for who it is signed in as and its access, then redeems the reset with
    /// it. `Err`: the folder is signed in as someone else.
    fn use_codex_reset(
        &self,
        account: &AccountConfig,
        reset_id: &str,
        request_id: &str,
    ) -> ApiResult<Result<(), codex_resets::UseRefused>> {
        let (now, access) = match codex::read_login_with_access(&account.folder) {
            Ok(login) => (Ok(login.identity), login.access),
            Err(e) => (Err(e), None),
        };
        let unchecked = now.is_err();
        let signed_in = matches!(now, Ok(Some(_)));
        if self.apply_identity(&account.id, now) {
            return Err(signed_in_as_someone_else());
        }
        if unchecked {
            return Err(identity_unchecked());
        }
        // Only while the folder is signed in as the account itself: a login file with access but no identity in it is
        // signed out, and nothing is sent.
        let (true, Some((token, account_id))) = (signed_in, access) else {
            return Ok(Err(claude::login_refusal(&UsageError::SignedOut)));
        };
        if let Err(refused) = codex_resets::check_redeem_ids(reset_id, request_id) {
            return Ok(Err(refused));
        }
        let result = self
            .resets
            .redeem(&token, &account_id, reset_id, request_id);
        drop(token);
        Ok(result)
    }

    /// Reads a Codex account's banked resets with the access read from its login file with its identity: the access
    /// token and account id go to ChatGPT's backend in this one request and are dropped with it. Asked only while the
    /// folder is signed in as the account; an answer that comes back after its identity changed is dropped.
    fn read_codex_resets(&self, id: &str, (token, account_id): (Secret, String)) {
        let since = {
            let inner = self.lock();
            if inner.readings.identity_changed(id) {
                return;
            }
            inner
                .readings
                .accounts
                .get(id)
                .and_then(|r| r.identity_since)
        };
        let result = self.resets.fetch(&token, &account_id);
        drop(token);
        self.apply_resets(id, since, result);
    }

    /// Records a resets read: a list replaces the last one and counts as their last read; a failure keeps it, is logged
    /// once and is not counted, so the next reading asks again. Dropped when the folder's identity changed (or a new
    /// one was accepted) since the request was sent: it would be someone else's.
    fn apply_resets(&self, id: &str, since: Option<i64>, result: Result<Vec<Reset>, String>) {
        let mut inner = self.lock();
        let Some(label) = inner
            .settings
            .account_in(&self.env, id)
            .map(|a| a.label.clone())
        else {
            return;
        };
        let now_since = inner
            .readings
            .accounts
            .get(id)
            .and_then(|r| r.identity_since);
        if inner.readings.identity_changed(id) || now_since != since {
            return;
        }
        match result {
            Ok(resets) => {
                inner.resets_failed.remove(id);
                inner.resets_read.insert(id.to_owned(), now_ms());
                let reading = inner.readings.accounts.entry(id.to_owned()).or_default();
                if reading.resets != resets {
                    reading.resets = resets;
                    self.save_readings(&mut inner);
                }
            }
            Err(why) => {
                if inner.resets_failed.get(id) != Some(&why) {
                    (self.log)(&format!("{label}: no banked resets reading ({why})"));
                    inner.resets_failed.insert(id.to_owned(), why);
                }
            }
        }
    }

    /// Records the identity read from an account's folder. Returns whether the folder is signed in as someone else.
    pub fn apply_identity(&self, id: &str, now: io::Result<Option<Identity>>) -> bool {
        let mut inner = self.lock();
        let Some(account) = inner.settings.account_in(&self.env, id) else {
            return false;
        };
        let now = match now {
            Ok(n) => n,
            Err(e) => {
                (self.log)(&format!(
                    "{}: could not read who it is signed in as ({})",
                    account.label,
                    e.kind()
                ));
                // Unknown: keep whatever was known.
                return inner
                    .readings
                    .accounts
                    .get(id)
                    .is_some_and(|r| r.identity_change.is_some());
            }
        };
        if id == store::MAIN_ID {
            inner.main_signed_in = now.is_some();
        }
        let i = &mut *inner;
        let before = i.readings.accounts.get(id).cloned();
        let fresh_identity = now.clone();
        let (changed, dirty_settings) = match i.readings.apply_identity(&i.settings, &account, now)
        {
            IdentityUpdate::Unchanged => {
                // A plan or display name can change without changing the person/workspace.
                let updated = fresh_identity
                    .map(|mut fresh| {
                        if fresh.organization_id.is_none() {
                            fresh.organization_id = account
                                .identity
                                .as_ref()
                                .and_then(|known| known.organization_id.clone());
                        }
                        fresh
                    })
                    .filter(|fresh| account.identity.as_ref() != Some(fresh));
                if let Some(fresh) = updated {
                    if id == store::MAIN_ID {
                        i.settings.main.identity = Some(fresh);
                    } else if let Some(a) = i.settings.accounts.iter_mut().find(|a| a.id == id) {
                        a.identity = Some(fresh);
                    }
                    (false, true)
                } else {
                    (false, false)
                }
            }
            IdentityUpdate::FirstSeen(ident) => {
                if id == store::MAIN_ID {
                    i.settings.main.identity = Some(ident);
                } else if let Some(a) = i.settings.accounts.iter_mut().find(|a| a.id == id) {
                    a.identity = Some(ident);
                }
                (false, true)
            }
            IdentityUpdate::Changed { new } => {
                if new {
                    (self.log)(&format!("{}: signed in as someone else now", account.label));
                }
                (true, false)
            }
        };
        if dirty_settings {
            let _ = self.save_settings(&mut inner);
        }
        if changed {
            let who = signed_in_as_text(inner.readings.accounts.get(id));
            if inner.sessions.changed_account(id, &who, now_ms()) {
                self.touch_sessions(&mut inner);
            }
        }
        // Signed in as the account again: its sessions count for it from now on.
        let settled = before.as_ref().is_some_and(|r| r.identity_change.is_some())
            && !inner.readings.identity_changed(id);
        if settled && inner.sessions.settle_account(id, now_ms()) {
            self.touch_sessions(&mut inner);
        }
        // Saved only when they changed: every new session checks the identity, which is mostly unchanged.
        if inner.readings.accounts.get(id) != before.as_ref() {
            self.save_readings(&mut inner);
        }
        changed
    }

    /// Records a usage attempt: a reading replaces the last one; an error is kept beside the last good reading.
    pub fn apply_usage(&self, id: &str, result: Result<Usage, UsageError>) {
        let mut inner = self.lock();
        let Some(label) = inner
            .settings
            .account_in(&self.env, id)
            .map(|a| a.label.clone())
        else {
            return;
        };
        let message = result.as_ref().err().map(describe);
        // A Claude answer with a readable list of limit resets (eligible or not) is their last read, once kept: an
        // answer for an identity no longer the account's is ignored and does not hold off the new one's resets.
        let gave_resets = result.as_ref().is_ok_and(|u| u.resets.is_some());
        match inner.readings.apply_usage(id, result) {
            UsageUpdate::Ignored => return,
            UsageUpdate::Read => {
                if gave_resets {
                    inner.resets_read.insert(id.to_owned(), now_ms());
                }
                (self.log)(&format!("{label}: usage read"))
            }
            UsageUpdate::Failed { new: true } => (self.log)(&format!(
                "{label}: no usage reading ({})",
                message.unwrap_or_default()
            )),
            UsageUpdate::Failed { new: false } => {}
        }
        self.save_readings(&mut inner);
    }

    /// Applies a Codex usage reading from the logs, unless the folder is signed in as someone else.
    pub fn apply_codex_usage(&self, id: &str, usage: Usage) {
        let (held, signed_out) = {
            let inner = self.lock();
            let signed_out = inner
                .settings
                .account_in(&self.env, id)
                .is_some_and(|a| !codex::has_login(&a.folder));
            (inner.readings.identity_changed(id), signed_out)
        };
        if held {
            self.apply_usage(id, Err(UsageError::IdentityChanged));
        } else if signed_out {
            // A log written after signing out: the account stays signed out until its login is back.
            self.apply_usage(id, Err(UsageError::SignedOut));
        } else {
            self.apply_usage(id, Ok(usage));
            let mut inner = self.lock();
            inner.last_read = Some(now_ms());
            self.touch(&mut inner);
        }
    }

    /// Checks a Codex folder's login file: who it is signed in as, and whether it is signed in at all.
    pub fn check_codex_login(&self, id: &str, folder: &Path) {
        self.apply_identity(id, codex::read_identity_from_login_file(folder));
        let signed_out = !codex::has_login(folder);
        let mut inner = self.lock();
        let was = inner
            .readings
            .accounts
            .get(id)
            .and_then(|r| r.error.clone());
        inner.readings.set_signed_out(id, signed_out);
        if inner
            .readings
            .accounts
            .get(id)
            .and_then(|r| r.error.clone())
            != was
        {
            self.save_readings(&mut inner);
        }
    }

    pub fn usage_interval(&self) -> Duration {
        Duration::from_secs(u64::from(self.lock().settings.usage_interval_min) * 60)
    }

    /// The usage loop waits for the interval or a wake-up. Returns false once the server stops.
    pub fn wait(&self, timeout: Duration) -> bool {
        let mut inner = self.lock();
        inner.usage_busy = false;
        if inner.stop {
            return false;
        }
        let (mut inner, _) = self
            .wake
            .wait_timeout_while(inner, timeout, |i| !i.usage_wanted && !i.stop)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.usage_wanted = false;
        inner.usage_busy = !inner.stop;
        !inner.stop
    }

    /// Asks the usage loop for a reading now.
    pub fn wake_up(&self) {
        self.lock().usage_wanted = true;
        self.wake.notify_all();
    }

    /// Whether the usage loop is waiting with no reading asked for, so no background reading is under way.
    pub fn usage_idle(&self) -> bool {
        let inner = self.lock();
        !inner.usage_wanted && !inner.usage_busy
    }

    pub fn stop(&self) {
        self.lock().stop = true;
        self.wake.notify_all();
        self.changed.notify_all();
        self.poke_codex();
        self.poke_hooks();
    }

    /// Wakes the Codex watcher so it notices a change in the accounts (or the stop).
    fn poke_codex(&self) {
        poke(&self.codex_wake);
    }

    /// Wakes the hook file watcher so it notices a change in the accounts (or the stop).
    fn poke_hooks(&self) {
        poke(&self.hooks_wake);
    }

    pub fn stopped(&self) -> bool {
        self.lock().stop
    }

    /// The Codex accounts to watch, and a number that changes when they do.
    pub fn codex_folders(&self) -> (u64, Vec<(String, PathBuf)>) {
        let inner = self.lock();
        let folders = inner
            .settings
            .accounts
            .iter()
            .filter(|a| a.provider == Provider::Codex)
            .map(|a| (a.id.clone(), a.folder.clone()))
            .collect();
        (inner.codex_generation, folders)
    }

    pub fn codex_generation(&self) -> u64 {
        self.lock().codex_generation
    }

    /// Every account's folder and provider, and a number that changes when they do.
    pub fn account_folders(&self) -> (u64, Vec<(String, Provider, PathBuf)>) {
        let inner = self.lock();
        let folders = inner
            .settings
            .all_accounts(&self.env)
            .into_iter()
            .map(|a| (a.id.clone(), a.provider, a.folder.clone()))
            .collect();
        (inner.folders_generation, folders)
    }

    pub fn folders_generation(&self) -> u64 {
        self.lock().folders_generation
    }

    fn codex_changed(inner: &mut Inner) {
        inner.codex_generation += 1;
    }

    // ---- Sessions ------------------------------------------------------------------------------------------

    /// Takes one status hook event from an account's hook. A session seen for the first time (or resumed) gets the
    /// identity check: when the folder is signed in as someone else, the session shows who and counts for no account.
    pub fn receive_event(&self, account_id: &str, event: &Event) -> ApiResult<()> {
        let (account, is_new) = {
            let mut inner = self.lock();
            let account = inner
                .settings
                .account_in(&self.env, account_id)
                .ok_or_else(ApiError::not_found)?;
            inner.hooked.insert(account.id.clone());
            let is_new = inner.sessions.is_new(account.provider, &event.session_id);
            (account, is_new)
        };
        let mut signed_in_as = None;
        if (is_new && event.kind != EventKind::End) || event.kind == EventKind::Start {
            let now =
                detect::read_identity(account.provider, &account.folder, self.env.home.as_deref());
            if self.apply_identity(&account.id, now) {
                signed_in_as = Some(self.signed_in_as(&account.id));
            }
        }
        // The project is worked out (a few small file reads) before taking the lock.
        let place = if is_new {
            event
                .cwd
                .as_deref()
                .map(sessions::place_of)
                .unwrap_or_default()
        } else {
            sessions::Place::default()
        };
        // A Claude session's title, from what was added to its transcript since the last event (a path inside the
        // account's folder only). Read before taking the lock, like the project.
        let title = event
            .transcript
            .as_deref()
            .filter(|t| {
                account.provider == Provider::Claude && titles::is_transcript_of(&account.folder, t)
            })
            .and_then(|t| {
                let mut transcripts = self
                    .transcripts
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let title = transcripts.read(&event.session_id, t);
                if event.kind == EventKind::End {
                    transcripts.forget(&event.session_id);
                }
                title
            });
        let mut inner = self.lock();
        let mut changed = inner.sessions.apply_as(
            account.provider,
            &account.id,
            signed_in_as.as_deref(),
            || place,
            event,
            now_ms(),
        );
        if let Some(title) = title {
            changed |= inner
                .sessions
                .name(account.provider, vec![(event.session_id.clone(), title)]);
        }
        if changed {
            self.touch_sessions(&mut inner);
        }
        Ok(())
    }

    /// Who an account's folder is signed in as instead of the account, for its sessions: the email, or a plain
    /// word when the change is known but not the new sign-in.
    fn signed_in_as(&self, id: &str) -> String {
        signed_in_as_text(self.lock().readings.accounts.get(id))
    }

    /// Reads a Claude transcript that changed on disk, when a session follows it, for a new title.
    pub fn transcript_changed(&self, path: &Path) {
        let named = self
            .transcripts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .changed(path);
        if let Some(named) = named {
            self.name_sessions(Provider::Claude, vec![named]);
        }
    }

    /// Gives sessions their own names (Codex's thread names, as its index grows).
    pub fn name_sessions(&self, provider: Provider, names: Vec<(String, String)>) {
        if names.is_empty() {
            return;
        }
        let mut inner = self.lock();
        if inner.sessions.name(provider, names) {
            self.touch_sessions(&mut inner);
        }
    }

    /// Takes one message from a hook over the pipe: the event, and the process that sent it (the hook's
    /// `warden.exe`, still running). A session's first event (or a restart) also finds the session's CLI process
    /// up that process's parent chain, so the session ends when the CLI exits.
    /// Keeps which main session started each delegated run, so the tree is whole again after a restart.
    fn save_parent_links(&self, inner: &Inner) {
        if let Err(e) = store::save(
            &self.data_dir,
            sessions::PARENT_LINKS_FILE,
            inner.sessions.parent_links(),
        ) {
            (self.log)(&format!(
                "could not keep the sessions' parents ({})",
                e.kind()
            ));
        }
    }

    pub fn receive_message(
        self: &Arc<Self>,
        message: &Message,
        client_pid: Option<u32>,
    ) -> ApiResult<()> {
        let event = sessions::event_from(&message.event).map_err(ApiError::bad)?;
        self.receive_event(&message.account, &event)?;
        let Some(provider) = self
            .lock()
            .settings
            .account_in(&self.env, &message.account)
            .map(|a| a.provider)
        else {
            return Ok(());
        };
        // The main session this run names (or its saved link names), looked up by id when it is not known: it may
        // have been idle for longer than the lists' window since Warden started. Once per id per start; one found
        // before and dropped since (no run under it shown for a while) comes back from what was found.
        let to_find = {
            let mut inner = self.lock();
            let named = message.event.parent_session_id.as_deref();
            if inner
                .sessions
                .restore_found_parent(provider, &event.session_id, named, now_ms())
            {
                self.touch_sessions(&mut inner);
            }
            inner
                .sessions
                .parent_to_find(provider, &event.session_id, named, now_ms())
        };
        if let Some(parent) = to_find {
            self.find_parents(vec![parent]);
        }
        {
            let mut inner = self.lock();
            if inner.sessions.set_parent(
                provider,
                &event.session_id,
                message.event.parent_session_id.as_deref(),
                now_ms(),
            ) {
                self.save_parent_links(&inner);
                self.touch_sessions(&mut inner);
            }
        }
        if !self
            .lock()
            .sessions
            .needs_process(provider, &event.session_id)
        {
            return Ok(());
        }
        let cli = client_pid.and_then(|pid| crate::procs::cli_process(pid, provider));
        let parent = if message.event.parent_session_id.is_none() {
            let main = self.lock().sessions.main_processes();
            cli.and_then(|pid| crate::procs::parent_session(pid, &main))
        } else {
            None
        };
        let mut inner = self.lock();
        inner.sessions.set_process(provider, &event.session_id, cli);
        if inner
            .sessions
            .set_parent(provider, &event.session_id, parent.as_deref(), now_ms())
        {
            self.save_parent_links(&inner);
            self.touch_sessions(&mut inner);
        }
        let Some(pid) = cli else {
            return Ok(());
        };
        if inner.watched.insert(pid) {
            drop(inner);
            let app: Weak<App> = Arc::downgrade(self);
            let grace = Duration::from_millis(u64::try_from(sessions::LATE_MS).unwrap_or(2_000));
            if crate::procs::watch(pid, grace, move || {
                if let Some(app) = app.upgrade() {
                    app.process_exited(pid);
                }
            })
            .is_err()
            {
                // Gone already: its sessions end now.
                self.process_exited(pid);
            }
        }
        Ok(())
    }

    /// A session's CLI process exited: its sessions end.
    pub fn process_exited(&self, pid: u32) {
        let mut inner = self.lock();
        inner.watched.remove(&pid);
        if inner.sessions.process_exited(pid, now_ms()) {
            self.touch_sessions(&mut inner);
        }
    }

    /// The fallback where no hook reports: sessions from the CLIs' own lists, for accounts whose status hook has
    /// not reported since Warden started. Read when Warden starts and when usage is refreshed by hand, never on a
    /// timer.
    pub fn list_sessions(&self) {
        /// Sessions written to within [`sessions::LISTED_KEPT_MS`] count (and stay that long after their last
        /// write unless a hook event takes them over); at most this many per account.
        const MAX_PER_ACCOUNT: usize = 20;
        // As with the status hook's sessions: a folder signed in as someone else shows its sessions with who, and
        // they count for no account.
        let accounts: Vec<(AccountConfig, Option<String>)> = {
            let inner = self.lock();
            inner
                .settings
                .all_accounts(&self.env)
                .into_iter()
                .filter(|a| a.enabled && !inner.hooked.contains(&a.id))
                .map(|a| {
                    let r = inner.readings.accounts.get(&a.id);
                    let signed_in_as = r
                        .is_some_and(|r| r.identity_change.is_some())
                        .then(|| signed_in_as_text(r));
                    (a.clone(), signed_in_as)
                })
                .collect()
        };
        let now = now_ms();
        for (a, signed_in_as) in accounts {
            let listed: Vec<_> = fallback::listed_sessions(
                a.provider,
                &a.folder,
                now,
                sessions::LISTED_KEPT_MS,
                MAX_PER_ACCOUNT,
            )
            .into_iter()
            .map(|l| {
                let place = l.cwd.as_deref().map(sessions::place_of).unwrap_or_default();
                (l.id, l.last_at, place)
            })
            .collect();
            if listed.is_empty() {
                continue;
            }
            let mut inner = self.lock();
            if !inner.hooked.contains(&a.id)
                && inner.sessions.add_listed_as(
                    a.provider,
                    &a.id,
                    signed_in_as.as_deref(),
                    listed,
                    now,
                )
            {
                self.touch_sessions(&mut inner);
            }
        }
    }

    /// Looks up main sessions that delegated runs name as their parent, by id, in the accounts in use (main's
    /// folder first, the Codex app's folder never), whatever their age, and adds each one found as a listed session.
    /// Only the session files are read ([`fallback::find_session`]); nothing is started.
    fn find_parents(&self, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }
        let accounts: Vec<(AccountConfig, Option<String>)> = {
            let inner = self.lock();
            inner
                .settings
                .all_accounts(&self.env)
                .into_iter()
                .filter(|a| a.enabled && !self.env.in_codex_app(&a.folder))
                .map(|a| {
                    let r = inner.readings.accounts.get(&a.id);
                    let signed_in_as = r
                        .is_some_and(|r| r.identity_change.is_some())
                        .then(|| signed_in_as_text(r));
                    (a, signed_in_as)
                })
                .collect()
        };
        for id in ids {
            let Some((a, signed_in_as, found)) = accounts.iter().find_map(|(a, signed_in_as)| {
                fallback::find_session(&self.env, a.provider, &a.folder, &id)
                    .map(|found| (a, signed_in_as, found))
            }) else {
                continue;
            };
            let place = found
                .cwd
                .as_deref()
                .map(sessions::place_of)
                .unwrap_or_default();
            let mut inner = self.lock();
            if inner.sessions.add_found(
                a.provider,
                &a.id,
                signed_in_as.as_deref(),
                (found.id, found.last_at, place),
                now_ms(),
            ) {
                self.touch_sessions(&mut inner);
            }
        }
    }

    // ---- Settings and accounts ------------------------------------------------------------------------------

    /// "Open log folder": the desktop app opens the data folder's `logs` in Explorer. The demo server only shows its
    /// path.
    pub fn open_log_folder(&self) -> ApiResult<()> {
        let Some(desktop) = &self.desktop else {
            return Err(ApiError::bad(
                "Only the desktop app opens the log folder; its path is shown in Settings.",
            ));
        };
        let dir = logfile::log_dir(&self.data_dir);
        std::fs::create_dir_all(&dir)
            .and_then(|()| desktop.open_folder(&dir))
            .map_err(|e| {
                (self.log)(&format!("could not open the log folder: {e}"));
                ApiError(500, "Warden could not open the log folder.".into())
            })
    }

    /// The desktop app's updater reports a newer Warden, its download, or nothing newer.
    pub fn set_update(&self, update: Option<UpdateView>) {
        let mut inner = self.lock();
        if inner.update != update {
            inner.update = update;
            self.touch_sessions(&mut inner);
        }
    }

    /// The newer Warden the desktop app found, if any.
    pub fn update(&self) -> Option<UpdateView> {
        self.lock().update.clone()
    }

    /// "Install and restart": the desktop app installs the downloaded update and starts again.
    pub fn install_update(&self) -> ApiResult<()> {
        let Some(desktop) = &self.desktop else {
            return Err(ApiError::bad("Only the desktop app installs updates."));
        };
        if !self.lock().update.as_ref().is_some_and(|u| u.ready) {
            return Err(ApiError(409, "No update is ready to install.".into()));
        }
        desktop.install_update().map_err(|e| {
            (self.log)(&format!("could not install the update: {e}"));
            ApiError(500, "Warden could not install the update.".into())
        })
    }

    pub fn set_settings(&self, patch: SettingsPatch) -> ApiResult<()> {
        if patch.reserve.is_some_and(|r| r > 50 || r % 5 != 0) {
            return Err(ApiError::bad(
                "Main's reserve is from 0 to 50% in steps of 5%.",
            ));
        }
        if patch
            .reset_warning_days
            .is_some_and(|d| !store::RESET_WARNING_DAYS.contains(&d))
        {
            return Err(ApiError::bad(
                "Warn from 1 to 14 days before a reset expires.",
            ));
        }
        if patch
            .usage_interval_min
            .is_some_and(|i| !store::USAGE_INTERVALS.contains(&i))
        {
            return Err(ApiError::bad(
                "Usage can be read every 2, 5, 10 or 30 minutes.",
            ));
        }
        if let Some(ids) = &patch.hidden_reset_accounts {
            let inner = self.lock();
            if !ids.iter().all(|id| known_account(&inner.settings, id)) {
                return Err(ApiError::bad(
                    "Only the resets of accounts Warden knows can be left out.",
                ));
            }
        }
        // `Some(None)` clears the work hours; anything but a valid set of hours or `null` is refused.
        let work_hours: Option<Option<WorkHours>> = match patch.work_hours.clone() {
            None => None,
            Some(serde_json::Value::Null) => Some(None),
            Some(v) => match serde_json::from_value::<WorkHours>(v) {
                Ok(w) if w.valid() => Some(Some(w)),
                _ => return Err(ApiError::bad(WORK_HOURS_REFUSED)),
            },
        };
        let desktop_only = patch.keep_in_tray.is_some() || patch.start_with_windows.is_some();
        if desktop_only && self.desktop.is_none() {
            return Err(ApiError::bad("Only the desktop app has this setting."));
        }
        if let (Some(on), Some(desktop)) = (patch.start_with_windows, &self.desktop) {
            let _one = self
                .start_with_windows_change
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let set = desktop.set_start_with_windows(on);
            // Read back either way: a refused change may still have changed the registry.
            let now = desktop.start_with_windows();
            let mut inner = self.lock();
            if inner.start_with_windows != Some(now) {
                inner.start_with_windows = Some(now);
                self.touch(&mut inner);
            }
            drop(inner);
            set.map_err(|e| {
                (self.log)(&format!("could not change start with Windows: {e}"));
                ApiError(500, "Warden could not change Start with Windows.".into())
            })?;
        }
        let mut inner = self.lock();
        // Only a new interval wakes the usage loop, which then waits the new one; the rest needs no reading (the
        // routing decision is made from the saved readings).
        let new_interval = patch
            .usage_interval_min
            .is_some_and(|i| i != inner.settings.usage_interval_min);
        if let Some(i) = patch.usage_interval_min {
            inner.settings.usage_interval_min = i;
        }
        if let Some(reserve) = patch.reserve {
            inner.settings.reserve = reserve;
        }
        if let Some(t) = patch.theme {
            inner.settings.theme = t;
        }
        if let Some(k) = patch.keep_in_tray {
            inner.settings.keep_in_tray = k;
        }
        if let Some(w) = work_hours {
            inner.settings.work_hours = w;
        }
        if let Some(d) = patch.reset_warning_days {
            inner.settings.reset_warning_days = d;
        }
        if let Some(q) = patch.show_quiet_sessions {
            inner.settings.show_quiet_sessions = q;
        }
        if let Some(ids) = patch.hidden_reset_accounts {
            // Checked above; an account removed since then is dropped here, under the same lock as the save, so no id
            // is kept that names no account.
            let mut seen = HashSet::new();
            let ids: Vec<String> = ids
                .into_iter()
                .filter(|id| known_account(&inner.settings, id) && seen.insert(id.clone()))
                .collect();
            inner.settings.hidden_reset_accounts = ids;
        }
        if let Some(s) = patch.soonest_reset_only {
            inner.settings.soonest_reset_only = s;
        }
        self.save_settings(&mut inner)?;
        drop(inner);
        if new_interval {
            self.wake_up();
        }
        Ok(())
    }

    pub fn scan_tools(&self) {
        let mut inner = self.lock();
        inner.tools = detect::tools(&self.env);
        self.touch(&mut inner);
    }

    pub fn candidates(&self) -> Vec<Candidate> {
        let known: Vec<PathBuf> = self
            .lock()
            .settings
            .accounts
            .iter()
            .map(|a| a.folder.clone())
            .collect();
        detect::account_folders_except(
            &self.env,
            Some(&warden_core::mainlogin::folder(&self.data_dir)),
        )
        .into_iter()
        .map(|(provider, folder)| Candidate {
            provider,
            label: detect::suggested_label(provider, &folder),
            identity: detect::read_identity(provider, &folder, self.env.home.as_deref())
                .ok()
                .flatten()
                .as_ref()
                .map(Into::into),
            signed_in: detect::has_login(provider, &folder),
            known: known.iter().any(|k| detect::same_path(k, &folder)),
            folder: show(&folder),
        })
        .collect()
    }

    /// Adds an existing Warden account, refusing protected folders before checking its login.
    pub fn add_existing(&self, provider: Provider, folder: &str, label: &str) -> ApiResult<String> {
        let folder = PathBuf::from(folder.trim().trim_matches('"').trim());
        let label = clean_label(label)?;
        if !folder.is_absolute() {
            return Err(ApiError::bad("Give the folder's full path."));
        }
        let folder = self.checked_account_folder(folder)?;
        if !folder.is_dir() {
            return Err(ApiError::bad("That folder does not exist."));
        }
        if !detect::has_login(provider, &folder) {
            return Err(ApiError::bad(format!(
                "There is no signed-in {} login in that folder.",
                provider.name()
            )));
        }
        let identity = detect::read_identity(provider, &folder, self.env.home.as_deref())
            .ok()
            .flatten();
        self.insert_account(provider, folder, label, identity)
    }

    /// Main, the Codex app and `~/.warden` itself are never added as Warden accounts, regardless of aliases or
    /// confirmation.
    fn checked_account_folder(&self, folder: PathBuf) -> ApiResult<PathBuf> {
        self.left_alone(&folder)?;
        if let Some(home) = &self.env.home
            && detect::same_folder(&folder, &home.join(detect::WARDEN_FOLDER))
        {
            return Err(ApiError(
                400,
                "That is Warden's folder for its accounts; choose a folder inside it.".into(),
            ));
        }
        Ok(detect::real_folder(&folder))
    }

    /// Refuses main's folder, the Codex app's (and everything in it) and Warden's own sign-in folder.
    fn left_alone(&self, folder: &Path) -> ApiResult<()> {
        if self.env.in_codex_app(folder) {
            return Err(ApiError(
                400,
                "That is the Codex app's folder; Warden leaves it alone.".into(),
            ));
        }
        if self.env.desktop_app_of(folder) == Some(Provider::Claude) {
            return Err(ApiError(
                409,
                "That is main's folder; main is always the Claude app's account.".into(),
            ));
        }
        if detect::same_folder(folder, &warden_core::mainlogin::folder(&self.data_dir)) {
            return Err(ApiError(400, "That is Warden's own sign-in folder.".into()));
        }
        Ok(())
    }

    fn insert_account(
        &self,
        provider: Provider,
        folder: PathBuf,
        label: String,
        identity: Option<Identity>,
    ) -> ApiResult<String> {
        let mut inner = self.lock();
        if inner
            .settings
            .accounts
            .iter()
            .any(|a| detect::same_folder(&a.folder, &folder))
        {
            return Err(ApiError(
                409,
                "That folder is already one of Warden's accounts.".into(),
            ));
        }
        let id = inner.settings.new_id();
        (self.log)(&format!("{label}: added"));
        inner.settings.accounts.push(AccountConfig {
            id: id.clone(),
            label,
            provider,
            folder,
            enabled: true,
            identity,
        });
        inner.settings.normalise();
        let first_after_setup = inner.settings.setup_done && inner.settings.accounts.len() == 1;
        if provider == Provider::Codex {
            Self::codex_changed(&mut inner);
        }
        inner.folders_generation += 1;
        self.save_settings(&mut inner)?;
        drop(inner);
        self.wake_up();
        self.poke_codex();
        self.poke_hooks();
        let _ = self.repair_account_hooks(&id);
        if first_after_setup {
            let _ = self.repair_account_hooks(store::MAIN_ID);
        }
        Ok(id)
    }

    /// The app, shared: what finishes later (a sign-in's window closing) can reach it.
    pub fn shared(self) -> Arc<App> {
        let app = Arc::new(self);
        let _ = app.me.set(Arc::downgrade(&app));
        app
    }

    /// What runs when a sign-in's CLI exits: usage is read at once, not at the next interval. A cancelled sign-in
    /// changed no login file, so that reading spends nothing.
    fn after_sign_in(&self, label: String) -> fallback::Done {
        let me = self.me.get().cloned();
        Box::new(move || {
            if let Some(app) = me.and_then(|w| w.upgrade()) {
                (app.log)(&format!("{label}: the sign-in window closed"));
                app.wake_up();
            }
        })
    }

    /// Starts the official sign-in in Warden's own main-sign-in folder.
    pub fn sign_in_main(&self) -> ApiResult<()> {
        let cli=detect::cli_path(&self.env,Provider::Claude).ok_or_else(||ApiError::bad("The claude command line was not found on PATH. Choose Scan again after installing it."))?;
        let _one = self
            .reading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (folder, login) = mainlogin::begin_sign_in(&self._lock)
            .map_err(|_| ApiError(500, "Warden could not prepare its sign-in folder.".into()))?;
        let done = self.after_sign_in("Main".into());
        (self.sign_in)(Provider::Claude, &cli, &folder, done).map_err(|e| {
            (self.log)(&format!(
                "Main: could not start Warden's sign-in ({})",
                e.kind()
            ));
            ApiError(500, "Warden could not start the sign-in.".into())
        })?;
        let mut inner = self.lock();
        inner.readings.main_login = login;
        inner
            .readings
            .apply_usage(store::MAIN_ID, Err(UsageError::SignedOut));
        self.save_readings(&mut inner);
        drop(inner);
        self.wake_up();
        Ok(())
    }

    /// Creates a new Warden account folder and starts its official sign-in.
    pub fn add_new(&self, provider: Provider, label: &str) -> ApiResult<String> {
        let label = clean_label(label)?;
        let Some(cli) = detect::cli_path(&self.env, provider) else {
            return Err(ApiError::bad(format!(
                "The {} command line was not found on PATH. Install it, then choose Scan again in Settings, General.",
                provider.name().to_lowercase()
            )));
        };
        let no_home = || ApiError(500, "Warden could not find your home folder.".into());
        // `~/.warden` may be a link the user made: nothing is looked up or made in a folder Warden leaves alone.
        let parent = self
            .env
            .home
            .as_ref()
            .ok_or_else(no_home)?
            .join(detect::WARDEN_FOLDER);
        self.left_alone(&parent)?;
        let folder = detect::new_account_folder(&self.env, provider, &label).ok_or_else(no_home)?;
        let made_parent = !parent.exists();
        std::fs::create_dir_all(&folder)
            .map_err(|_| ApiError(500, "Warden could not create the folder.".into()))?;
        if let Err(e) = (self.sign_in)(provider, &cli, &folder, self.after_sign_in(label.clone())) {
            (self.log)(&format!(
                "{label}: could not start the sign-in ({})",
                e.kind()
            ));
            let _ = std::fs::remove_dir(&folder);
            if made_parent {
                let _ = std::fs::remove_dir(&parent);
            }
            return Err(ApiError(500, "Warden could not start the sign-in.".into()));
        }
        self.insert_account(provider, folder, label, None)
    }

    pub fn rename_main(&self, label: &str) -> ApiResult<()> {
        let label = clean_label(label)?;
        let mut inner = self.lock();
        inner.settings.main.label = label;
        self.save_settings(&mut inner)
    }
    pub fn update_account(
        &self,
        id: &str,
        label: Option<&str>,
        enabled: Option<bool>,
    ) -> ApiResult<()> {
        let change = self.hook_change(id);
        let _one = change
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if id == store::MAIN_ID {
            return Err(ApiError::bad("Use main's Rename action."));
        }
        let label = label.map(clean_label).transpose()?;
        let mut inner = self.lock();
        let account = inner
            .settings
            .accounts
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(ApiError::not_found)?;
        if let Some(label) = label {
            account.label = label;
        }
        if let Some(enabled) = enabled {
            account.enabled = enabled;
        }
        self.save_settings(&mut inner)?;
        drop(inner);
        if enabled.is_some() {
            self.apply_current_hooks(id)?;
        }
        Ok(())
    }
    pub fn sign_in_account(&self, id: &str) -> ApiResult<()> {
        if id == store::MAIN_ID {
            return Err(ApiError::bad("Use Warden's main sign-in."));
        }
        let account = self
            .lock()
            .settings
            .account(id)
            .cloned()
            .ok_or_else(ApiError::not_found)?;
        let cli = detect::cli_path(&self.env, account.provider).ok_or_else(|| {
            ApiError::bad(
                "The command line was not found on PATH. Choose Scan again after installing it.",
            )
        })?;
        let done = self.after_sign_in(account.label.clone());
        (self.sign_in)(account.provider, &cli, &account.folder, done)
            .map_err(|_| ApiError(500, "Warden could not start the sign-in.".into()))
    }
    pub fn repair_account_hooks(&self, id: &str) -> ApiResult<()> {
        let change = self.hook_change(id);
        let _one = change
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.apply_current_hooks(id)
    }

    fn hook_change(&self, id: &str) -> Arc<Mutex<()>> {
        self.hook_changes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(id.into())
            .or_default()
            .clone()
    }

    fn apply_current_hooks(&self, id: &str) -> ApiResult<()> {
        let account = self
            .lock()
            .settings
            .account_in(&self.env, id)
            .ok_or_else(ApiError::not_found)?;
        self.apply_hooks(&account, id == store::MAIN_ID, account.enabled)
    }

    /// Accepts the identity the folder is signed in as now as the account's own.
    pub fn accept_identity(&self, id: &str) -> ApiResult<()> {
        if id == store::MAIN_ID {
            return Err(ApiError::bad("Restore main's sign-in in the Claude app."));
        }
        let mut inner = self.lock();
        let change: Option<IdentityChange> = inner
            .readings
            .accounts
            .get_mut(id)
            .and_then(|r| r.identity_change.take());
        let Some(change) = change else {
            return Err(ApiError::bad("This account's identity has not changed."));
        };
        let account = inner
            .settings
            .accounts
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(ApiError::not_found)?;
        account.identity = change.now;
        (self.log)(&format!("{}: new identity accepted", account.label));
        // Readings of the old identity no longer describe this account, now or after a restart.
        inner.readings.accounts.insert(
            id.to_owned(),
            AccountReading {
                identity_since: Some(now_ms()),
                ..AccountReading::default()
            },
        );
        // The new identity's resets are read with the reading this starts, not hours later.
        inner.resets_read.remove(id);
        // Its sessions signed in as the new identity are the account's own now.
        inner.sessions.settle_account(id, now_ms());
        self.save_settings(&mut inner)?;
        self.save_readings(&mut inner);
        drop(inner);
        self.wake_up();
        Ok(())
    }

    pub fn remove_account(&self, id: &str) -> ApiResult<()> {
        let change = self.hook_change(id);
        let _one = change
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if id == store::MAIN_ID {
            return Err(ApiError::bad("Main cannot be removed."));
        }
        // Warden's own hooks go with the account; the folder is otherwise left as it is.
        let account = self.lock().settings.account_in(&self.env, id);
        if let Some(a) = account {
            // A hook that cannot be removed would keep advising (or reporting) that account's sessions with nothing
            // left to remove it later: the account stays until it can be.
            self.apply_hooks(&a, false, false)?;
        }
        let mut inner = self.lock();
        let i = inner
            .settings
            .accounts
            .iter()
            .position(|a| a.id == id)
            .ok_or_else(ApiError::not_found)?;
        let removed = inner.settings.accounts.remove(i);
        (self.log)(&format!("{}: removed", removed.label));
        inner.readings.accounts.remove(id);
        inner.resets_read.remove(id);
        inner.sessions.forget_account(id);
        inner.hooked.remove(id);
        inner.hook_setups.remove(id);
        inner.claude_shells.remove(id);
        inner.folders_generation += 1;
        for r in inner.readings.accounts.values_mut() {
            if let Some(c) = &mut r.identity_change
                && c.known_account_id.as_deref() == Some(id)
            {
                c.known_account_id = None;
            }
        }
        if removed.provider == Provider::Codex {
            Self::codex_changed(&mut inner);
        }
        inner.settings.normalise();
        self.save_settings(&mut inner)?;
        self.save_readings(&mut inner);
        // Removing a Warden account never changes main.
        drop(inner);
        self.poke_codex();
        self.poke_hooks();
        Ok(())
    }

    /// Finishes setup with Warden folders; main is implied. Validate every choice before adding any.
    pub fn finish_setup(&self, chosen: &[(Provider, String, String)], skip: bool) -> ApiResult<()> {
        if skip {
            if !chosen.is_empty() {
                return Err(ApiError::bad("Skip cannot include accounts."));
            }
            let mut inner = self.lock();
            inner.settings.setup_done = true;
            return self.save_settings(&mut inner);
        }
        if !self.lock().tools.iter().any(|t| {
            t.provider == Provider::Claude && t.kind == detect::ToolKind::App && t.path.is_some()
        }) {
            return Err(ApiError(
                409,
                "Install and sign in to the Claude app first.".into(),
            ));
        }
        let main = self
            .lock()
            .settings
            .main_account(&self.env)
            .ok_or_else(|| ApiError(409, "Sign in to the Claude app first.".into()))?;
        let identity = claude::read_identity(&main.folder, self.env.home.as_deref())
            .ok()
            .flatten()
            .ok_or_else(|| ApiError(409, "Sign in to the Claude app first.".into()))?;
        self.apply_identity(store::MAIN_ID, Ok(Some(identity)));
        let mut checked = Vec::with_capacity(chosen.len());
        for (provider, folder, label) in chosen {
            let path = PathBuf::from(folder.trim().trim_matches('"').trim());
            let path = self.checked_account_folder(path)?;
            if !detect::looks_like_account(*provider, &path, self.env.home.as_deref()) {
                return Err(ApiError::bad(format!(
                    "{folder} is not a {} account folder.",
                    provider.name()
                )));
            }
            checked.push((*provider, path, clean_label(label)?));
        }
        for (provider, path, label) in checked {
            let identity = detect::read_identity(provider, &path, self.env.home.as_deref())
                .ok()
                .flatten();
            match self.insert_account(provider, path, label, identity) {
                Ok(_) | Err(ApiError(409, _)) => {}
                Err(e) => return Err(e),
            }
        }
        let mut inner = self.lock();
        inner.settings.setup_done = true;
        self.save_settings(&mut inner)?;
        drop(inner);
        self.repair_account_hooks(store::MAIN_ID)?;
        Ok(())
    }

    // ---- Routing and hooks --------------------------------------------------------------------------------------

    /// Rewrites Warden's own outdated hooks (an earlier build's `warden.exe`, data folder, form or flags, or its
    /// `curl` status hook), and removes hooks no longer wanted. Missing wanted hooks stay visible for Repair.
    /// The user's own hooks and backup ledger are preserved; repairs are logged without an update note.
    pub fn repair_hooks(&self) {
        let ours = self.ours();
        if ours.tool.is_none() {
            return;
        }
        let (accounts, too_old) = {
            let inner = self.lock();
            let accounts = inner.settings.all_accounts(&self.env);
            (accounts, claude_too_old(&inner.tools).is_some())
        };
        for a in accounts {
            let change = self.hook_change(&a.id);
            let _one = change
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(a) = self.lock().settings.account_in(&self.env, &a.id) else {
                continue;
            };
            let (route, status) = (a.id == store::MAIN_ID, a.enabled);
            if (route || status) && a.provider == Provider::Claude && too_old {
                continue;
            }
            let (old_route, old_status) = hooks::outdated(a.provider, &a.folder, &a.id, &ours);
            let setup = hooks::setup(a.provider, &a.folder, &a.id, &ours);
            let unwanted = (!route && (old_route || setup.routing != Setup::NotInstalled))
                || (!status && (old_status || setup.status != Setup::NotInstalled));
            if !(unwanted || (route && old_route) || (status && old_status)) {
                continue;
            }
            match self.apply_hooks(&a, route, status) {
                Ok(()) => {
                    (self.log)(&format!("{}: updated Warden's hooks", a.label));
                }
                Err(e) => (self.log)(&format!("{}: could not update the hooks: {}", a.label, e.1)),
            }
        }
    }

    /// The shell a Claude account's Claude Code runs commands in on this computer, for the routing guidance's
    /// preview; the default for Codex.
    fn claude_shell(&self, provider: Provider, folder: &Path) -> hooks::ClaudeShell {
        match provider {
            Provider::Claude => hooks::claude_shell(folder, &self.env),
            Provider::Codex => hooks::ClaudeShell::default(),
        }
    }

    /// Writes one account's hooks as wanted, taking out the others.
    fn apply_hooks(&self, a: &AccountConfig, route: bool, status: bool) -> ApiResult<()> {
        if self.env.hooks_off() {
            (self.log)(&format!(
                "{}: hooks off for this run (routing {route}, status {status})",
                a.label
            ));
            return Ok(());
        }
        let tool = self.hook_tool();
        if (route || status) && tool.is_none() {
            return Err(ApiError(
                500,
                "Warden's hook tool (warden.exe) was not found beside the server.".into(),
            ));
        }
        if (route || status)
            && a.provider == Provider::Claude
            && let Some(found) = claude_too_old(&self.lock().tools)
        {
            return Err(ApiError(
                409,
                format!(
                    "Claude Code {found} is too old for Warden's hooks; update it to {} or later.",
                    hooks::MIN_CLAUDE_VERSION
                ),
            ));
        }
        let want = Want {
            tool,
            route,
            status,
        };
        let result = hooks::sync(a.provider, &a.folder, &a.id, &self.data_dir, &want);
        // Read back from the files, whatever was written: the watcher would see it too, but not before the answer.
        self.refresh_hook_setup(&a.id);
        let mut inner = self.lock();
        self.touch(&mut inner);
        result.map_err(|e| {
            (self.log)(&format!(
                "{}: could not write the hooks ({})",
                a.label,
                e.kind()
            ));
            ApiError(
                500,
                format!("Warden could not write the hooks in {}.", a.label),
            )
        })
    }

    pub fn restart_setup(&self) -> ApiResult<()> {
        let mut inner = self.lock();
        inner.settings.setup_done = false;
        self.save_settings(&mut inner)
    }
}

/// The data folder's server lock. A `warden refresh` holding it is waited for (it takes seconds); another server
/// holding it is not.
fn wait_for_lock(data_dir: &Path) -> io::Result<store::ServerLock> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        match store::hold_server_lock(data_dir) {
            Ok(lock) => return Ok(lock),
            Err(e) if store::is_held(&e) && std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) if store::is_held(&e) => {
                return Err(io::Error::new(
                    e.kind(),
                    "another Warden server is already using this data folder",
                ));
            }
            Err(e) => return Err(e),
        }
    }
}

/// The Claude CLI's version when it is known and older than Claude's hooks need.
fn claude_too_old(tools: &[Tool]) -> Option<String> {
    tools
        .iter()
        .find(|t| t.provider == Provider::Claude && t.kind == detect::ToolKind::Cli)
        .and_then(|t| t.version.clone())
        .filter(|v| !detect::version_at_least(v, hooks::MIN_CLAUDE_VERSION))
}

/// The email an account's folder is signed in as instead of the account; "someone else" when it is not known.
fn signed_in_as_text(reading: Option<&AccountReading>) -> String {
    reading
        .and_then(|r| r.identity_change.as_ref())
        .and_then(|c| c.now.as_ref())
        .map_or_else(|| "someone else".to_owned(), |i| i.email.clone())
}

fn clean_label(label: &str) -> ApiResult<String> {
    let l = label.trim();
    if l.is_empty() {
        return Err(ApiError::bad("Give the account a name."));
    }
    if l.chars().count() > 60 {
        return Err(ApiError::bad("Keep the name under 60 characters."));
    }
    Ok(l.to_owned())
}

/// How a usage error is written in the log.
/// Using a reset is refused while the folder is signed in as someone other than its account.
fn signed_in_as_someone_else() -> ApiError {
    ApiError(
        409,
        "The folder is signed in as someone else. Accept or undo that in Settings first.".into(),
    )
}

/// Using a reset is refused while who the folder is signed in as cannot be read.
fn identity_unchecked() -> ApiError {
    ApiError(
        409,
        "Warden could not check who the folder is signed in as. Try again.".into(),
    )
}

/// A new id for one use of a reset: a random UUID (version 4), as the Claude CLI makes one.
fn new_request_id() -> ApiResult<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| ApiError(500, "Warden could not make a request id.".into()))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

fn describe(e: &UsageError) -> String {
    match e {
        UsageError::SignedOut => "signed out".into(),
        UsageError::LoginExpired { status } => match status {
            Some(status) => format!("HTTP {status}: login expired, waiting for it to be renewed"),
            None => "login expired, waiting for it to be renewed".into(),
        },
        UsageError::IdentityChanged => "signed in as someone else".into(),
        UsageError::NoLogs => "no session logs yet".into(),
        UsageError::Failed { message } => message.clone(),
    }
}

/// Watches the Codex accounts' folders and reads their session logs as they grow; rechecks identity when the
/// login file changes. Runs until the server stops; starts over when the set of Codex accounts changes.
pub fn watch_codex(app: &Arc<App>) {
    use notify::{RecursiveMode, Watcher};
    while !app.stopped() {
        let (generation, folders) = app.codex_folders();
        // Watch first, then read where the logs stand, so nothing appended in between is missed.
        let (tx, rx) = std::sync::mpsc::channel();
        *app.codex_wake
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(tx.clone());
        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w,
            Err(e) => {
                (app.log)(&format!("could not watch the Codex logs: {e}"));
                return;
            }
        };
        for (_, folder) in &folders {
            if let Err(e) = watcher.watch(folder, RecursiveMode::Recursive) {
                (app.log)(&format!("could not watch {}: {e}", folder.display()));
            }
        }
        let mut tails: HashMap<String, LogTail> = HashMap::new();
        // Each folder's session names, read from where the index was last read.
        let mut indexes: HashMap<String, titles::CodexIndex> = HashMap::new();
        for (id, folder) in &folders {
            let mut index = titles::CodexIndex::default();
            app.name_sessions(Provider::Codex, index.read(folder));
            indexes.insert(id.clone(), index);
            // Who is signed in first: a login changed while Warden was stopped must not have the new identity's
            // logs read into the old account.
            app.check_codex_login(id, folder);
            let mut tail = LogTail::new();
            match tail.start(folder) {
                Some(u) => app.apply_codex_usage(id, u),
                None => {
                    // Signed out (just checked) stays the reason; otherwise there are no logs yet.
                    let keep = app.lock().readings.accounts.get(id).is_some_and(|r| {
                        r.usage.is_some() || r.error == Some(UsageError::SignedOut)
                    });
                    if !keep {
                        app.apply_usage(id, Err(UsageError::NoLogs));
                    }
                }
            }
            tails.insert(id.clone(), tail);
        }
        while !app.stopped() && app.codex_generation() == generation {
            // Blocks until a file changes or the accounts change: no polling.
            let event = match rx.recv() {
                Ok(Ok(event)) => event,
                Ok(Err(_)) => continue,
                Err(_) => break,
            };
            if event.kind.is_access() {
                continue;
            }
            for path in &event.paths {
                let Some((id, folder)) = folders.iter().find(|(_, f)| path.starts_with(f)) else {
                    continue;
                };
                if path.file_name().is_some_and(|n| n == "auth.json") {
                    // Removed too: signing out deletes the login file.
                    app.check_codex_login(id, folder);
                } else if event.kind.is_remove() {
                    continue;
                } else if path.parent() == Some(folder.as_path())
                    && path.file_name().is_some_and(|n| n == titles::CODEX_INDEX)
                    && let Some(index) = indexes.get_mut(id)
                {
                    app.name_sessions(Provider::Codex, index.read(folder));
                } else if LogTail::is_session_log(path)
                    && let Some(tail) = tails.get_mut(id)
                {
                    if let Ok(Some(u)) = tail.read_appended(path) {
                        app.apply_codex_usage(id, u);
                    }
                    tail.forget_older_than(64);
                }
            }
        }
    }
}

/// The files an account's hooks are written in, which the hook file watcher follows.
fn is_hook_file(provider: Provider, name: &std::ffi::OsStr) -> bool {
    hooks::files_of(provider)
        .iter()
        .any(|f| name.eq_ignore_ascii_case(f))
}

/// Watches every account's folder (not its subfolders) for changes to the files its hooks are written in, and reads
/// where that account's hooks stand again, so a hook added, changed or taken out by hand, or a file that appears or
/// goes, shows at once. The folder above each one is watched too (not its other subfolders), and a folder that is
/// not there is waited for in the nearest folder above it that is, so a folder that goes or comes back starts the
/// watch over. A Claude account's `projects` folder is watched with what is in it, for the sessions' titles. Runs
/// until the server stops; starts over when the set of accounts changes.
/// What the hook file watcher watches: each account folder that is there, and the nearest folder above it that is,
/// to see it go or come.
fn watch_targets(
    folders: &[(String, Provider, PathBuf)],
    identity_files: &[Vec<PathBuf>],
) -> Vec<PathBuf> {
    let mut targets: Vec<PathBuf> = Vec::new();
    for ((_, _, folder), files) in folders.iter().zip(identity_files) {
        let identity_parents = files
            .iter()
            .filter_map(|file| file.ancestors().skip(1).find(|p| p.is_dir()));
        let here = folder.is_dir().then_some(folder.as_path());
        let above = folder.ancestors().skip(1).find(|p| p.is_dir());
        for at in here.into_iter().chain(above).chain(identity_parents) {
            if !targets.iter().any(|w| detect::same_path(w, at)) {
                targets.push(at.to_owned());
            }
        }
    }
    targets
}

pub fn watch_hook_files(app: &Arc<App>) {
    use notify::{RecursiveMode, Watcher};
    while !app.stopped() {
        let (generation, folders) = app.account_folders();
        let identity_files: Vec<Vec<PathBuf>> = folders
            .iter()
            .map(|(_, provider, folder)| match provider {
                Provider::Claude => claude::settings_files(folder, app.env.home.as_deref()),
                Provider::Codex => vec![folder.join("auth.json")],
            })
            .collect();
        // Watch first, then read where the hooks stand, so nothing changed in between is missed.
        let (tx, rx) = std::sync::mpsc::channel();
        *app.hooks_wake
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(tx.clone());
        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w,
            Err(e) => {
                (app.log)(&format!("could not watch the hook files: {e}"));
                return;
            }
        };
        let targets = watch_targets(&folders, &identity_files);
        for at in &targets {
            if let Err(e) = watcher.watch(at, RecursiveMode::NonRecursive) {
                (app.log)(&format!("could not watch {}: {e}", at.display()));
            }
        }
        // A folder that came or went while the watches were being set up sent no event to them: start over, now
        // that they are in place.
        if watch_targets(&folders, &identity_files) != targets {
            continue;
        }
        // Each Claude account's transcripts, so a session renamed between its hook events shows its new name. Only
        // transcripts a session's events named are read, from where they were last read.
        let transcript_roots: Vec<PathBuf> = folders
            .iter()
            .filter(|(_, p, _)| *p == Provider::Claude)
            .map(|(_, _, f)| f.join("projects"))
            .filter(|r| r.is_dir())
            .collect();
        for root in &transcript_roots {
            if let Err(e) = watcher.watch(root, RecursiveMode::Recursive) {
                (app.log)(&format!("could not watch {}: {e}", root.display()));
            }
        }
        for ((id, provider, folder), files) in folders.iter().zip(&identity_files) {
            let now = match provider {
                Provider::Claude => claude::read_identity_from_files(files),
                Provider::Codex => codex::read_identity_from_login_file(folder),
            };
            app.apply_identity(id, now);
            app.refresh_hook_setup(id);
        }
        let mut rewatch = false;
        while !rewatch && !app.stopped() && app.folders_generation() == generation {
            // Blocks until a file changes or the accounts change: no polling.
            let event = match rx.recv() {
                Ok(Ok(event)) => event,
                Ok(Err(_)) => continue,
                Err(_) => break,
            };
            if event.kind.is_access() {
                continue;
            }
            // Windows also reports a folder as modified when what is in it changes: only coming, going and renames
            // of the folders themselves start the watch over.
            let moved = event.kind.is_create()
                || event.kind.is_remove()
                || matches!(
                    event.kind,
                    notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                );
            for path in &event.paths {
                // An account folder (or a folder above a missing one) appeared, went or was renamed: watch again.
                if moved
                    && folders
                        .iter()
                        .any(|(_, _, f)| f.ancestors().any(|a| detect::same_path(a, path)))
                {
                    rewatch = true;
                }
                // A Claude account's transcripts folder appeared, went or was renamed: watch again.
                if moved
                    && folders.iter().any(|(_, p, f)| {
                        *p == Provider::Claude && detect::same_path(&f.join("projects"), path)
                    })
                {
                    rewatch = true;
                }
                if !event.kind.is_remove()
                    && path.extension().is_some_and(|e| e == "jsonl")
                    && transcript_roots.iter().any(|r| path.starts_with(r))
                {
                    app.transcript_changed(path);
                }
                let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
                    continue;
                };
                for ((id, provider, folder), files) in folders.iter().zip(&identity_files) {
                    if files.iter().any(|file| detect::same_path(path, file)) {
                        let now = match provider {
                            Provider::Claude => claude::read_identity_from_files(files),
                            Provider::Codex => codex::read_identity_from_login_file(folder),
                        };
                        app.apply_identity(id, now);
                    }
                    if is_hook_file(*provider, name) && detect::same_path(dir, folder) {
                        app.refresh_hook_setup(id);
                    }
                }
            }
        }
    }
}
