//! Sessions' own names: for Claude, the newest `custom-title` record in a session's transcript, or without one its
//! newest `ai-title` (Claude Code's own generated title); for Codex, the `thread_name` lines of `session_index.jsonl`
//! in the account's folder.
//!
//! Both files are read only from where they were last read, when an event or a file-change notification says
//! something may have been added, never on a timer. Only lines holding a title are parsed (others are skipped
//! unparsed, and one longer than [`MAX_LINE`] is never buffered), and only the title is kept from them.

use crate::sessions::on_a_drive;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

/// Codex's list of named sessions, in the account's folder.
pub const CODEX_INDEX: &str = "session_index.jsonl";
/// A line longer than this (a large tool output in a transcript) is skipped rather than buffered; a title's line is
/// far shorter.
const MAX_LINE: usize = 64 * 1024;
/// How much is read at a time.
const READ_CHUNK: usize = 64 * 1024;
/// The longest title kept, in characters.
const MAX_TITLE: usize = 120;
/// Transcripts followed at most (one per live session).
const MAX_TRANSCRIPTS: usize = crate::sessions::MAX_SESSIONS;
/// Codex names handed on from one reading of the index at most (the newest).
pub const MAX_NAMES: usize = 2 * crate::sessions::MAX_SESSIONS;

/// Where reading a growing file of JSON lines stopped.
#[derive(Debug, Default)]
struct Lines {
    offset: u64,
    /// The start of a line whose end has not been read yet (only kept while it may hold a title).
    partial: Vec<u8>,
    /// Inside a line that is skipped: longer than `MAX_LINE`.
    skipping: bool,
}

impl Lines {
    /// Reads what was added to `path` since the last read and calls `found` with each complete line that holds one
    /// of `needles`. A file that got shorter (replaced or rewritten) is read again from its start: then it answers
    /// `true`.
    fn read(
        &mut self,
        path: &Path,
        needles: &[&[u8]],
        mut found: impl FnMut(&[u8]),
    ) -> io::Result<bool> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        let restarted = len < self.offset;
        if restarted {
            *self = Lines::default();
        }
        if len == self.offset {
            return Ok(restarted);
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut reader = file.take(len - self.offset);
        let mut chunk = vec![0u8; READ_CHUNK];
        loop {
            let n = reader.read(&mut chunk)?;
            if n == 0 {
                return Ok(restarted);
            }
            self.offset += n as u64;
            self.feed(&chunk[..n], needles, &mut found);
        }
    }

    fn feed(&mut self, bytes: &[u8], needles: &[&[u8]], found: &mut impl FnMut(&[u8])) {
        let mut rest = bytes;
        while let Some(i) = rest.iter().position(|&b| b == b'\n') {
            let piece = &rest[..i];
            rest = &rest[i + 1..];
            if std::mem::take(&mut self.skipping) {
                self.partial.clear();
                continue;
            }
            if self.partial.is_empty() {
                if contains(piece, needles) {
                    found(piece);
                }
            } else if self.partial.len() + piece.len() <= MAX_LINE {
                self.partial.extend_from_slice(piece);
                let line = std::mem::take(&mut self.partial);
                if contains(&line, needles) {
                    found(&line);
                }
            } else {
                self.partial.clear();
            }
        }
        if !self.skipping {
            if self.partial.len() + rest.len() > MAX_LINE {
                self.partial.clear();
                self.skipping = true;
            } else {
                self.partial.extend_from_slice(rest);
            }
        }
    }
}

fn contains(hay: &[u8], needles: &[&[u8]]) -> bool {
    needles
        .iter()
        .any(|needle| hay.windows(needle.len()).any(|w| w == *needle))
}

/// Keeps a title short and printable, on one line; `None` when nothing is left.
pub fn clean_title(s: &str) -> Option<String> {
    let s: String = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TITLE)
        .collect();
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

/// The record a session's own name is in (`/rename`, `--name`): it always wins over a generated title.
const CUSTOM_TITLE: &[u8] = b"\"custom-title\"";
/// Claude Code's own generated title, used only while a session has no custom one.
const AI_TITLE: &[u8] = b"\"ai-title\"";

/// A title found in a transcript line.
#[derive(Debug, PartialEq, Eq)]
enum ClaudeTitle {
    Custom(String),
    Generated(String),
}

/// The title in one transcript line, if it is a `custom-title` or an `ai-title` record.
fn claude_title(line: &[u8]) -> Option<ClaudeTitle> {
    #[derive(Deserialize)]
    struct Record {
        #[serde(rename = "type")]
        kind: Option<String>,
        #[serde(rename = "customTitle")]
        custom_title: Option<String>,
        #[serde(rename = "aiTitle")]
        ai_title: Option<String>,
    }
    let r: Record = serde_json::from_slice(line).ok()?;
    match r.kind.as_deref()? {
        "custom-title" => clean_title(r.custom_title.as_deref()?).map(ClaudeTitle::Custom),
        "ai-title" => clean_title(r.ai_title.as_deref()?).map(ClaudeTitle::Generated),
        _ => None,
    }
}

/// Whether a transcript path from a hook event is one Warden reads: a `.jsonl` file inside the account's folder, on
/// one of this computer's drives, with no `..` in it and no link on the way to it. (Claude Code keeps transcripts
/// in `<folder>\projects`.)
pub fn is_transcript_of(folder: &Path, path: &Path) -> bool {
    inside(folder, path) && no_links_below(folder, path)
}

/// The checks on the path itself, with nothing read: a `.jsonl` file below the folder, both on a drive, no `..`.
fn inside(folder: &Path, path: &Path) -> bool {
    let fold = |p: &Path| -> Vec<String> {
        p.components()
            .filter(|c| !matches!(c, Component::CurDir))
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect()
    };
    let (f, p) = (fold(folder), fold(path));
    on_a_drive(path)
        && on_a_drive(folder)
        && !path.components().any(|c| c == Component::ParentDir)
        && path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
        && p.len() > f.len()
        && p.starts_with(&f)
}

/// Whether nothing on the way from the folder down to the file (the file included) is a symbolic link, junction or
/// other reparse point, which could lead outside the folder or to a network path. Looks at each entry itself
/// without following it.
fn no_links_below(folder: &Path, path: &Path) -> bool {
    let depth = folder.components().count();
    path.ancestors()
        .take_while(|a| a.components().count() > depth)
        .all(|a| std::fs::symlink_metadata(a).is_ok_and(|m| !is_link(&m)))
}

#[cfg(windows)]
fn is_link(m: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    m.file_type().is_symlink() || m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link(m: &std::fs::Metadata) -> bool {
    m.file_type().is_symlink()
}

/// The Claude transcripts being followed, one per session, each read only from where it was last read.
#[derive(Debug, Default)]
pub struct Transcripts {
    tails: HashMap<String, Tail>,
    uses: u64,
}

#[derive(Debug)]
struct Tail {
    path: PathBuf,
    lines: Lines,
    /// The file has held a custom title: generated titles no longer count.
    has_custom: bool,
    last_use: u64,
}

impl Transcripts {
    /// Reads what was added to a session's transcript since the last time and returns the newest title in it
    /// (`None`: no new title): its newest custom title, else, while the file has held none, its newest generated
    /// one. A session whose transcript moved is read from the new file's start.
    pub fn read(&mut self, session_id: &str, path: &Path) -> Option<String> {
        self.uses += 1;
        if !self.tails.contains_key(session_id) && self.tails.len() >= MAX_TRANSCRIPTS {
            let oldest = self
                .tails
                .iter()
                .min_by_key(|(_, t)| t.last_use)
                .map(|(id, _)| id.clone());
            if let Some(oldest) = oldest {
                self.tails.remove(&oldest);
            }
        }
        let tail = self
            .tails
            .entry(session_id.to_owned())
            .or_insert_with(|| Tail {
                path: path.to_owned(),
                lines: Lines::default(),
                has_custom: false,
                last_use: 0,
            });
        if tail.path != path {
            tail.path = path.to_owned();
            tail.lines = Lines::default();
            tail.has_custom = false;
        }
        tail.last_use = self.uses;
        let (mut custom, mut generated) = (None, None);
        let restarted =
            tail.lines.read(path, &[CUSTOM_TITLE, AI_TITLE], |line| {
                match claude_title(line) {
                    Some(ClaudeTitle::Custom(t)) => custom = Some(t),
                    Some(ClaudeTitle::Generated(t)) => generated = Some(t),
                    None => {}
                }
            });
        // A rewritten file is read from its start and may no longer hold a custom title.
        if restarted.unwrap_or(false) {
            tail.has_custom = false;
        }
        if custom.is_some() {
            tail.has_custom = true;
            return custom;
        }
        generated.filter(|_| !tail.has_custom)
    }

    /// Reads what was added to a followed transcript that changed on disk; `(session id, title)` when it holds a
    /// new title. A file no session follows is not read.
    pub fn changed(&mut self, path: &Path) -> Option<(String, String)> {
        let id = self
            .tails
            .iter()
            .find(|(_, t)| crate::detect::same_path(&t.path, path))
            .map(|(id, t)| (id.clone(), t.path.clone()))?;
        let title = self.read(&id.0, &id.1)?;
        Some((id.0, title))
    }

    /// Stops following a session's transcript.
    pub fn forget(&mut self, session_id: &str) {
        self.tails.remove(session_id);
    }
}

/// Codex's `session_index.jsonl` in one account's folder, read only from where it was last read.
#[derive(Debug, Default)]
pub struct CodexIndex {
    lines: Lines,
}

impl CodexIndex {
    /// The names added to `<codex_home>\session_index.jsonl` since the last read, as `(session id, name)`, the
    /// latest line for a session winning; at most the newest [`MAX_NAMES`]. A missing file gives none.
    pub fn read(&mut self, codex_home: &Path) -> Vec<(String, String)> {
        #[derive(Deserialize)]
        struct Line {
            id: Option<String>,
            thread_name: Option<String>,
        }
        // Each session's latest name with the line it came from; pruned to the newest as it grows, so a long
        // index never needs more than twice `MAX_NAMES`.
        let mut latest: HashMap<String, (u64, String)> = HashMap::new();
        let mut line_no = 0u64;
        let _ = self.lines.read(
            &codex_home.join(CODEX_INDEX),
            &[b"\"thread_name\""],
            |line| {
                let Ok(l) = serde_json::from_slice::<Line>(line) else {
                    return;
                };
                let (Some(id), Some(name)) = (
                    l.id.as_deref().and_then(crate::sessions::clean_name),
                    l.thread_name.as_deref().and_then(clean_title),
                ) else {
                    return;
                };
                line_no += 1;
                latest.insert(id, (line_no, name));
                if latest.len() > 2 * MAX_NAMES {
                    keep_newest(&mut latest);
                }
            },
        );
        keep_newest(&mut latest);
        let mut names: Vec<(u64, String, String)> = latest
            .into_iter()
            .map(|(id, (at, name))| (at, id, name))
            .collect();
        names.sort();
        names.into_iter().map(|(_, id, name)| (id, name)).collect()
    }
}

/// Keeps only the [`MAX_NAMES`] names from the latest lines.
fn keep_newest(latest: &mut HashMap<String, (u64, String)>) {
    if latest.len() <= MAX_NAMES {
        return;
    }
    let mut at: Vec<u64> = latest.values().map(|(n, _)| *n).collect();
    at.sort_unstable();
    let cut = at[at.len() - MAX_NAMES];
    latest.retain(|_, (n, _)| *n >= cut);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::Dir;
    use std::io::Write;

    fn append(path: &Path, text: &str) {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open")
            .write_all(text.as_bytes())
            .expect("write");
    }

    #[test]
    fn the_newest_custom_title_wins_and_only_what_was_added_is_read() {
        let dir = Dir::new("titles-claude");
        let t = dir.path().join("s1.jsonl");
        append(
            &t,
            concat!(
                r#"{"type":"user","message":{"content":"a private prompt about custom-title"}}"#,
                "\n",
                r#"{"type":"custom-title","customTitle":"First name","sessionId":"s1"}"#,
                "\n",
                r#"{"type":"custom-title","customTitle":"  Payroll\nexport  ","sessionId":"s1"}"#,
                "\n",
            ),
        );
        let mut tr = Transcripts::default();
        assert_eq!(tr.read("s1", &t), Some("Payroll export".into()));
        // Nothing new: no title.
        assert_eq!(tr.read("s1", &t), None);
        // A line written in two parts is read once it is whole.
        append(&t, r#"{"type":"custom-title","customTi"#);
        assert_eq!(tr.read("s1", &t), None);
        append(&t, "tle\":\"Renamed\"}\n");
        assert_eq!(tr.read("s1", &t), Some("Renamed".into()));
        // A message quoting a title record is not one.
        append(
            &t,
            &format!(
                "{}\n",
                serde_json::json!({"type":"assistant","text":r#"{"type":"custom-title","customTitle":"Fake"}"#})
            ),
        );
        assert_eq!(tr.read("s1", &t), None);
        // A missing file gives nothing; a rewritten (shorter) one is read again from its start.
        assert_eq!(tr.read("s2", &dir.path().join("none.jsonl")), None);
        std::fs::write(&t, "{\"type\":\"custom-title\",\"customTitle\":\"New\"}\n").expect("write");
        assert_eq!(tr.read("s1", &t), Some("New".into()));
    }

    #[test]
    fn a_generated_title_counts_only_while_there_is_no_custom_one() {
        let dir = Dir::new("titles-ai");
        let t = dir.path().join("s1.jsonl");
        let mut tr = Transcripts::default();
        // An ai-title alone gives the title; the newest wins.
        append(
            &t,
            concat!(
                r#"{"type":"user","message":{"content":"a private prompt about ai-title"}}"#,
                "\n",
                r#"{"type":"ai-title","aiTitle":"Fix the login","sessionId":"s1"}"#,
                "\n",
                r#"{"type":"ai-title","aiTitle":"  Fix the\nlogin page  ","sessionId":"s1"}"#,
                "\n",
            ),
        );
        assert_eq!(tr.read("s1", &t), Some("Fix the login page".into()));
        assert_eq!(tr.read("s1", &t), None);
        // A later custom-title replaces it; a generated one in the same read does not win, before or after it.
        append(
            &t,
            concat!(
                r#"{"type":"ai-title","aiTitle":"Before"}"#,
                "\n",
                r#"{"type":"custom-title","customTitle":"Payroll export"}"#,
                "\n",
                r#"{"type":"ai-title","aiTitle":"After"}"#,
                "\n",
            ),
        );
        assert_eq!(tr.read("s1", &t), Some("Payroll export".into()));
        // A later ai-title, read on its own, does not replace the custom title.
        append(&t, "{\"type\":\"ai-title\",\"aiTitle\":\"Later\"}\n");
        assert_eq!(tr.read("s1", &t), None);
        assert_eq!(tr.changed(&t), None);
        // Read from where it was last read: a line in two parts, of either kind.
        append(&t, r#"{"type":"ai-title","aiTi"#);
        assert_eq!(tr.read("s1", &t), None);
        append(
            &t,
            "tle\":\"Still not\"}\n{\"type\":\"custom-title\",\"cust",
        );
        assert_eq!(tr.read("s1", &t), None);
        append(&t, "omTitle\":\"Renamed\"}\n");
        assert_eq!(tr.read("s1", &t), Some("Renamed".into()));
        // A record of another type, or one quoting a title, is not one.
        append(
            &t,
            &format!(
                "{}\n{}\n",
                serde_json::json!({"type":"assistant","text":r#"{"type":"ai-title","aiTitle":"Fake"}"#}),
                serde_json::json!({"type":"user","aiTitle":"Fake"})
            ),
        );
        assert_eq!(tr.read("s1", &t), None);
        // A rewritten file with only a generated title: that title counts again.
        std::fs::write(&t, "{\"type\":\"ai-title\",\"aiTitle\":\"New\"}\n").expect("write");
        assert_eq!(tr.read("s1", &t), Some("New".into()));
        // Another session's transcript keeps its own: a custom title read first is not replaced.
        let other = dir.path().join("s2.jsonl");
        append(
            &other,
            "{\"type\":\"custom-title\",\"customTitle\":\"Mine\"}\n{\"type\":\"ai-title\",\"aiTitle\":\"Not mine\"}\n",
        );
        assert_eq!(tr.read("s2", &other), Some("Mine".into()));
        append(&other, "{\"type\":\"ai-title\",\"aiTitle\":\"Later\"}\n");
        assert_eq!(tr.changed(&other), None);
    }

    #[test]
    fn an_overlong_line_is_skipped_without_being_buffered() {
        let mut lines = Lines::default();
        let mut found = Vec::new();
        let long = format!("{}\"custom-title\"", "x".repeat(MAX_LINE * 2));
        for piece in long.as_bytes().chunks(READ_CHUNK) {
            lines.feed(piece, &[CUSTOM_TITLE], &mut |l: &[u8]| {
                found.push(l.to_vec())
            });
            assert!(lines.partial.len() <= MAX_LINE);
        }
        lines.feed(
            b"\n{\"custom-title\"}\n",
            &[CUSTOM_TITLE],
            &mut |l: &[u8]| found.push(l.to_vec()),
        );
        assert_eq!(found, vec![b"{\"custom-title\"}".to_vec()]);
    }

    #[test]
    fn codex_names_come_from_the_index_the_latest_line_winning() {
        let home = Dir::new("titles-codex");
        let mut index = CodexIndex::default();
        assert!(index.read(home.path()).is_empty());
        let file = home.path().join(CODEX_INDEX);
        append(
            &file,
            concat!(
                r#"{"id":"a","thread_name":"Fix login","updated_at":"2026-11-10T08:00:00Z"}"#,
                "\n",
                r#"{"id":"b","thread_name":"Docs","updated_at":"2026-11-10T08:01:00Z"}"#,
                "\n",
                r#"{"id":"a","thread_name":"Fix login 55402","updated_at":"2026-11-10T08:02:00Z"}"#,
                "\n",
                r#"{"id":"c"}"#,
                "\n",
            ),
        );
        assert_eq!(
            index.read(home.path()),
            vec![
                ("b".into(), "Docs".into()),
                ("a".into(), "Fix login 55402".into())
            ]
        );
        assert!(index.read(home.path()).is_empty());
        append(&file, "{\"id\":\"b\",\"thread_name\":\"Docs v2\"}\n");
        assert_eq!(
            index.read(home.path()),
            vec![("b".into(), "Docs v2".into())]
        );
        // A long index gives only the newest names, in order.
        let many: String = (0..MAX_NAMES * 3)
            .map(|i| format!("{{\"id\":\"n{i}\",\"thread_name\":\"N {i}\"}}\n"))
            .collect();
        append(&file, &many);
        let names = index.read(home.path());
        assert_eq!(names.len(), MAX_NAMES);
        assert_eq!(names[0].0, format!("n{}", MAX_NAMES * 2));
        assert_eq!(names[MAX_NAMES - 1].0, format!("n{}", MAX_NAMES * 3 - 1));
    }

    #[test]
    fn a_followed_transcript_that_changes_is_read_and_others_are_not() {
        let dir = Dir::new("titles-changed");
        let t = dir.path().join("s1.jsonl");
        append(&t, "{\"type\":\"user\"}\n");
        let mut tr = Transcripts::default();
        assert_eq!(tr.read("s1", &t), None);
        append(
            &t,
            "{\"type\":\"custom-title\",\"customTitle\":\"Later\"}\n",
        );
        assert_eq!(tr.changed(&t), Some(("s1".into(), "Later".into())));
        assert_eq!(tr.changed(&t), None);
        let other = dir.path().join("s2.jsonl");
        append(
            &other,
            "{\"type\":\"custom-title\",\"customTitle\":\"No\"}\n",
        );
        assert_eq!(tr.changed(&other), None);
    }

    #[test]
    fn a_link_inside_the_account_folder_is_not_followed() {
        let dir = Dir::new("titles-link");
        let folder = dir.path().join("acc");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(folder.join("projects")).expect("dirs");
        std::fs::create_dir_all(&outside).expect("dirs");
        std::fs::write(outside.join("s.jsonl"), "{}\n").expect("write");
        std::fs::write(folder.join("projects").join("s.jsonl"), "{}\n").expect("write");
        assert!(is_transcript_of(
            &folder,
            &folder.join("projects").join("s.jsonl")
        ));
        let link = folder.join("projects").join("away");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&outside, &link);
        #[cfg(not(windows))]
        let made = std::os::unix::fs::symlink(&outside, &link);
        if made.is_err() {
            // Windows lets only administrators and developer mode make symbolic links: nothing to test here then.
            return;
        }
        assert!(link.join("s.jsonl").is_file());
        assert!(!is_transcript_of(&folder, &link.join("s.jsonl")));
    }

    #[test]
    fn only_transcripts_inside_the_account_folder_are_read() {
        let folder = if cfg!(windows) {
            PathBuf::from(r"C:\Users\<name>\.claude")
        } else {
            PathBuf::from("/home/<name>/.claude")
        };
        let transcript = folder.join("projects").join("p").join("s.jsonl");
        assert!(inside(&folder, &transcript));
        assert!(!inside(&folder, &folder));
        assert!(!inside(&folder, &folder.join("projects").join("s.txt")));
        assert!(!inside(
            &folder,
            &folder.join("..").join("other").join("s.jsonl")
        ));
        assert!(!inside(
            &folder.join("x"),
            &folder.join("xy").join("s.jsonl")
        ));
        assert!(!inside(&folder, Path::new("s.jsonl")));
        #[cfg(windows)]
        {
            let upper = PathBuf::from(r"C:\USERS\<name>\.Claude\projects\s.jsonl");
            assert!(inside(&folder, &upper));
        }
    }
}
