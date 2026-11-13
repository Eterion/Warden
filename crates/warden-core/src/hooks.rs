//! Installing and removing Warden's two hooks in an account folder. Both run only `warden.exe`.
//!
//! - The routing hook: one group of two command handlers on `UserPromptSubmit`: `warden route`, whose output reaches
//!   the model, and `warden refresh --if-stale` as an async handler (the CLI does not wait for it), which reads usage
//!   again when the saved readings are old.
//! - The status hook: one async `warden event` handler on each session event ([`status_events`]), which keeps only
//!   what [`crate::pipe::HookEvent`] keeps and sends it to the core over the per-user pipe.
//!
//! Claude: the groups in the folder's `settings.json`, each handler in Claude Code's exec form (`command` the program,
//! `args` its arguments, run with no shell; [`MIN_CLAUDE_VERSION`] and later). Codex: the groups in the folder's
//! `hooks.json`, run by `cmd.exe /C` (Codex has no argument-list form), and a trust entry per handler in `config.toml`
//! (`[hooks.state.'<folder>\hooks.json:<event>:<group>:<handler>']` with `trusted_hash`), in the format Orca writes
//! and Codex checks. Without them Codex skips the hooks silently.
//!
//! [`sync`] writes the hooks wanted and takes out the others in one go, so the groups' places (which Codex's trust
//! keys name) stay consistent; a user's own Codex hook that moves with it keeps its trust ([`trust_moves`]). Every
//! file is backed up in Warden's data folder (`hooks/<account id>.json`) before Warden first writes it. With neither
//! hook wanted, the file is put back byte for byte when nobody changed it since Warden wrote it (deleted when Warden
//! created it); otherwise only Warden's own entries are taken out.

use crate::model::Provider;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The hook event the routing hook runs on: every message the user sends (it prints only when something changed).
pub const ROUTE_EVENT: &str = "UserPromptSubmit";
/// Seconds the CLIs give `warden route` before giving up on it.
pub const ROUTE_TIMEOUT_SEC: u64 = 10;
/// Seconds the CLIs give the background refresh (one usage request per account).
pub const REFRESH_TIMEOUT_SEC: u64 = 60;
/// Seconds the CLIs give the status hook (`warden event` itself gives up after one); it runs in the background.
pub const STATUS_TIMEOUT_SEC: u64 = 5;
/// The first Claude Code that runs a hook's program directly (`args`, the exec form): 2.1.139. On an older Claude
/// CLI Warden writes no Claude hooks and the Hooks page says why.
pub const MIN_CLAUDE_VERSION: &str = "2.1.139";

/// The events Claude's status hook runs on: session start and end, each prompt, each tool use, permission requests,
/// a turn stopping, and subagents starting and stopping.
const CLAUDE_STATUS_EVENTS: [&str; 9] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Stop",
    "SubagentStart",
    "SubagentStop",
    "SessionEnd",
];
/// Codex's: the same less session end, which Codex does not have.
const CODEX_STATUS_EVENTS: [&str; 8] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Stop",
    "SubagentStart",
    "SubagentStop",
];

/// The events the status hook runs on for a provider.
pub fn status_events(provider: Provider) -> &'static [&'static str] {
    match provider {
        Provider::Claude => &CLAUDE_STATUS_EVENTS,
        Provider::Codex => &CODEX_STATUS_EVENTS,
    }
}

const CLAUDE_SETTINGS: &str = "settings.json";
const CODEX_HOOKS: &str = "hooks.json";
const CODEX_CONFIG: &str = "config.toml";
/// The parts of a hook's command line that mark it as one of the routing hook's handlers.
const MARKS: [&str; 2] = [" route --account ", " refresh --if-stale "];
/// The part of a hook's command line that marks it as the status hook.
const STATUS_MARK: &str = " event --account ";
/// The part of an earlier build's status hook (`curl` to the local server), still recognised so it can be repaired
/// or removed.
const OLD_STATUS_MARK: &str = "/api/events?account=";

/// Where one of an account's hooks stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Setup {
    /// In place and run (for Codex: trusted).
    Installed,
    /// Codex only: in `hooks.json`, but Codex has not trusted it, so it skips it.
    NotTrusted,
    NotInstalled,
}

/// Where both of an account's hooks stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Setups {
    pub routing: Setup,
    pub status: Setup,
}

/// What Warden's hooks run: this Warden's `warden.exe`, with its data folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookTool {
    pub exe: PathBuf,
    pub data_dir: PathBuf,
    #[cfg(debug_assertions)]
    pub home: Option<PathBuf>,
}

/// The hooks wanted in an account, and what they run (needed when either is wanted).
#[derive(Debug, Clone, Default)]
pub struct Want {
    pub tool: Option<HookTool>,
    pub route: bool,
    pub status: bool,
}

/// The shell Claude Code runs the AI's commands in on Windows, for the routing guidance's commands (hooks run with no
/// shell).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClaudeShell {
    /// Git Bash (`bash -c`), whenever Claude Code finds it.
    #[default]
    Bash,
    /// PowerShell, when it does not.
    PowerShell,
}

/// The variable that points Claude Code at Git Bash, in its environment or in the account's `settings.json` `env`.
const GIT_BASH_VAR: &str = "CLAUDE_CODE_GIT_BASH_PATH";

/// The shell Claude Code will run this account's commands in, decided as Claude Code decides it: Git Bash when it
/// finds one, otherwise PowerShell. It looks at `CLAUDE_CODE_GIT_BASH_PATH` (the account's `settings.json` `env`
/// first, then the environment; used when it names an existing `bash.exe`, `sh.exe`, `bash` or `sh`), then
/// `Git\bin\bash.exe` in the Program Files folders ([`crate::detect::claude_program_files`]), then `bin\bash.exe`
/// beside the folder of the first `git` on PATH (or of the Git it belongs to). Only files are looked at; nothing is
/// started. The routing guidance writes its commands for this shell.
pub fn claude_shell(folder: &Path, env: &crate::detect::Env) -> ClaudeShell {
    let from_settings = read_text(&folder.join(CLAUDE_SETTINGS))
        .ok()
        .flatten()
        .and_then(|t| parse_json(Some(&t)).ok())
        .and_then(|v| v["env"][GIT_BASH_VAR].as_str().map(PathBuf::from))
        .filter(|p| !p.as_os_str().is_empty());
    let named = from_settings.or_else(|| env.claude_git_bash.clone());
    let named_ok = named.is_some_and(|p| {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        ["bash.exe", "sh.exe", "bash", "sh"].contains(&name.as_str()) && p.exists()
    });
    let installed = || {
        env.program_files
            .iter()
            .any(|d| d.join("Git").join("bin").join("bash.exe").exists())
    };
    // `where git`: the first folder on PATH with a `git` program, in PATHEXT's default order. Claude Code finds it in
    // Git's `cmd` folder; `warden route`, run by Claude Code, may see a PATH where Git's own copy in
    // `<Git>\<environment>\bin` (`mingw64`, `ucrt64`, ...) comes first: that is the same Git, so its `bin\bash.exe`
    // is two folders up.
    let beside_git = || {
        env.path
            .iter()
            .find(|d| {
                ["git.com", "git.exe", "git.bat", "git.cmd"]
                    .iter()
                    .any(|n| d.join(n).is_file())
            })
            .is_some_and(|d| {
                let up = d.parent().unwrap_or(d);
                let msys = d.file_name().is_some_and(|n| n.eq_ignore_ascii_case("bin"))
                    && up.file_name().is_some_and(|n| {
                        ["mingw64", "mingw32", "ucrt64", "clang64", "clangarm64"]
                            .iter()
                            .any(|e| n.eq_ignore_ascii_case(e))
                    });
                let root = if msys { up.parent().unwrap_or(up) } else { up };
                root.join("bin").join("bash.exe").exists()
            })
    };
    if named_ok || installed() || beside_git() {
        ClaudeShell::Bash
    } else {
        ClaudeShell::PowerShell
    }
}

/// A path as a Codex hook command writes it: forward slashes, which `cmd.exe` passes on as they are.
fn slashed(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn plain(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.:/~-".contains(c))
}

/// One command handler of a Warden hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handler {
    /// Claude: the program (`warden.exe`). Codex: the whole command line for `cmd.exe /C`.
    pub command: String,
    /// Claude's exec form: the arguments, passed to the program as they are, with no shell. `None` for Codex.
    pub args: Option<Vec<String>>,
    pub timeout: u64,
    pub is_async: bool,
}

impl Handler {
    /// The command line it runs, for recognising Warden's handlers: the program and its arguments.
    pub fn line(&self) -> String {
        match &self.args {
            Some(args) => format!("{} {}", self.command, args.join(" ")),
            None => self.command.clone(),
        }
    }
}

/// One of Warden's handlers for an account: `warden.exe <sub...> --account <id> --data-dir <data folder>`. Claude:
/// the exec form, `warden.exe` with its arguments, run with no shell. Codex runs hooks in `cmd.exe /C` (`%COMSPEC%`,
/// its hook runner's default shell): plain paths are written as they are, others in double quotes, which Windows
/// paths never contain.
fn handler(
    provider: Provider,
    tool: &HookTool,
    sub: &[&str],
    account_id: &str,
    timeout: u64,
    is_async: bool,
) -> Handler {
    let (command, args) = match provider {
        Provider::Claude => {
            let mut args: Vec<String> = sub.iter().map(|s| (*s).to_owned()).collect();
            args.extend([
                "--account".to_owned(),
                account_id.to_owned(),
                "--data-dir".to_owned(),
                tool.data_dir.to_string_lossy().into_owned(),
            ]);
            #[cfg(debug_assertions)]
            if sub
                .first()
                .is_some_and(|s| matches!(*s, "route" | "refresh"))
                && let Some(home) = &tool.home
            {
                args.extend(["--home".into(), home.to_string_lossy().into_owned()]);
            }
            (tool.exe.to_string_lossy().into_owned(), Some(args))
        }
        Provider::Codex => {
            let q = |p: &Path| {
                let s = slashed(p);
                if plain(&s) { s } else { format!("\"{s}\"") }
            };
            let id = if plain(account_id) {
                account_id.to_owned()
            } else {
                format!("\"{account_id}\"")
            };
            let line = format!(
                "{} {} --account {id} --data-dir {}",
                q(&tool.exe),
                sub.join(" "),
                q(&tool.data_dir)
            );
            #[cfg(debug_assertions)]
            let line = if sub
                .first()
                .is_some_and(|s| matches!(*s, "route" | "refresh"))
                && let Some(home) = &tool.home
            {
                format!("{line} --home {}", q(home))
            } else {
                line
            };
            (line, None)
        }
    };
    Handler {
        command,
        args,
        timeout,
        is_async,
    }
}

/// The routing hook's handlers for an account: `warden route`, then the async refresh.
pub fn handlers(provider: Provider, tool: &HookTool, account_id: &str) -> [Handler; 2] {
    [
        handler(
            provider,
            tool,
            &["route"],
            account_id,
            ROUTE_TIMEOUT_SEC,
            false,
        ),
        handler(
            provider,
            tool,
            &["refresh", "--if-stale"],
            account_id,
            REFRESH_TIMEOUT_SEC,
            true,
        ),
    ]
}

/// The status hook's handler for an account: `warden event` in the background, with a short timeout.
pub fn status_handler(provider: Provider, tool: &HookTool, account_id: &str) -> Handler {
    handler(
        provider,
        tool,
        &["event"],
        account_id,
        STATUS_TIMEOUT_SEC,
        true,
    )
}

/// Whether a hook command line is one of Warden's routing hook handlers (for any account).
fn is_route(line: &str) -> bool {
    MARKS.iter().any(|m| line.contains(m)) && line.to_ascii_lowercase().contains("warden")
}

/// Whether a hook command line is Warden's status hook (for any account), as written now or by an earlier build.
fn is_status(line: &str) -> bool {
    let now = line.contains(STATUS_MARK) && line.to_ascii_lowercase().contains("warden");
    // `curl ` as written by earlier builds.
    let old = (line.starts_with("curl.exe ") || line.starts_with("curl "))
        && line.contains("http://127.0.0.1:")
        && line.contains(OLD_STATUS_MARK);
    now || old
}

/// Whether a hook command line is one of Warden's.
fn is_ours(line: &str) -> bool {
    is_route(line) || is_status(line)
}

/// The groups to write for the hooks wanted: each event with its handlers, in order.
fn groups(provider: Provider, want: &Want, account_id: &str) -> Vec<(&'static str, Vec<Handler>)> {
    let mut out = Vec::new();
    let Some(tool) = &want.tool else {
        return out;
    };
    if want.route {
        out.push((ROUTE_EVENT, handlers(provider, tool, account_id).to_vec()));
    }
    if want.status {
        for event in status_events(provider) {
            out.push((*event, vec![status_handler(provider, tool, account_id)]));
        }
    }
    out
}

// ---- Backups -------------------------------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Ledger {
    /// Where the ledger itself is saved.
    #[serde(skip)]
    at: PathBuf,
    files: Vec<FileRecord>,
    /// Codex trust keys Warden wrote.
    trust_keys: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileRecord {
    path: PathBuf,
    /// The file as it was before Warden wrote it; `None`: it did not exist.
    original: Option<String>,
    /// What Warden last wrote.
    written: String,
}

fn ledger_path(data_dir: &Path, account_id: &str) -> PathBuf {
    data_dir.join("hooks").join(format!("{account_id}.json"))
}

fn load_ledger(data_dir: &Path, account_id: &str) -> io::Result<Ledger> {
    let at = ledger_path(data_dir, account_id);
    let mut ledger: Ledger = match fs::read(&at) {
        Ok(b) => serde_json::from_slice(&b).map_err(io::Error::other)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ledger::default(),
        Err(e) => return Err(e),
    };
    ledger.at = at;
    Ok(ledger)
}

impl Ledger {
    /// Saves the ledger; called before every config write, so a backup always exists before Warden changes a file.
    fn save(&self) -> io::Result<()> {
        if self.files.is_empty() && self.trust_keys.is_empty() {
            return match fs::remove_file(&self.at) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        if let Some(dir) = self.at.parent() {
            fs::create_dir_all(dir)?;
        }
        write_atomic(
            &self.at,
            &serde_json::to_vec_pretty(self).map_err(io::Error::other)?,
        )
    }
}

fn read_text(path: &Path) -> io::Result<Option<String>> {
    match fs::read(path) {
        Ok(b) => String::from_utf8(b)
            .map(Some)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "the file is not text")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Replaces a file (or, with `None`, deletes it) only if it still holds `read`, what Warden read and worked from: a
/// change the CLI or an editor made since is not overwritten. `Interrupted` then, and [`sync`] starts over from the
/// file as it is now. The check and the replacement are two steps: a write landing in the moment between them is
/// still lost, since the CLIs take no lock Warden could wait for.
fn replace(path: &Path, read: Option<&str>, new: Option<&[u8]>) -> io::Result<()> {
    if read_text(path)?.as_deref() != read {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "the file changed while Warden was writing it",
        ));
    }
    match new {
        Some(bytes) => crate::store::write_atomic(path, bytes),
        None => fs::remove_file(path),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    crate::store::write_atomic(path, bytes)
}

/// Writes Warden's version of a file, keeping the backup of what was there before Warden first touched it (or of
/// what the user made of it since).
fn write_ours(
    ledger: &mut Ledger,
    path: &Path,
    current: Option<&str>,
    stripped: Option<String>,
    new: String,
) -> io::Result<()> {
    let i = ledger.files.iter().position(|f| f.path == path);
    let untouched = i.is_some_and(|i| current == Some(ledger.files[i].written.as_str()));
    if !untouched {
        // Changed since Warden wrote it, or never written: back up what is there now, without Warden's entries.
        let original = stripped.or_else(|| current.map(str::to_owned));
        let record = FileRecord {
            path: path.to_owned(),
            original,
            written: String::new(),
        };
        match i {
            Some(i) => ledger.files[i] = record,
            None => ledger.files.push(record),
        }
    }
    if current == Some(new.as_str()) {
        return Ok(());
    }
    // The backup and what is about to be written are saved first: if the write fails or Warden stops in between,
    // removal still finds the original (the file then differs from `written`, so only Warden's entries come out).
    if let Some(f) = ledger.files.iter_mut().find(|f| f.path == path) {
        f.written = new.clone();
    }
    ledger.save()?;
    replace(path, current, Some(new.as_bytes()))
}

/// Puts a file back: byte for byte when untouched since Warden wrote it, otherwise without Warden's entries.
fn restore(
    ledger: &mut Ledger,
    path: &Path,
    strip: impl FnOnce(&str) -> io::Result<Option<String>>,
) -> io::Result<()> {
    let current = read_text(path)?;
    let i = ledger.files.iter().position(|f| f.path == path);
    let record = i.map(|i| &ledger.files[i]);
    let result = match (record, current.as_deref()) {
        (Some(r), Some(cur)) if cur == r.written => {
            replace(path, Some(cur), r.original.as_ref().map(String::as_bytes))
        }
        (_, Some(cur)) => match strip(cur)? {
            Some(new) => replace(path, Some(cur), Some(new.as_bytes())),
            None => Ok(()),
        },
        (_, None) => Ok(()),
    };
    // The backup is dropped only once the file is back.
    if result.is_ok()
        && let Some(i) = i
    {
        ledger.files.remove(i);
    }
    result
}

// ---- JSON hook files (Claude's settings.json, Codex's hooks.json) ----------------------------------------------

/// How a JSON file is written: two-space indentation as both CLIs write it, the file's own line endings and final
/// newline.
fn to_text(value: &Value, like: Option<&str>) -> io::Result<String> {
    let mut text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    let crlf = like.is_some_and(|l| l.contains("\r\n"));
    let final_newline = like.is_none_or(|l| l.ends_with('\n'));
    if crlf {
        text = text.replace('\n', "\r\n");
    }
    if final_newline {
        text.push_str(if crlf { "\r\n" } else { "\n" });
    }
    Ok(text)
}

fn parse_json(text: Option<&str>) -> io::Result<Value> {
    match text.map(|t| t.trim_start_matches('\u{feff}')) {
        None => Ok(Value::Object(Map::new())),
        Some(t) if t.trim().is_empty() => Ok(Value::Object(Map::new())),
        Some(t) => {
            let v: Value = serde_json::from_str(t).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "the file is not valid JSON")
            })?;
            if v.is_object() {
                Ok(v)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the file is not a JSON object",
                ))
            }
        }
    }
}

/// A handler in a JSON hook file as Warden reads it: `None` when it has no command (another hook type).
fn read_handler(h: &Value) -> Option<Handler> {
    let command = h.get("command")?.as_str()?.to_owned();
    let args = match h.get("args") {
        None => None,
        Some(v) => Some(
            v.as_array()?
                .iter()
                .map(|a| a.as_str().map(str::to_owned))
                .collect::<Option<Vec<String>>>()?,
        ),
    };
    Some(Handler {
        command,
        args,
        timeout: h
            .get("timeout")
            .and_then(Value::as_u64)
            .unwrap_or(600)
            .max(1),
        is_async: h.get("async").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// Whether a handler in a JSON hook file is one of Warden's.
fn is_ours_json(h: &Value) -> bool {
    read_handler(h).is_some_and(|h| is_ours(&h.line()))
}

/// Takes Warden's hooks out of `{"hooks": {event: [group]}}`. Returns whether anything was taken out.
fn strip_json(root: &mut Value) -> bool {
    let Some(events) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return false;
    };
    let mut removed = false;
    for groups in events.values_mut() {
        let Some(groups) = groups.as_array_mut() else {
            continue;
        };
        groups.retain_mut(|g| {
            let Some(handlers) = g.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let before = handlers.len();
            handlers.retain(|h| !is_ours_json(h));
            if handlers.len() == before {
                return true;
            }
            removed = true;
            !handlers.is_empty()
        });
    }
    if removed {
        events.retain(|_, g| g.as_array().is_none_or(|a| !a.is_empty()));
        if events.is_empty()
            && let Some(obj) = root.as_object_mut()
        {
            obj.remove("hooks");
        }
    }
    removed
}

/// Adds a hook as a group of its own at the end of the event's list (so the user's own groups keep their places,
/// which Codex's trust keys depend on). Returns the group's index.
fn add_json(root: &mut Value, event: &str, handlers: &[Handler]) -> io::Result<usize> {
    let bad = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "its hooks are not in the expected shape",
        )
    };
    let obj = root.as_object_mut().ok_or_else(bad)?;
    let events = obj
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(bad)?;
    let groups = events
        .entry(event)
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(bad)?;
    let list: Vec<Value> = handlers
        .iter()
        .map(|h| {
            let mut v = serde_json::json!({ "type": "command", "command": h.command });
            if let Some(args) = &h.args {
                v["args"] = serde_json::json!(args);
            }
            v["timeout"] = serde_json::json!(h.timeout);
            if h.is_async {
                v["async"] = Value::Bool(true);
            }
            v
        })
        .collect();
    groups.push(serde_json::json!({ "hooks": list }));
    Ok(groups.len() - 1)
}

/// A handler in a JSON hook file: its event, the handler as written there, and its group and handler index.
struct Found {
    event: String,
    handler: Handler,
    group: usize,
    index: usize,
}

/// Every command handler in a JSON hook file, on every event, with each one's JSON and its group's matcher (for
/// [`trust_moves`]).
fn every_handler(root: &Value) -> Vec<(Found, Value)> {
    let Some(events) = root.get("hooks").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (event, groups) in events {
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for (gi, g) in groups.iter().enumerate() {
            let Some(list) = g.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for (hi, h) in list.iter().enumerate() {
                if let Some(handler) = read_handler(h) {
                    let identity = serde_json::json!([g.get("matcher"), h]);
                    found.push((
                        Found {
                            event: event.clone(),
                            handler,
                            group: gi,
                            index: hi,
                        },
                        identity,
                    ));
                }
            }
        }
    }
    found
}

/// Warden's handlers in a JSON hook file, on every event.
fn find_json(root: &Value) -> Vec<Found> {
    every_handler(root)
        .into_iter()
        .map(|(f, _)| f)
        .filter(|f| is_ours(&f.handler.line()))
        .collect()
}

fn strip_json_text(text: &str) -> io::Result<Option<String>> {
    let mut v = parse_json(Some(text))?;
    if !strip_json(&mut v) {
        return Ok(None);
    }
    to_text(&v, Some(text)).map(Some)
}

/// Writes the groups into a JSON hook file in place of any Warden wrote before. Returns each group's index, and the
/// file's hooks before and after.
fn install_json(
    ledger: &mut Ledger,
    path: &Path,
    groups: &[(&'static str, Vec<Handler>)],
) -> io::Result<(Vec<usize>, Value, Value)> {
    let current = read_text(path)?;
    let mut v = parse_json(current.as_deref())?;
    let before = v.clone();
    let had_ours = strip_json(&mut v);
    let stripped = had_ours
        .then(|| to_text(&v, current.as_deref()))
        .transpose()?;
    let indexes = groups
        .iter()
        .map(|(event, handlers)| add_json(&mut v, event, handlers))
        .collect::<io::Result<Vec<usize>>>()?;
    let new = to_text(&v, current.as_deref())?;
    write_ours(ledger, path, current.as_deref(), stripped, new)?;
    Ok((indexes, before, v))
}

// ---- Codex trust ----------------------------------------------------------------------------------------------

/// Codex's name for an event in trust keys and hashes: `UserPromptSubmit` is `user_prompt_submit`.
fn event_label(event: &str) -> String {
    let mut out = String::with_capacity(event.len() + 4);
    for (i, c) in event.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Codex's trust key for a hook: the hooks file's path, the event, the group and handler index. Codex writes the
/// path as it displays it; both spellings of a Windows path are written, as Orca does.
fn trust_keys(hooks_file: &Path, event: &str, group: usize, handler: usize) -> Vec<String> {
    let path = hooks_file.to_string_lossy();
    let suffix = format!(":{}:{group}:{handler}", event_label(event));
    let mut keys = vec![
        format!("{}{suffix}", path.replace('/', "\\")),
        format!("{}{suffix}", path.replace('\\', "/")),
    ];
    keys.dedup();
    keys
}

/// The hash Codex expects in `trusted_hash`: SHA-256 of the hook's normalised identity as compact JSON with sorted
/// keys (Codex: `hook_hash` and `version_for_toml`; Orca: `computeCodexTrustedHash`).
pub fn codex_trusted_hash(event: &str, h: &Handler) -> String {
    // Keys written in sorted order; `async` and `timeout` as Codex normalises them.
    let identity = serde_json::json!({
        "event_name": event_label(event),
        "hooks": [{ "async": h.is_async, "command": h.command, "timeout": h.timeout.max(1), "type": "command" }],
    });
    let digest = Sha256::digest(identity.to_string().as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

fn toml_key(key: &str) -> String {
    if !key.contains('\'') && !key.contains('\n') {
        format!("'{key}'")
    } else {
        format!("\"{}\"", key.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// The key of a `[hooks.state.<key>]` header line, if the line is one.
fn header_key(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix('[')?.trim_start();
    let rest = rest
        .strip_prefix("hooks")?
        .trim_start()
        .strip_prefix('.')?
        .trim_start();
    let rest = rest
        .strip_prefix("state")?
        .trim_start()
        .strip_prefix('.')?
        .trim_start();
    if let Some(r) = rest.strip_prefix('\'') {
        let end = r.find('\'')?;
        return Some(r[..end].to_owned());
    }
    let r = rest.strip_prefix('"')?;
    let mut key = String::new();
    let mut chars = r.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(key),
            '\\' => match chars.next()? {
                'n' => key.push('\n'),
                't' => key.push('\t'),
                other => key.push(other),
            },
            c => key.push(c),
        }
    }
    None
}

/// Takes the `[hooks.state.<key>]` blocks for these keys out of config.toml's text, each up to the next table.
fn strip_trust(text: &str, keys: &[String]) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut skipping = false;
    let mut removed = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with('[') {
            skipping = header_key(line).is_some_and(|k| keys.contains(&k));
            removed |= skipping;
        }
        if !skipping {
            out.push_str(line);
        }
    }
    if !removed {
        return None;
    }
    // Drop the blank lines left at the end where a block was cut, keeping one final line break.
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let trimmed = out.trim_end_matches(['\r', '\n']).len();
    out.truncate(trimmed);
    if !out.is_empty() {
        out.push_str(eol);
    }
    Some(out)
}

fn append_trust(text: &str, entries: &[(String, String)]) -> String {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = text.to_owned();
    for (key, hash) in entries {
        if !out.is_empty() {
            let blank = format!("{eol}{eol}");
            if !out.ends_with(&blank) {
                out.push_str(if out.ends_with(eol) { eol } else { &blank });
            }
        }
        out.push_str(&format!(
            "[hooks.state.{}]{eol}enabled = true{eol}trusted_hash = \"{hash}\"{eol}",
            toml_key(key)
        ));
    }
    out
}

fn parses(text: &str) -> bool {
    text.parse::<toml::Table>().is_ok()
}

/// Whether config.toml trusts the hook under one of its keys, enabled.
fn trusted(config: &str, keys: &[String], hash: &str) -> bool {
    let Ok(table) = config.parse::<toml::Table>() else {
        return false;
    };
    let Some(state) = table
        .get("hooks")
        .and_then(|h| h.get("state"))
        .and_then(toml::Value::as_table)
    else {
        return false;
    };
    keys.iter().any(|k| {
        state
            .get(k)
            .and_then(toml::Value::as_table)
            .is_some_and(|s| {
                s.get("trusted_hash").and_then(toml::Value::as_str) == Some(hash)
                    && s.get("enabled").and_then(toml::Value::as_bool) != Some(false)
            })
    })
}

/// A trust entry to move: a user's own Codex hook that Warden's change shifted, from its keys (both spellings) to its
/// new ones, spelling for spelling.
type Move = (Vec<String>, Vec<String>);

/// The user's own Codex handlers that a change of `hooks.json` from `before` to `after` shifts: on each event, the
/// n-th handler that is not Warden's before and after is the same hook, and when it is unchanged (the same handler
/// and group matcher) and its place differs, its trust entry moves with it. Codex's trusted hash does not depend on
/// the place, so the moved entry trusts exactly what it trusted before, and nothing new.
fn trust_moves(hooks_file: &Path, before: &Value, after: &Value) -> Vec<Move> {
    let users = |v: &Value| -> Vec<(Found, Value)> {
        every_handler(v)
            .into_iter()
            .filter(|(f, _)| !is_ours(&f.handler.line()))
            .collect()
    };
    let (old, new) = (users(before), users(after));
    let mut moves = Vec::new();
    let events: std::collections::BTreeSet<&str> =
        old.iter().map(|(f, _)| f.event.as_str()).collect();
    for event in events {
        let on = |list: &'_ [(Found, Value)]| -> Vec<(usize, usize, Value)> {
            list.iter()
                .filter(|(f, _)| f.event == event)
                .map(|(f, id)| (f.group, f.index, id.clone()))
                .collect()
        };
        for (a, b) in on(&old).into_iter().zip(on(&new)) {
            if a.2 == b.2 && (a.0, a.1) != (b.0, b.1) {
                moves.push((
                    trust_keys(hooks_file, event, a.0, a.1),
                    trust_keys(hooks_file, event, b.0, b.1),
                ));
            }
        }
    }
    moves
}

/// Moves trust entries in config.toml's text: each `[hooks.state.<old key>]` header is written with its new key, the
/// entry's body kept as it is. An entry already at a key one moves to (left from an earlier arrangement) goes.
/// `None` when nothing moved.
fn move_trust(text: &str, moves: &[Move]) -> Option<String> {
    let renames: Vec<(&String, &String)> = moves
        .iter()
        .flat_map(|(from, to)| from.iter().zip(to.iter()))
        .collect();
    let present: Vec<String> = text.split_inclusive('\n').filter_map(header_key).collect();
    // The keys an entry lands on; an entry already there that is not itself moving away is replaced.
    let landing: Vec<&String> = renames
        .iter()
        .filter(|(from, _)| present.contains(from))
        .map(|(_, to)| *to)
        .collect();
    if landing.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut skipping = false;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with('[') {
            skipping = false;
            if let Some(key) = header_key(line) {
                if let Some((_, to)) = renames.iter().find(|(from, _)| **from == key) {
                    let eol = &line[line.trim_end_matches(['\r', '\n']).len()..];
                    out.push_str(&format!("[hooks.state.{}]{eol}", toml_key(to)));
                    continue;
                }
                skipping = landing.contains(&&key);
            }
        }
        if !skipping {
            out.push_str(line);
        }
    }
    Some(out)
}

/// Writes a trust entry (key and hash) per handler, replacing any Warden wrote before, and moves the user's own
/// entries Warden's change shifted.
fn install_trust(
    ledger: &mut Ledger,
    path: &Path,
    entries: &[(String, String)],
    moves: &[Move],
) -> io::Result<()> {
    let current = read_text(path)?;
    let text = current.as_deref().unwrap_or("");
    let had_valid = parses(text);
    let keys: Vec<String> = entries.iter().map(|(k, _)| k.clone()).collect();
    let mut drop_keys = ledger.trust_keys.clone();
    drop_keys.extend(keys.iter().cloned());
    // Saved with the backup before the write: both the old and the new keys are Warden's until it succeeds.
    ledger.trust_keys = drop_keys.clone();
    // A user's entry about to move sits at a place that may be Warden's from now on: it is the user's, so it is
    // moved, never stripped (its new place, once Warden's, is cleared by the move).
    let sources: Vec<&String> = moves.iter().flat_map(|(from, _)| from.iter()).collect();
    drop_keys.retain(|k| !sources.contains(&k));
    let stripped = strip_trust(text, &drop_keys);
    let base = stripped.clone().unwrap_or_else(|| text.to_owned());
    // The user's entries moved with their hooks, in the backup too: hooks.json's backup has them in their new places.
    let moved = move_trust(&base, moves);
    let stripped = moved.clone().or(stripped);
    let base = moved.unwrap_or(base);
    let new = append_trust(&base, entries);
    if had_valid && !parses(&new) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "adding the trust entry would break config.toml",
        ));
    }
    write_ours(ledger, path, current.as_deref(), stripped, new)?;
    ledger.trust_keys = keys;
    ledger.save()
}

// ---- Install, remove, state ------------------------------------------------------------------------------------

/// The names of the files in an account folder that hold its hooks (and, for Codex, their trust).
pub fn files_of(provider: Provider) -> &'static [&'static str] {
    match provider {
        Provider::Claude => &[CLAUDE_SETTINGS],
        Provider::Codex => &[CODEX_HOOKS, CODEX_CONFIG],
    }
}

fn files(provider: Provider, folder: &Path) -> (PathBuf, Option<PathBuf>) {
    match provider {
        Provider::Claude => (folder.join(CLAUDE_SETTINGS), None),
        Provider::Codex => (folder.join(CODEX_HOOKS), Some(folder.join(CODEX_CONFIG))),
    }
}

/// Writes the hooks wanted into an account folder (with Codex's trust) and takes out the others; with neither
/// wanted, puts the files back as they were. Idempotent. A file someone else changed while it was being written is
/// read again and the whole sync tried again, a few times.
pub fn sync(
    provider: Provider,
    folder: &Path,
    account_id: &str,
    data_dir: &Path,
    want: &Want,
) -> io::Result<()> {
    let mut tries = 1;
    loop {
        match sync_once(provider, folder, account_id, data_dir, want) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted && tries < 4 => tries += 1,
            result => return result,
        }
    }
}

fn sync_once(
    provider: Provider,
    folder: &Path,
    account_id: &str,
    data_dir: &Path,
    want: &Want,
) -> io::Result<()> {
    if (want.route || want.status) && want.tool.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Warden's warden.exe was not found",
        ));
    }
    let groups = groups(provider, want, account_id);
    if groups.is_empty() {
        return remove(provider, folder, account_id, data_dir);
    }
    // cmd.exe expands `%NAME%` even inside double quotes, and nothing escapes it there: such a path would reach
    // `warden` changed, so Codex's hooks are refused rather than written to read the wrong folder.
    if provider == Provider::Codex
        && let Some(tool) = &want.tool
        && [&tool.exe, &tool.data_dir]
            .iter()
            .any(|p| p.to_string_lossy().contains('%'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a path with % in it, which Codex's hook shell (cmd.exe) would expand",
        ));
    }
    if !folder.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the account folder does not exist",
        ));
    }
    let mut ledger = load_ledger(data_dir, account_id)?;
    let (hooks_file, config) = files(provider, folder);
    let result = (|| {
        let (indexes, before, after) = install_json(&mut ledger, &hooks_file, &groups)?;
        if let Some(config) = config {
            let mut entries: Vec<(String, String)> = Vec::new();
            for ((event, handlers), group) in groups.iter().zip(indexes) {
                for (hi, h) in handlers.iter().enumerate() {
                    let hash = codex_trusted_hash(event, h);
                    for k in trust_keys(&hooks_file, event, group, hi) {
                        entries.push((k, hash.clone()));
                    }
                }
            }
            let moves = trust_moves(&hooks_file, &before, &after);
            install_trust(&mut ledger, &config, &entries, &moves)?;
        }
        Ok(())
    })();
    // Saved again either way (a step that wrote nothing has not saved it yet).
    ledger.save()?;
    result
}

/// Takes both hooks (and Codex's trust entries for them) out of an account folder. A user's own Codex hook that
/// moves up as Warden's groups go keeps its trust.
fn remove(provider: Provider, folder: &Path, account_id: &str, data_dir: &Path) -> io::Result<()> {
    let mut ledger = load_ledger(data_dir, account_id)?;
    let (hooks_file, config) = files(provider, folder);
    let before = read_text(&hooks_file)?;
    let mut result = restore(&mut ledger, &hooks_file, strip_json_text);
    if let Some(config) = config {
        // The places as they were and as they are now: when the file was put back byte for byte, its own trust
        // entries fit it again (config.toml goes back with it); otherwise the user's moved hooks take theirs along.
        let after = read_text(&hooks_file).unwrap_or(None);
        let parsed = |t: &Option<String>| t.as_deref().and_then(|t| parse_json(Some(t)).ok());
        let moves = match (parsed(&before), parsed(&after)) {
            (Some(b), Some(a)) => trust_moves(&hooks_file, &b, &a),
            _ => Vec::new(),
        };
        let keys = ledger.trust_keys.clone();
        let r = if moves.is_empty() {
            restore(&mut ledger, &config, |t| Ok(strip_trust(t, &keys)))
        } else {
            let current = read_text(&config)?.unwrap_or_default();
            let sources: Vec<&String> = moves.iter().flat_map(|(from, _)| from.iter()).collect();
            let ours: Vec<String> = keys
                .iter()
                .filter(|k| !sources.contains(k))
                .cloned()
                .collect();
            let base = strip_trust(&current, &ours).unwrap_or_else(|| current.clone());
            let new = move_trust(&base, &moves).unwrap_or(base);
            let r = replace(&config, Some(&current), Some(new.as_bytes()));
            if r.is_ok() {
                ledger.files.retain(|f| f.path != config);
            }
            r
        };
        result = result.and(r);
        if result.is_ok() {
            ledger.trust_keys.clear();
        }
    }
    ledger.save()?;
    result
}

/// What this Warden writes as an account's hooks: `ours`'s tool, both hooks.
fn expected(provider: Provider, ours: &Want, account_id: &str) -> (Vec<Handler>, Option<Handler>) {
    match &ours.tool {
        Some(tool) => (
            handlers(provider, tool, account_id).to_vec(),
            Some(status_handler(provider, tool, account_id)),
        ),
        None => (Vec::new(), None),
    }
}

/// Where the account's hooks stand, read from its files. The routing hook is installed when both its handlers are
/// there, the status hook when it is on every one of its events; for Codex, each of the hook's handlers must also be
/// trusted. Each handler must be the one this Warden writes (`ours`: its `warden.exe` and data folder, the same form
/// and flags): a hook left by another build of Warden runs something else, so it counts as not installed until
/// Warden writes it again.
pub fn setup(provider: Provider, folder: &Path, account_id: &str, ours: &Want) -> Setups {
    let none = Setups {
        routing: Setup::NotInstalled,
        status: Setup::NotInstalled,
    };
    let (hooks_file, config) = files(provider, folder);
    let Ok(Some(text)) = read_text(&hooks_file) else {
        return none;
    };
    let Ok(v) = parse_json(Some(&text)) else {
        return none;
    };
    let found = find_json(&v);
    let config = config.map(|c| read_text(&c).ok().flatten().unwrap_or_default());
    let state = |mine: &[&Found], complete: bool| {
        if !complete {
            return Setup::NotInstalled;
        }
        let Some(config) = &config else {
            return Setup::Installed;
        };
        let all_trusted = mine.iter().all(|f| {
            trusted(
                config,
                &trust_keys(&hooks_file, &f.event, f.group, f.index),
                &codex_trusted_hash(&f.event, &f.handler),
            )
        });
        if all_trusted {
            Setup::Installed
        } else {
            Setup::NotTrusted
        }
    };
    let (route_want, status_want) = expected(provider, ours, account_id);
    let route: Vec<&Found> = found
        .iter()
        .filter(|f| f.event == ROUTE_EVENT && is_route(&f.handler.line()))
        .collect();
    let route_complete = !route_want.is_empty()
        && route_want
            .iter()
            .all(|h| route.iter().any(|f| f.handler == *h));
    let status: Vec<&Found> = found
        .iter()
        .filter(|f| is_status(&f.handler.line()))
        .collect();
    let status_complete = status_want.is_some_and(|h| {
        status_events(provider)
            .iter()
            .all(|e| status.iter().any(|f| f.event == *e && f.handler == h))
    });
    Setups {
        routing: state(&route, route_complete),
        status: state(&status, status_complete),
    }
}

/// Which of Warden's own hooks in an account's files are outdated: written by an earlier build (another
/// `warden.exe` or data folder, another form or flags, `curl` for the status hook) rather than as this Warden writes
/// them now. `(routing, status)`; Warden repairs a hook that is outdated and switched on.
pub fn outdated(provider: Provider, folder: &Path, account_id: &str, ours: &Want) -> (bool, bool) {
    let (hooks_file, _) = files(provider, folder);
    let Some(v) = read_text(&hooks_file)
        .ok()
        .flatten()
        .and_then(|t| parse_json(Some(&t)).ok())
    else {
        return (false, false);
    };
    let (route_want, status_want) = expected(provider, ours, account_id);
    let mut out = (false, false);
    for f in find_json(&v) {
        let line = f.handler.line();
        if is_route(&line) {
            out.0 |= !(f.event == ROUTE_EVENT && route_want.contains(&f.handler));
        } else if is_status(&line) {
            out.1 |= status_want.as_ref() != Some(&f.handler);
        }
    }
    out
}

#[cfg(test)]
mod tests;
