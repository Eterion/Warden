//! The fallbacks: the one place that starts outside programs (`claude`, `codex`, and Explorer for the desktop
//! app's "Open log folder"), and the CLIs' own session lists for accounts no status hook reports.
//!
//! Each CLI call starts a whole process, so it is a rare fallback, never called on a timer. The session lists are
//! read from the files the CLIs keep them in (Claude's `projects`, Codex's `sessions`), only when Warden starts and
//! when usage is refreshed by hand, and only for each session's folder and age: no CLI is started for them. A main
//! session a delegated run names as its parent is looked up by its id the same way, at most once per id per start.
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
/// `max`. Only plain files and folders are walked, as for [`find_session`]. Only each file's age and the session's
/// folder are read; nothing else in the files is kept.
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
            let modified = plain(&f)?.modified().ok()?;
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

/// One session in an account folder's own list, looked up by its id whatever its age: a main session named by a
/// delegated run's parent link, idle for longer than the list's window. Only the session files' usual places are
/// looked in (Claude's `projects/<folder name>/<id>.jsonl`, Codex's `sessions/<year>/<month>/<day>/rollout-*-<id>.jsonl`),
/// only plain files and folders all the way down (no link, junction or other reparse point followed, `projects` and
/// `sessions` themselves included), and only the file's age and the session's folder are read.
/// The Codex app's folder is never looked in.
pub fn find_session(
    env: &crate::detect::Env,
    provider: Provider,
    folder: &Path,
    id: &str,
) -> Option<Listed> {
    if !crate::pipe::valid_session_id(id) || env.in_codex_app(folder) {
        return None;
    }
    let (file, cwd) = match provider {
        Provider::Claude => {
            let name = format!("{id}.jsonl");
            let file = read_dirs(&folder.join("projects"))
                .into_iter()
                .map(|project| project.join(&name))
                .find(|f| plain_file(f))?;
            let cwd = claude_cwd(&file);
            (file, cwd)
        }
        Provider::Codex => {
            let suffix = format!("-{id}.jsonl");
            let mut days = Vec::new();
            for year in read_dirs(&folder.join("sessions")) {
                for month in read_dirs(&year) {
                    days.extend(read_dirs(&month));
                }
            }
            // Newest day first: a session's file is in the day it started.
            days.sort();
            days.iter().rev().find_map(|day| {
                jsonl_files(day)
                    .into_iter()
                    .filter(|f| {
                        f.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(&suffix))
                    })
                    .find_map(|f| match codex_meta(&f) {
                        Some((found, cwd)) if found == id => Some((f, cwd)),
                        _ => None,
                    })
            })?
        }
    };
    let modified = plain(&file)?.modified().ok()?;
    let last_at = i64::try_from(modified.duration_since(UNIX_EPOCH).ok()?.as_millis()).ok()?;
    Some(Listed {
        id: id.to_owned(),
        last_at,
        cwd,
    })
}

/// A file or folder as it is on disk, not followed, when it is neither a link nor any other Windows reparse point
/// (a junction, a mount point, a cloud placeholder): only these are stepped into or read on a session list's walk.
fn plain(path: &Path) -> Option<std::fs::Metadata> {
    let meta = path.symlink_metadata().ok()?;
    if meta.file_type().is_symlink() {
        return None;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return None;
        }
    }
    Some(meta)
}

/// A file that is not a link or other reparse point.
fn plain_file(path: &Path) -> bool {
    plain(path).is_some_and(|m| m.is_file())
}

/// A folder that is not a link or other reparse point.
fn plain_dir(path: &Path) -> bool {
    plain(path).is_some_and(|m| m.is_dir())
}

/// The plain folders ([`plain_dir`]) in a plain folder: a link anywhere on the walk, its start (`projects`,
/// `sessions`) included, is neither listed nor looked into.
fn read_dirs(dir: &Path) -> Vec<PathBuf> {
    if !plain_dir(dir) {
        return Vec::new();
    }
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| plain_dir(p))
                .collect()
        })
        .unwrap_or_default()
}

/// The plain `.jsonl` files ([`plain_file`]) in a plain folder.
fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    if !plain_dir(dir) {
        return Vec::new();
    }
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "jsonl") && plain_file(p))
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
    fn finds_a_session_by_its_id_whatever_its_age() {
        let dir = crate::testdir::Dir::new("find-session");
        let env = crate::detect::Env {
            home: Some(dir.path().join("home")),
            ..Default::default()
        };
        let claude = dir.path().join("claude");
        let project = claude.join("projects").join("C--work-shop");
        std::fs::create_dir_all(&project).expect("dirs");
        std::fs::write(
            project.join("main-1.jsonl"),
            "{\"type\":\"user\",\"cwd\":\"%USERPROFILE%\\\\shop\",\"message\":\"private\"}\n",
        )
        .expect("log");
        // Hours later, far outside the list's window, the named session is still found.
        let later = crate::time::now_ms() + 5 * 60 * 60_000;
        assert!(listed_sessions(Provider::Claude, &claude, later, 60_000, 10).is_empty());
        let found = find_session(&env, Provider::Claude, &claude, "main-1").expect("found");
        assert_eq!(found.id, "main-1");
        assert_eq!(found.cwd, Some(PathBuf::from("%USERPROFILE%\\shop")));
        assert!(found.last_at <= crate::time::now_ms());
        assert_eq!(find_session(&env, Provider::Claude, &claude, "other"), None);
        // Only session ids: nothing outside the session files' places.
        for bad in [
            "",
            "../main-1",
            "projects\\C--work-shop\\main-1",
            "main-1.jsonl",
        ] {
            assert_eq!(find_session(&env, Provider::Claude, &claude, bad), None);
        }

        let codex = dir.path().join("codex");
        let day = codex.join("sessions/2026/10/01");
        std::fs::create_dir_all(&day).expect("dirs");
        std::fs::create_dir_all(codex.join("sessions/2026/11/09")).expect("dirs");
        std::fs::write(
            day.join("rollout-2026-10-01T10-00-00-c1.jsonl"),
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"c1\",\"cwd\":\"/w\"}}\n",
        )
        .expect("log");
        let found = find_session(&env, Provider::Codex, &codex, "c1").expect("found");
        assert_eq!(
            (found.id.as_str(), found.cwd),
            ("c1", Some(PathBuf::from("/w")))
        );
        assert_eq!(find_session(&env, Provider::Codex, &codex, "c2"), None);
    }

    #[test]
    fn never_looks_in_the_codex_apps_folder() {
        let dir = crate::testdir::Dir::new("find-codex-app");
        let home = dir.path().join("home");
        let env = crate::detect::Env {
            home: Some(home.clone()),
            ..Default::default()
        };
        let app = home.join(".codex");
        let day = app.join("sessions/2026/11/09");
        std::fs::create_dir_all(&day).expect("dirs");
        std::fs::write(
            day.join("rollout-2026-11-09T10-00-00-c1.jsonl"),
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"c1\",\"cwd\":\"/w\"}}\n",
        )
        .expect("log");
        let project = app.join("projects").join("p");
        std::fs::create_dir_all(&project).expect("dirs");
        std::fs::write(project.join("c1.jsonl"), "{\"cwd\":\"/w\"}\n").expect("log");
        for folder in [app.clone(), app.join("sessions"), home.join("x/../.codex")] {
            for provider in [Provider::Claude, Provider::Codex] {
                assert_eq!(find_session(&env, provider, &folder, "c1"), None);
            }
        }
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
