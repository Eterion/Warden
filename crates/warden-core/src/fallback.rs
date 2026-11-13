//! The fallbacks: the one place that starts outside programs (`claude`, `codex`, and Explorer for the desktop
//! app's "Open log folder"), and the CLIs' own session lists for accounts no status hook reports.
//!
//! Each CLI call starts a whole process, so it is a rare fallback, never called on a timer. The session lists are
//! read from the files the CLIs keep them in (Claude's `projects`, Codex's `sessions`), only when Warden starts and
//! when usage is refreshed by hand, and only for each session's folder and age: no CLI is started for them.
//! The `cli-fallback-only` rule check fails if a process is started anywhere else in `crates/`.

use crate::model::Provider;
use serde::Deserialize;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

/// What a provider's sign-in runs: the official CLI, pointed at the account folder.
///
/// Claude: `claude auth login --claudeai`, the sign-in command for a Claude subscription (`/login` is a command
/// inside an interactive session, which a fresh folder starts with onboarding, and which refuses to run when its
/// output is not a terminal). Codex: `codex login`. Both open the sign-in in the browser.
pub fn sign_in_args(provider: Provider) -> &'static [&'static str] {
    match provider {
        Provider::Claude => &["auth", "login", "--claudeai"],
        Provider::Codex => &["login"],
    }
}

/// Called once a sign-in's CLI has exited, signed in or not (its window closes then).
pub type Done = Box<dyn FnOnce() + Send>;

/// Starts the official sign-in for an account folder in a window of its own, and returns without waiting for it.
/// When the CLI exits, `done` runs on a thread of its own, so the new login can be read at once.
///
/// `cli` is the CLI found on PATH. Arguments are passed as a list, never through a shell.
pub fn start_sign_in(provider: Provider, cli: &Path, folder: &Path, done: Done) -> io::Result<()> {
    let mut cmd = Command::new(cli);
    cmd.args(sign_in_args(provider))
        .env(provider.folder_var(), folder)
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// A console window of its own, so the sign-in can be seen and answered.
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        cmd.creation_flags(CREATE_NEW_CONSOLE);
        // A new console gets fresh standard handles; null ones would hide the sign-in.
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    }
    let mut child = cmd.spawn()?;
    // Without the thread the sign-in still runs; its first reading then waits for the usage interval.
    let _ = std::thread::Builder::new()
        .name("sign-in".into())
        .spawn(move || {
            if child.wait().is_ok() {
                done();
            }
        });
    Ok(())
}

/// Opens a folder in Windows Explorer (the desktop app's "Open log folder"), without waiting for it. The folder is
/// passed as one argument, never through a shell.
pub fn open_folder(folder: &Path) -> io::Result<()> {
    let explorer = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("explorer.exe"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("explorer.exe"));
    Command::new(explorer)
        .arg(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

/// A session in a CLI's own list: its id, when it was last written to, and the folder it ran in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    pub last_at: i64,
    pub cwd: Option<PathBuf>,
}

/// The sessions in an account folder's own list written to within `within_ms` of `now`, newest first, at most
/// `max`. Only each file's age and the session's folder are read; nothing else in the files is kept.
pub fn listed_sessions(
    provider: Provider,
    folder: &Path,
    now: i64,
    within_ms: i64,
    max: usize,
) -> Vec<Listed> {
    let mut files = Vec::new();
    match provider {
        // `projects/<folder name>/<session id>.jsonl`
        Provider::Claude => {
            for project in read_dirs(&folder.join("projects")) {
                files.extend(jsonl_files(&project));
            }
        }
        // `sessions/<year>/<month>/<day>/rollout-<time>-<id>.jsonl`: the two latest days are enough.
        Provider::Codex => {
            let mut days = Vec::new();
            for year in read_dirs(&folder.join("sessions")) {
                for month in read_dirs(&year) {
                    days.extend(read_dirs(&month));
                }
            }
            days.sort();
            for day in days.iter().rev().take(2) {
                files.extend(jsonl_files(day));
            }
        }
    }
    let mut recent: Vec<(PathBuf, i64)> = files
        .into_iter()
        .filter_map(|f| {
            let modified = f.metadata().ok()?.modified().ok()?;
            let ms = i64::try_from(modified.duration_since(UNIX_EPOCH).ok()?.as_millis()).ok()?;
            (now - ms < within_ms).then_some((f, ms))
        })
        .collect();
    recent.sort_by_key(|r| std::cmp::Reverse(r.1));
    recent.truncate(max);
    recent
        .into_iter()
        .filter_map(|(file, last_at)| {
            let (id, cwd) = match provider {
                Provider::Claude => (
                    file.file_stem()?.to_string_lossy().into_owned(),
                    claude_cwd(&file),
                ),
                Provider::Codex => codex_meta(&file)?,
            };
            Some(Listed { id, last_at, cwd })
        })
        .collect()
}

fn read_dirs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
                .collect()
        })
        .unwrap_or_default()
}

/// Only the folder is read from a line; the rest (messages among it) is skipped by the parser.
#[derive(Deserialize)]
struct CwdLine {
    cwd: Option<String>,
}

/// The longest line looked at, and how many: the folder is on the first few.
const LINE_LIMIT: u64 = 256 * 1024;
const LINES_LOOKED_AT: usize = 20;

fn first_lines(file: &Path) -> impl Iterator<Item = String> {
    std::fs::File::open(file)
        .ok()
        .map(|f| BufReader::new(f.take(LINE_LIMIT * LINES_LOOKED_AT as u64)))
        .into_iter()
        .flat_map(|r| r.lines().take(LINES_LOOKED_AT).map_while(Result::ok))
}

fn claude_cwd(file: &Path) -> Option<PathBuf> {
    first_lines(file).find_map(|line| {
        serde_json::from_str::<CwdLine>(&line)
            .ok()?
            .cwd
            .filter(|c| !c.is_empty())
            .map(PathBuf::from)
    })
}

#[derive(Deserialize)]
struct CodexMetaLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    payload: Option<CodexMeta>,
}

#[derive(Deserialize)]
struct CodexMeta {
    id: Option<String>,
    cwd: Option<String>,
}

fn codex_meta(file: &Path) -> Option<(String, Option<PathBuf>)> {
    let line = first_lines(file).next()?;
    let meta: CodexMetaLine = serde_json::from_str(&line).ok()?;
    if meta.kind.as_deref() != Some("session_meta") {
        return None;
    }
    let payload = meta.payload?;
    Some((
        payload.id?,
        payload.cwd.filter(|c| !c.is_empty()).map(PathBuf::from),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_recent_sessions_from_the_clis_own_files() {
        let dir = crate::testdir::Dir::new("listed");
        let claude = dir.path().join("claude");
        let project = claude.join("projects").join("C--work-shop");
        std::fs::create_dir_all(&project).expect("dirs");
        std::fs::write(
            project.join("s-1.jsonl"),
            "{\"type\":\"summary\"}\n{\"type\":\"user\",\"cwd\":\"%USERPROFILE%\\\\shop\",\"message\":\"private\"}\n",
        )
        .expect("log");
        let now = crate::time::now_ms();
        let listed = listed_sessions(Provider::Claude, &claude, now + 1000, 60_000, 10);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "s-1");
        assert_eq!(listed[0].cwd, Some(PathBuf::from("%USERPROFILE%\\shop")));
        assert!(listed_sessions(Provider::Claude, &claude, now + 120_000, 60_000, 10).is_empty());

        let codex = dir.path().join("codex");
        let day = codex.join("sessions/2026/11/09");
        std::fs::create_dir_all(&day).expect("dirs");
        std::fs::write(
            day.join("rollout-2026-11-09T10-00-00-c1.jsonl"),
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"c1\",\"cwd\":\"/w\"}}\n",
        )
        .expect("log");
        let listed = listed_sessions(Provider::Codex, &codex, now + 1000, 60_000, 10);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "c1");
        assert_eq!(listed[0].cwd, Some(PathBuf::from("/w")));
    }

    #[test]
    fn sign_in_uses_the_official_commands() {
        assert_eq!(
            sign_in_args(Provider::Claude),
            ["auth", "login", "--claudeai"]
        );
        assert_eq!(sign_in_args(Provider::Codex), ["login"]);
    }
}
