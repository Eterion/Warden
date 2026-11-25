//! Memory over time: how much the AI sessions' processes hold, recorded while Warden runs.
//!
//! Every [`SAMPLE_EVERY_MS`] the server reads the computer's total and available memory. It takes a process
//! snapshot only if a hooked session has a known CLI process or a remembered process was running at the last
//! snapshot. The snapshot gives the parents, start times and private memory of processes. Only those linked to a
//! session and Warden's own count ([`plan`]). A delegated run's CLI starts its own tree, which counts once for
//! that run. A process linked to a session keeps its owner by id and start time ([`LeftRunning`]), even after a
//! parent exits. A child can also inherit the session of a recently seen parent. All linked processes count for
//! their session while it lives, then as "Left running" until they exit.
//!
//! The [`Recorder`] keeps each minute's highest value per series and appends it to the day's file in the data
//! folder, `memory/memory-<YYYY-MM-DD>.jsonl` (local days): one series line when a series first appears that day (or
//! its names change), one minute line a minute. Kept 30 days and 20 MB in all; at most 200 series and 1,440 minutes a
//! day. Today's minutes are also kept in memory, so today's answer reads no file. Never a prompt, command line, path
//! or process name: only memory figures, account ids and labels, session ids, and session and project names (a word
//! in a name that reads like a path or a program becomes `…`). `memory/linked.json` keeps the last saved snapshot's
//! time, the running processes' ids and start times, and their series, so ownership can survive a restart. It has
//! the same restrictions on text as day files. Nothing is read, written or removed through a link.

use crate::model::Provider;
use crate::time::{date_text, local_offset_s, parse_date};
use serde::{Deserialize, Serialize};
use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The folder in the data folder holding the day files and saved process links.
pub const DIR: &str = "memory";
pub const LINK_FILE: &str = "linked.json";
/// Room for a distinct series per process, even with six text fields of 64 four-byte characters each.
pub const MAX_LINK_BYTES: u64 = 4 * 1024 * 1024;
const LINK_VERSION: u32 = 1;
/// How often memory is sampled.
pub const SAMPLE_EVERY_MS: u64 = 10_000;
/// Days of files kept, today's included.
pub const KEEP_DAYS: i64 = 30;
/// Most all day files may hold together; the oldest go first.
pub const MAX_TOTAL_BYTES: u64 = 20 * 1024 * 1024;
/// Most one day's file may hold (a worst day, 200 series every minute, is about 5 MB); past it the day's further
/// lines are dropped.
pub const MAX_DAY_BYTES: u64 = 6 * 1024 * 1024;
/// Series a day holds at most (the sessions table's own cap); a later one is not recorded that day.
pub const MAX_SERIES: usize = 200;
/// Minutes a day holds at most.
pub const MAX_MINUTES: usize = 1440;
/// Owned processes remembered, running and recently exited combined, at most.
pub const MAX_LEFT: usize = 2000;
/// A session's or project's name is kept to this many characters.
pub const MAX_NAME: usize = 64;
/// Series lines a day's file holds at most: each series once, and again when its names change.
const MAX_SERIES_LINES: usize = 4 * MAX_SERIES;
const PREFIX: &str = "memory-";
const SUFFIX: &str = ".jsonl";
const KIB: u64 = 1024;
const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 86_400_000;

/// A process in one snapshot: its id, its parent's, when it started (whole seconds since the epoch)
/// and, when it was read, its private memory in bytes. Nothing else of another process is ever read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub parent: Option<u32>,
    pub started: u64,
    pub private: Option<u64>,
}

/// A live hooked session whose CLI process is known.
#[derive(Debug, Clone)]
pub struct Live {
    pub provider: Provider,
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub account_id: String,
    /// The account's label now.
    pub account_label: Option<String>,
    /// Its folder is signed in as someone else: it counts for no account.
    pub other_sign_in: bool,
    pub name: Option<String>,
    pub project: Option<String>,
    /// Its CLI's process id.
    pub cli: u32,
    pub last_event_at: i64,
}

/// What a series counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// A live session's CLI and the processes linked to it.
    Session,
    /// Processes that are still running after their session ended.
    Left,
    /// A session in a folder signed in as someone else.
    OtherSignIn,
    /// Warden itself, with its window's WebView2 processes.
    Warden,
}

/// One series: what it counts and, for a session's, whose it is, named as the Sessions table names it then.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesInfo {
    pub kind: Kind,
    pub provider: Option<Provider>,
    pub account_id: Option<String>,
    /// The account's label at the time (accounts can be renamed or removed later).
    pub account: Option<String>,
    pub session_id: Option<String>,
    pub parent_session_id: Option<String>,
    pub name: Option<String>,
    pub project: Option<String>,
}

type Key = (Kind, Option<Provider>, Option<String>);

impl SeriesInfo {
    fn key(&self) -> Key {
        (self.kind, self.provider, self.session_id.clone())
    }

    fn warden() -> SeriesInfo {
        SeriesInfo {
            kind: Kind::Warden,
            provider: None,
            account_id: None,
            account: None,
            session_id: None,
            parent_session_id: None,
            name: None,
            project: None,
        }
    }

    fn of(live: &Live, kind: Kind) -> SeriesInfo {
        let own = !live.other_sign_in;
        SeriesInfo {
            kind,
            provider: Some(live.provider),
            account_id: own.then(|| live.account_id.clone()),
            account: own.then(|| live.account_label.clone()).flatten(),
            session_id: Some(live.session_id.clone()),
            parent_session_id: live.parent_session_id.clone(),
            name: live.name.as_deref().map(masked),
            project: live.project.as_deref().map(masked),
        }
    }

    /// Itself with every text capped, as a day file keeps it.
    fn capped(mut self) -> SeriesInfo {
        for text in [
            &mut self.account_id,
            &mut self.account,
            &mut self.session_id,
            &mut self.parent_session_id,
            &mut self.name,
            &mut self.project,
        ] {
            if let Some(t) = text.as_mut() {
                *t = short(t);
            }
        }
        for text in [&mut self.name, &mut self.project] {
            if let Some(t) = text.as_mut() {
                *t = masked(t);
            }
        }
        self
    }
}

/// At most [`MAX_NAME`] characters, on one line.
fn short(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect()
}

/// A word that reads like a path or a program: a folder separator, a drive, an environment variable or `.exe`.
fn path_like(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    word.contains('\\')
        || word.contains(":/")
        || word.starts_with('/')
        || word.starts_with("~/")
        || word.contains('%')
        || lower.contains(".exe")
}

/// A session's or project's name as the history keeps it: each word that reads like a path or a program replaced by
/// `…` (a title can name a folder), on one line, at most [`MAX_NAME`] characters.
fn masked(text: &str) -> String {
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .map(|w| if path_like(w) { "…" } else { w })
        .collect();
    short(&words.join(" "))
}

/// Whether a path is a link or another reparse point (a junction): the history never reads, writes or removes
/// through one.
fn linked(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

/// Refused: the memory folder or one of its files is a link.
fn linked_error() -> io::Error {
    io::Error::other("the memory history is behind a link, so it is left alone")
}

/// A series line: its number in the day and what it counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    pub series: u16,
    #[serde(flatten)]
    pub info: SeriesInfo,
}

/// A minute line: the minute of the local day, the computer's total and in use, the AI sessions' total (sessions,
/// left running and other sign-ins; not Warden), and `[series, KiB]` for each series with memory that minute. All in
/// KiB, each the minute's highest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Minute {
    pub m: u16,
    pub total: u32,
    pub used: u32,
    pub sum: u32,
    pub s: Vec<(u16, u32)>,
}

impl Minute {
    /// Takes the higher of each value from another record of the same minute.
    fn merge(&mut self, other: &Minute) {
        self.total = self.total.max(other.total);
        self.used = self.used.max(other.used);
        self.sum = self.sum.max(other.sum);
        for &(n, kib) in &other.s {
            match self.s.iter_mut().find(|(m, _)| *m == n) {
                Some(pair) => pair.1 = pair.1.max(kib),
                None => self.s.push((n, kib)),
            }
        }
        self.s.sort_unstable();
    }
}

fn kib(bytes: u64) -> u32 {
    u32::try_from(bytes.div_ceil(KIB)).unwrap_or(u32::MAX)
}

// ---------------------------------------------------------------------------------------------------------------
// Attribution
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    Session(usize),
    Left(usize),
    Warden,
}

/// Which process counts for which series in one snapshot, before any memory is read.
#[derive(Debug)]
pub struct Plan {
    owners: HashMap<u32, Owner>,
    started: HashMap<u32, u64>,
    sessions: Vec<Live>,
    left: Vec<Arc<RememberedSeries>>,
    sampled_at_ms: u64,
}

#[derive(Debug)]
struct RememberedProcess {
    started: u64,
    /// The first snapshot that found this process missing. None if it was still running.
    first_missing_at_ms: Option<u64>,
    session: Arc<RememberedSeries>,
}

#[derive(Debug)]
struct RememberedSeries {
    info: SeriesInfo,
    /// A saved series uses the same shortened id as the day files.
    from_disk: bool,
}

impl RememberedProcess {
    fn can_parent(&self, started: u64, sampled_at_ms: u64, restored_at_ms: Option<u64>) -> bool {
        // A child present at the first missing snapshot already existed at its cutoff. For one first seen
        // later, require its whole reported start second to fit before that cutoff. This bounds the risk to
        // reuse followed by another exit between two snapshots, which the snapshots cannot rule out.
        self.started != 0
            && self.started <= started
            && started <= sampled_at_ms / 1000
            && self
                .first_missing_at_ms
                .or(restored_at_ms)
                .is_none_or(|missing_at| started < missing_at / 1000)
    }
}

/// Processes attributed to a session, with their identities and whether they were running at the last snapshot.
#[derive(Debug, Default)]
pub struct LeftRunning {
    processes: HashMap<u32, RememberedProcess>,
    sampled_at_ms: u64,
    restored_at_ms: Option<u64>,
}

impl LeftRunning {
    pub fn is_empty(&self) -> bool {
        self.processes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.processes.len()
    }

    /// Whether an owned process was seen running at the last snapshot.
    pub fn any_running(&self) -> bool {
        self.processes
            .values()
            .any(|p| p.first_missing_at_ms.is_none())
    }

    fn restored_cutoff(&self, sampled_at_ms: u64) -> Option<u64> {
        self.restored_at_ms.map(|saved| saved.min(sampled_at_ms))
    }

    /// After a snapshot: refresh running owners first, then keep newly missing parents for one more snapshot in
    /// the remaining space. A PID that now belongs to a replacement is safe to forget after this snapshot.
    pub fn update(&mut self, plan: &Plan) {
        // Across a restart the parent might have exited just after the saved snapshot. A real child that
        // started after that snapshot may be missed, but reuse during downtime cannot extend the window.
        let missing_at_ms = self
            .restored_cutoff(plan.sampled_at_ms)
            .unwrap_or(plan.sampled_at_ms);
        self.restored_at_ms = None;
        self.sampled_at_ms = plan.sampled_at_ms;
        let mut missing: Vec<_> = self
            .processes
            .extract_if(|pid, p| p.started == 0 || plan.started.get(pid) != Some(&p.started))
            .filter_map(|(pid, mut p)| {
                if p.started == 0
                    || plan.started.contains_key(&pid)
                    || p.first_missing_at_ms.is_some()
                {
                    return None;
                }
                p.first_missing_at_ms = Some(missing_at_ms);
                Some((pid, p))
            })
            .collect();
        let sessions: Vec<_> = plan
            .sessions
            .iter()
            .map(|s| {
                Arc::new(RememberedSeries {
                    info: SeriesInfo::of(s, Kind::Left),
                    from_disk: false,
                })
            })
            .collect();
        let mut owned: Vec<(&u32, &Owner)> = plan.owners.iter().collect();
        owned.sort_unstable_by_key(|(pid, _)| **pid);
        for (pid, owner) in owned {
            let started = plan.started[pid];
            if started == 0 {
                continue;
            }
            let session = match *owner {
                Owner::Session(index) => &sessions[index],
                Owner::Left(index) => &plan.left[index],
                Owner::Warden => continue,
            };
            if self.processes.len() < MAX_LEFT || self.processes.contains_key(pid) {
                self.processes.insert(
                    *pid,
                    RememberedProcess {
                        started,
                        first_missing_at_ms: None,
                        session: Arc::clone(session),
                    },
                );
            }
        }
        let room = MAX_LEFT - self.processes.len();
        if room > 0 {
            missing.sort_unstable_by_key(|(pid, _)| *pid);
            self.processes.extend(missing.into_iter().take(room));
        }
    }
}

/// Works out whose each process is: every live session's CLI and the processes under it (each step down started no
/// earlier than its parent; never into another session's CLI), then remembered processes and children of recently
/// seen missing parents, then Warden's own process and what it started. `sampled_at_ms` is the time the process
/// snapshot was read, in milliseconds since the epoch.
pub fn plan(
    procs: &[Proc],
    live: &[Live],
    own: u32,
    left: &LeftRunning,
    sampled_at_ms: u64,
) -> Plan {
    let started: HashMap<u32, u64> = procs.iter().map(|p| (p.pid, p.started)).collect();
    let restored_cutoff = left.restored_cutoff(sampled_at_ms);
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut orphans = Vec::new();
    for p in procs {
        let Some(parent) = p.parent.filter(|&pid| pid != p.pid) else {
            continue;
        };
        match started.get(&parent) {
            Some(&start) if start <= p.started => children.entry(parent).or_default().push(p.pid),
            // The original parent is missing if no process holds the PID or the holder started after the child.
            _ => {
                if let Some(q) = left
                    .processes
                    .get(&parent)
                    .filter(|q| q.can_parent(p.started, sampled_at_ms, restored_cutoff))
                {
                    orphans.push((p.pid, q));
                }
            }
        }
    }
    // One session per CLI: a CLI that started a new session (Claude's /clear) counts for its newest.
    let mut sessions: Vec<Live> = Vec::new();
    for s in live.iter().filter(|s| started.contains_key(&s.cli)) {
        match sessions.iter_mut().find(|t| t.cli == s.cli) {
            Some(t) if t.last_event_at < s.last_event_at => *t = s.clone(),
            Some(_) => {}
            None => sessions.push(s.clone()),
        }
    }
    sessions.sort_by(|a, b| (a.provider, &a.session_id).cmp(&(b.provider, &b.session_id)));
    let clis: HashSet<u32> = sessions.iter().map(|s| s.cli).collect();
    let mut owners: HashMap<u32, Owner> = HashMap::new();
    let walk = |root: u32, owner: Owner, owners: &mut HashMap<u32, Owner>| {
        let mut stack = vec![root];
        while let Some(pid) = stack.pop() {
            if owners.contains_key(&pid) {
                continue;
            }
            owners.insert(pid, owner);
            for &c in children.get(&pid).into_iter().flatten() {
                if !clis.contains(&c) && !owners.contains_key(&c) {
                    stack.push(c);
                }
            }
        }
    };
    for (i, s) in sessions.iter().enumerate() {
        walk(s.cli, Owner::Session(i), &mut owners);
    }
    let by_session: HashMap<_, _> = sessions
        .iter()
        .enumerate()
        .map(|(i, s)| ((s.provider, s.session_id.as_str()), i))
        .collect();
    let by_saved_session = OnceCell::new();
    let mut left_infos = Vec::new();
    let mut recall = |pid, series: &Arc<RememberedSeries>, owners: &mut HashMap<u32, Owner>| {
        if owners.contains_key(&pid) {
            return;
        }
        let session = series.info.provider.zip(series.info.session_id.as_deref());
        let index = session.and_then(|(provider, id)| {
            if !series.from_disk {
                return by_session.get(&(provider, id)).copied();
            }
            let saved = by_saved_session.get_or_init(|| {
                let mut saved = HashMap::new();
                for (i, s) in sessions.iter().enumerate() {
                    // Two live ids can share the same stored id. Keep their saved processes as Left running.
                    saved
                        .entry(s.provider)
                        .or_insert_with(HashMap::new)
                        .entry(short(&s.session_id))
                        .and_modify(|i| *i = None)
                        .or_insert(Some(i));
                }
                saved
            });
            saved.get(&provider)?.get(id).copied().flatten()
        });
        let owner = if let Some(index) = index {
            Owner::Session(index)
        } else {
            let index = left_infos.len();
            left_infos.push(Arc::clone(series));
            Owner::Left(index)
        };
        walk(pid, owner, owners);
    };
    let mut remembered: Vec<_> = left.processes.iter().collect();
    remembered.sort_unstable_by_key(|(pid, _)| **pid);
    for (&pid, p) in remembered {
        if started.get(&pid) == Some(&p.started) {
            recall(pid, &p.session, &mut owners);
        }
    }
    for (pid, parent) in orphans {
        recall(pid, &parent.session, &mut owners);
    }
    if started.contains_key(&own) && !owners.contains_key(&own) && !clis.contains(&own) {
        walk(own, Owner::Warden, &mut owners);
    }
    Plan {
        owners,
        started,
        sessions,
        left: left_infos,
        sampled_at_ms,
    }
}

impl Plan {
    /// The processes whose memory to read.
    pub fn pids(&self) -> Vec<u32> {
        let mut pids: Vec<u32> = self.owners.keys().copied().collect();
        pids.sort_unstable();
        pids
    }

    /// Each series' memory in bytes, from the processes read (a process whose memory could not be read counts 0).
    pub fn values(&self, procs: &[Proc]) -> Vec<(SeriesInfo, u64)> {
        let mut out: Vec<(SeriesInfo, u64)> = Vec::new();
        let mut index: HashMap<Key, usize> = HashMap::new();
        for p in procs {
            let (Some(owner), Some(bytes)) = (self.owners.get(&p.pid), p.private) else {
                continue;
            };
            if self.started.get(&p.pid) != Some(&p.started) {
                continue;
            }
            let info = match *owner {
                Owner::Session(index) => {
                    let s = &self.sessions[index];
                    let kind = if s.other_sign_in {
                        Kind::OtherSignIn
                    } else {
                        Kind::Session
                    };
                    SeriesInfo::of(s, kind)
                }
                Owner::Left(index) => self.left[index].info.clone(),
                Owner::Warden => SeriesInfo::warden(),
            };
            match index.get(&info.key()) {
                Some(&at) => out[at].1 += bytes,
                None => {
                    index.insert(info.key(), out.len());
                    out.push((info, bytes));
                }
            }
        }
        out.sort_by_key(|a| a.0.key());
        out
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------------------------------------------

/// One sample: when, the computer's memory, and each series' memory (bytes).
#[derive(Debug, Clone)]
pub struct Reading {
    pub at_ms: i64,
    pub total: u64,
    pub available: u64,
    pub values: Vec<(SeriesInfo, u64)>,
}

/// The local date (`YYYY-MM-DD`) and minute of the day of a moment, in a time zone `offset_s` ahead of UTC.
pub fn local_minute(at_ms: i64, offset_s: i32) -> (String, u16) {
    let local = at_ms + i64::from(offset_s) * 1000;
    let minute = u16::try_from(local.rem_euclid(DAY_MS) / MINUTE_MS).unwrap_or(0);
    (date_text(local.div_euclid(DAY_MS)), minute)
}

/// Today's local date, `YYYY-MM-DD`.
pub fn today(now_ms: i64) -> String {
    local_minute(now_ms, local_offset_s(now_ms)).0
}

/// A day's file in the memory folder.
pub fn day_file(dir: &Path, date: &str) -> PathBuf {
    dir.join(format!("{PREFIX}{date}{SUFFIX}"))
}

/// A day as its file holds it: its series (the last line for each number) and its minutes (in order, one each).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Day {
    pub series: Vec<Series>,
    pub minutes: Vec<Minute>,
}

impl Day {
    fn add_series(&mut self, s: Series) {
        if usize::from(s.series) == 0 || usize::from(s.series) > MAX_SERIES {
            return;
        }
        match self.series.iter_mut().find(|t| t.series == s.series) {
            Some(t) => *t = s,
            None => self.series.push(s),
        }
    }

    fn add_minute(&mut self, mut m: Minute) {
        if usize::from(m.m) >= MAX_MINUTES {
            return;
        }
        m.s.retain(|(n, _)| usize::from(*n) >= 1 && usize::from(*n) <= MAX_SERIES);
        m.s.sort_unstable();
        m.s.dedup_by_key(|(n, _)| *n);
        match self.minutes.binary_search_by_key(&m.m, |x| x.m) {
            Ok(i) => self.minutes[i].merge(&m),
            Err(i) => self.minutes.insert(i, m),
        }
    }
}

/// Reads a day's file: a line that is not a series or a minute (a cut last line, say) is skipped. No file: an
/// empty day.
pub fn read_day(dir: &Path, date: &str) -> Day {
    let mut day = Day::default();
    let path = day_file(dir, date);
    if linked(dir) || linked(&path) {
        return day;
    }
    let bytes = read_bounded(&path);
    // Line by line: a line cut inside a character spoils only itself.
    for line in String::from_utf8_lossy(&bytes).lines() {
        if let Ok(m) = serde_json::from_str::<Minute>(line) {
            day.add_minute(m);
        } else if let Ok(s) = serde_json::from_str::<Series>(line) {
            day.add_series(s);
        }
    }
    day.series.sort_by_key(|s| s.series);
    day
}

/// A day file's bytes, at most a full day's and a little more; none when it cannot be read.
fn read_bounded(path: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    let read = fs::File::open(path)
        .and_then(|file| file.take(MAX_DAY_BYTES + 64 * KIB).read_to_end(&mut bytes));
    match read {
        Ok(_) => bytes,
        Err(_) => Vec::new(),
    }
}

/// Writes a whole day's file, unless it already exists (the demo's made-up past days).
pub fn write_day(dir: &Path, date: &str, day: &Day) -> io::Result<()> {
    if linked(dir) {
        return Err(linked_error());
    }
    fs::create_dir_all(dir)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(day_file(dir, date))?;
    let mut text = String::new();
    for s in &day.series {
        text.push_str(&serde_json::to_string(s).map_err(io::Error::other)?);
        text.push('\n');
    }
    for m in &day.minutes {
        text.push_str(&serde_json::to_string(m).map_err(io::Error::other)?);
        text.push('\n');
    }
    file.write_all(text.as_bytes())
}

/// The dates of the day files in the memory folder, oldest first.
pub fn days(dir: &Path) -> Vec<String> {
    if linked(dir) {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let date = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
            parse_date(date).map(|_| date.to_owned())
        })
        .collect();
    out.sort();
    out
}

/// Removes the day files from before the last [`KEEP_DAYS`] days (today's counted), then the oldest until the rest
/// leave room for a full day within [`MAX_TOTAL_BYTES`]. Today's file is never removed; only Warden's own
/// `memory-YYYY-MM-DD.jsonl` files are touched.
pub fn prune(dir: &Path, today: &str) {
    let Some(today_n) = parse_date(today) else {
        return;
    };
    let oldest = date_text(today_n - (KEEP_DAYS - 1));
    let mut others: Vec<(String, u64)> = days(dir)
        .into_iter()
        .filter(|d| d.as_str() != today)
        .filter(|d| !linked(&day_file(dir, d)))
        .filter_map(|d| {
            let len = fs::metadata(day_file(dir, &d)).ok()?.len();
            Some((d, len))
        })
        .collect();
    // Newest first.
    others.sort_by(|a, b| b.0.cmp(&a.0));
    let mut total = fs::metadata(day_file(dir, today)).map_or(0, |m| m.len());
    let room = MAX_DAY_BYTES.max(total);
    total = 0;
    for (date, len) in others {
        total += len;
        if date < oldest || total + room > MAX_TOTAL_BYTES {
            let _ = fs::remove_file(day_file(dir, &date));
        }
    }
}

/// One snapshot's running links, with each series stored once and the links sorted by PID.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LinkSnapshot {
    version: u32,
    sampled_at_ms: u64,
    series: Vec<SeriesInfo>,
    processes: Vec<(u32, u64, u16)>,
}

impl LinkSnapshot {
    fn from_left(left: &LeftRunning) -> Option<Self> {
        let mut running: Vec<_> = left
            .processes
            .iter()
            .filter(|(_, p)| p.started != 0 && p.first_missing_at_ms.is_none())
            .collect();
        if running.is_empty() {
            return None;
        }
        running.sort_unstable_by_key(|(pid, _)| **pid);
        let mut series = Vec::new();
        let mut stored = HashMap::new();
        let mut indices = HashMap::new();
        let mut processes = Vec::with_capacity(running.len());
        for (&pid, p) in running {
            let n = *indices.entry(&p.session.info).or_insert_with(|| {
                let info = p.session.info.clone().capped();
                *stored.entry(info).or_insert_with_key(|info| {
                    let n = series.len() as u16;
                    series.push(info.clone());
                    n
                })
            });
            processes.push((pid, p.started, n));
        }
        Some(Self {
            version: LINK_VERSION,
            sampled_at_ms: left.sampled_at_ms,
            series,
            processes,
        })
    }

    fn read(dir: &Path) -> io::Result<(Self, LeftRunning)> {
        let path = dir.join(LINK_FILE);
        if linked(dir) || linked(&path) {
            return Err(linked_error());
        }
        remove_link_temps(dir);
        let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid saved memory links");
        let mut bytes = Vec::new();
        fs::File::open(&path)?
            .take(MAX_LINK_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_LINK_BYTES {
            return Err(invalid());
        }
        let saved: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if saved.version != LINK_VERSION
            || saved.processes.is_empty()
            || saved.processes.len() > MAX_LEFT
            || saved.series.len() > saved.processes.len()
            || saved.series.iter().any(|s| {
                s.kind != Kind::Left
                    || s.provider.is_none()
                    || s.session_id.is_none()
                    || *s != s.clone().capped()
            })
        {
            return Err(invalid());
        }
        let mut left = LeftRunning {
            processes: HashMap::with_capacity(saved.processes.len()),
            sampled_at_ms: saved.sampled_at_ms,
            restored_at_ms: Some(saved.sampled_at_ms),
        };
        let series: Vec<_> = saved
            .series
            .iter()
            .cloned()
            .map(|info| {
                Arc::new(RememberedSeries {
                    info,
                    from_disk: true,
                })
            })
            .collect();
        for &(pid, started, n) in &saved.processes {
            if started == 0 {
                continue;
            }
            let session = Arc::clone(series.get(usize::from(n)).ok_or_else(invalid)?);
            let prev = left.processes.insert(
                pid,
                RememberedProcess {
                    started,
                    first_missing_at_ms: None,
                    session,
                },
            );
            if prev.is_some() {
                return Err(invalid());
            }
        }
        Ok((saved, left))
    }
}

/// The server's lock allows only one process to use this folder, and the server serializes its link writes.
fn remove_link_temps(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let is_temp = name
            .to_str()
            .and_then(|name| name.strip_prefix(LINK_FILE))
            .and_then(|suffix| suffix.strip_prefix('.'))
            .and_then(|suffix| suffix.strip_suffix(".tmp"))
            .is_some_and(|stem| !stem.is_empty());
        if is_temp && entry.file_type().is_ok_and(|kind| kind.is_file()) {
            let path = entry.path();
            if !linked(&path) {
                let _ = fs::remove_file(path);
            }
        }
    }
}

/// A serialized link update, ready to write without holding the recorder lock.
pub struct LinkWrite {
    dir: PathBuf,
    bytes: Option<Vec<u8>>,
}

impl LinkWrite {
    pub fn write(self) -> io::Result<()> {
        let path = self.dir.join(LINK_FILE);
        if linked(&self.dir) || linked(&path) {
            return Err(linked_error());
        }
        remove_link_temps(&self.dir);
        if let Some(bytes) = self.bytes {
            fs::create_dir_all(&self.dir)?;
            crate::store::replace_atomic(&path, &bytes, false)?;
        } else if let Err(e) = fs::remove_file(&path)
            && e.kind() != io::ErrorKind::NotFound
        {
            return Err(e);
        }
        Ok(())
    }
}

/// Today's day, as recorded so far, and how much of its file is written.
#[derive(Debug)]
struct Today {
    date: String,
    day: Day,
    numbers: HashMap<Key, u16>,
    series_lines: usize,
    bytes: u64,
    /// The file does not end with a whole line (a cut last line): the next line starts on a new one.
    cut: bool,
}

/// The minute being gathered: each value's highest so far.
#[derive(Debug)]
struct Gathering {
    date: String,
    m: u16,
    total: u32,
    used: u32,
    sum: u32,
    values: Vec<(SeriesInfo, u32)>,
}

/// Keeps the minutes and writes the day files and process links. One per data folder, owned by the running server.
#[derive(Debug)]
pub struct Recorder {
    dir: PathBuf,
    today: Option<Today>,
    gathering: Option<Gathering>,
    links: Option<LinkSnapshot>,
    links_dirty: bool,
}

impl Recorder {
    /// The recorder for a data folder at `now`: older days pruned, today's file read back.
    pub fn new(data_dir: &Path, now_ms: i64) -> Recorder {
        Recorder::new_in(data_dir, now_ms, local_offset_s(now_ms))
    }

    /// As [`Recorder::new`], in a time zone `offset_s` ahead of UTC.
    pub fn new_in(data_dir: &Path, now_ms: i64, offset_s: i32) -> Recorder {
        let mut r = Recorder {
            dir: data_dir.join(DIR),
            today: None,
            gathering: None,
            links: None,
            links_dirty: false,
        };
        let (date, _) = local_minute(now_ms, offset_s);
        r.open_day(&date);
        r
    }

    /// The memory folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Called once when the sampler starts. No file gives it an empty set of links.
    pub fn load_links(&mut self) -> io::Result<LeftRunning> {
        match LinkSnapshot::read(&self.dir) {
            Ok((saved, left)) => {
                self.links_dirty = saved.processes.iter().any(|&(_, started, _)| started == 0);
                self.links = if self.links_dirty {
                    LinkSnapshot::from_left(&left)
                } else {
                    Some(saved)
                };
                Ok(left)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.links = None;
                self.links_dirty = false;
                Ok(LeftRunning::default())
            }
            Err(e) => {
                self.links = None;
                self.links_dirty = true;
                Err(e)
            }
        }
    }

    /// Prepares changed links for writing. An unchanged snapshot still supplies the timestamp for shutdown.
    pub fn prepare_links(&mut self, left: &LeftRunning) -> io::Result<Option<LinkWrite>> {
        let next = LinkSnapshot::from_left(left);
        let changed = match (&self.links, &next) {
            (Some(a), Some(b)) => a.processes != b.processes || a.series != b.series,
            (None, None) => false,
            _ => true,
        };
        self.links = next;
        self.links_dirty |= changed;
        self.links_dirty.then(|| self.serialize_links()).transpose()
    }

    /// Confirms a successful write. The caller serializes preparation, writing and confirmation with other writers.
    pub fn links_saved(&mut self) {
        self.links_dirty = false;
    }

    /// Prepares the latest snapshot's links for the shutdown write.
    pub fn finish_links(&self) -> io::Result<Option<LinkWrite>> {
        (self.links.is_some() || self.links_dirty)
            .then(|| self.serialize_links())
            .transpose()
    }

    fn serialize_links(&self) -> io::Result<LinkWrite> {
        let bytes = self
            .links
            .as_ref()
            .map(serde_json::to_vec)
            .transpose()
            .map_err(io::Error::other)?;
        Ok(LinkWrite {
            dir: self.dir.clone(),
            bytes,
        })
    }

    /// Opens a day: older days pruned, its file read back.
    fn open_day(&mut self, date: &str) {
        prune(&self.dir, date);
        let path = day_file(&self.dir, date);
        let text = if linked(&self.dir) || linked(&path) {
            Vec::new()
        } else {
            read_bounded(&path)
        };
        let bytes = text.len() as u64;
        let cut = text.last().is_some_and(|b| *b != b'\n');
        let series_lines = String::from_utf8_lossy(&text)
            .lines()
            .filter(|l| l.starts_with("{\"series\":"))
            .count();
        let day = read_day(&self.dir, date);
        let numbers = day
            .series
            .iter()
            .map(|s| (s.info.key(), s.series))
            .collect();
        self.today = Some(Today {
            date: date.to_owned(),
            series_lines,
            day,
            numbers,
            bytes,
            cut,
        });
    }

    /// Takes one sample. The minute before it, once over, goes to today's file. Errors are the file's; the minute
    /// is kept in memory either way.
    pub fn record(&mut self, reading: &Reading) -> io::Result<()> {
        self.record_in(reading, local_offset_s(reading.at_ms))
    }

    /// As [`Recorder::record`], in a time zone `offset_s` ahead of UTC.
    pub fn record_in(&mut self, reading: &Reading, offset_s: i32) -> io::Result<()> {
        let (date, m) = local_minute(reading.at_ms, offset_s);
        let mut result = Ok(());
        if self
            .gathering
            .as_ref()
            .is_some_and(|g| g.date != date || g.m != m)
        {
            result = self.flush();
        }
        let g = self.gathering.get_or_insert_with(|| Gathering {
            date,
            m,
            total: 0,
            used: 0,
            sum: 0,
            values: Vec::new(),
        });
        g.total = g.total.max(kib(reading.total));
        g.used = g
            .used
            .max(kib(reading.total.saturating_sub(reading.available)));
        let mut sum = 0u64;
        for (info, bytes) in &reading.values {
            if info.kind != Kind::Warden {
                sum += bytes;
            }
            let info = info.clone().capped();
            let value = kib(*bytes);
            match g.values.iter_mut().find(|(i, _)| i.key() == info.key()) {
                Some((i, v)) => {
                    *i = info;
                    *v = (*v).max(value);
                }
                None => g.values.push((info, value)),
            }
        }
        g.sum = g.sum.max(kib(sum));
        result
    }

    /// Writes the minute being gathered (when Warden stops).
    pub fn finish(&mut self) -> io::Result<()> {
        self.flush()
    }

    fn flush(&mut self) -> io::Result<()> {
        let Some(g) = self.gathering.take() else {
            return Ok(());
        };
        if self.today.as_ref().is_none_or(|t| t.date != g.date) {
            self.open_day(&g.date);
        }
        let Some(t) = self.today.as_mut() else {
            return Ok(());
        };
        let known = t.day.minutes.iter().any(|x| x.m == g.m);
        if !known && t.day.minutes.len() >= MAX_MINUTES {
            return Ok(());
        }
        let mut lines = String::new();
        let mut pairs = Vec::new();
        for (info, value) in g.values {
            let number = match t.numbers.get(&info.key()) {
                Some(&n) => {
                    let known = t.day.series.iter().find(|s| s.series == n);
                    if known.is_some_and(|s| s.info != info) && t.series_lines < MAX_SERIES_LINES {
                        let s = Series { series: n, info };
                        lines.push_str(&line(&s)?);
                        t.series_lines += 1;
                        t.day.add_series(s);
                    }
                    n
                }
                None if t.day.series.len() < MAX_SERIES && t.series_lines < MAX_SERIES_LINES => {
                    let n = u16::try_from(t.day.series.len() + 1).unwrap_or(u16::MAX);
                    t.numbers.insert(info.key(), n);
                    let s = Series { series: n, info };
                    lines.push_str(&line(&s)?);
                    t.series_lines += 1;
                    t.day.add_series(s);
                    n
                }
                None => continue,
            };
            pairs.push((number, value));
        }
        pairs.sort_unstable();
        let minute = Minute {
            m: g.m,
            total: g.total,
            used: g.used,
            sum: g.sum,
            s: pairs,
        };
        // One line per minute of the day: a minute already written (Warden started again within it, or the hour that
        // repeats when daylight saving ends) is merged in memory only, so the file never holds more than 1,440.
        if !known {
            lines.push_str(&line(&minute)?);
        }
        t.day.add_minute(minute);
        if lines.is_empty() {
            return Ok(());
        }
        if t.bytes + lines.len() as u64 > MAX_DAY_BYTES {
            // The file is full for today: nothing more is written, so no minute names a series line left out.
            t.bytes = MAX_DAY_BYTES;
            return Ok(());
        }
        if t.cut {
            lines.insert(0, '\n');
        }
        let path = day_file(&self.dir, &t.date);
        let written = (|| {
            if linked(&self.dir) || linked(&path) {
                return Err(linked_error());
            }
            fs::create_dir_all(&self.dir)?;
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?
                .write_all(lines.as_bytes())
        })();
        match written {
            Ok(()) => {
                t.bytes += lines.len() as u64;
                t.cut = false;
                Ok(())
            }
            Err(e) => {
                // What was taken as written is not: today is read back from its file at the next minute (its
                // series numbers, its lines, a cut last line), so no minute names a series the file lacks.
                self.today = None;
                Err(e)
            }
        }
    }

    /// Today's day as recorded, with the minute being gathered (its new series numbered as they will be). Empty when
    /// nothing was recorded on `date`.
    pub fn today_day(&self, date: &str) -> Day {
        let mut day = match &self.today {
            Some(t) if t.date == date => t.day.clone(),
            _ => Day::default(),
        };
        let Some(g) = self.gathering.as_ref().filter(|g| g.date == date) else {
            return day;
        };
        let mut pairs = Vec::new();
        for (info, value) in &g.values {
            let n = match day.series.iter().find(|s| s.info.key() == info.key()) {
                Some(s) => s.series,
                None if day.series.len() < MAX_SERIES => {
                    let n = u16::try_from(day.series.len() + 1).unwrap_or(u16::MAX);
                    day.series.push(Series {
                        series: n,
                        info: info.clone(),
                    });
                    n
                }
                None => continue,
            };
            pairs.push((n, *value));
        }
        day.add_minute(Minute {
            m: g.m,
            total: g.total,
            used: g.used,
            sum: g.sum,
            s: pairs,
        });
        day
    }

    /// The minute being gathered on `date`, if any: after the clock goes back an hour it can come before minutes
    /// already recorded.
    pub fn gathering_minute(&self, date: &str) -> Option<u16> {
        self.gathering
            .as_ref()
            .filter(|g| g.date == date)
            .map(|g| g.m)
    }

    /// Whether anything was recorded on `date` so far (today's).
    pub fn has(&self, date: &str) -> bool {
        self.today
            .as_ref()
            .is_some_and(|t| t.date == date && !t.day.minutes.is_empty())
            || self.gathering.as_ref().is_some_and(|g| g.date == date)
    }
}

fn line(value: &impl Serialize) -> io::Result<String> {
    let mut text = serde_json::to_string(value).map_err(io::Error::other)?;
    text.push('\n');
    Ok(text)
}

// ---------------------------------------------------------------------------------------------------------------
// A day's answer
// ---------------------------------------------------------------------------------------------------------------

/// Minutes without a recording, from `from` up to (not including) `to`, between a day's first and last recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gap {
    pub from: u16,
    pub to: u16,
}

/// A series over the day: its latest value (today only) and its highest, with the minute it was reached.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesSummary {
    pub series: u16,
    pub now: Option<u32>,
    pub peak: u32,
    pub peak_at: u16,
}

/// The day in figures. Memory in KiB; shares in percent of the computer's total memory.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// The first and last minute recorded.
    pub first: Option<u16>,
    pub last: Option<u16>,
    /// The computer's total memory, as last recorded that day.
    pub computer_total: Option<u32>,
    /// The AI sessions' total in the minute recorded last (today only).
    pub now: Option<u32>,
    pub now_share: Option<f64>,
    /// The AI sessions' highest total, and the minute it was reached.
    pub peak: Option<u32>,
    pub peak_at: Option<u16>,
    pub peak_share: Option<f64>,
    pub series: Vec<SeriesSummary>,
}

/// One day's memory (`GET /api/memory/day`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDay {
    pub date: String,
    pub today: bool,
    /// The days with recordings, oldest first.
    pub days: Vec<String>,
    pub series: Vec<Series>,
    pub minutes: Vec<Minute>,
    pub gaps: Vec<Gap>,
    pub summary: Summary,
}

fn share(kib: u32, total: Option<u32>) -> Option<f64> {
    let total = total.filter(|t| *t > 0)?;
    Some((f64::from(kib) * 1000.0 / f64::from(total)).round() / 10.0)
}

impl MemoryDay {
    /// The answer for a day: its series and minutes, the gaps between them and its figures.
    pub fn new(date: &str, today: bool, days: Vec<String>, day: Day) -> MemoryDay {
        let Day { series, minutes } = day;
        let mut gaps = Vec::new();
        for pair in minutes.windows(2) {
            if pair[1].m > pair[0].m + 1 {
                gaps.push(Gap {
                    from: pair[0].m + 1,
                    to: pair[1].m,
                });
            }
        }
        let last = minutes.last();
        let computer_total = last.map(|m| m.total);
        let peak = minutes
            .iter()
            .max_by_key(|m| (m.sum, std::cmp::Reverse(m.m)));
        let mut per: Vec<SeriesSummary> = Vec::new();
        for minute in &minutes {
            for &(n, value) in &minute.s {
                match per.iter_mut().find(|p| p.series == n) {
                    Some(p) if value > p.peak => {
                        p.peak = value;
                        p.peak_at = minute.m;
                    }
                    Some(_) => {}
                    None => per.push(SeriesSummary {
                        series: n,
                        now: None,
                        peak: value,
                        peak_at: minute.m,
                    }),
                }
            }
        }
        if today && let Some(last) = last {
            for p in &mut per {
                p.now = Some(
                    last.s
                        .iter()
                        .find(|(n, _)| *n == p.series)
                        .map_or(0, |(_, v)| *v),
                );
            }
        }
        per.sort_by_key(|p| p.series);
        let now = last.filter(|_| today).map(|m| m.sum);
        MemoryDay {
            date: date.to_owned(),
            today,
            days,
            summary: Summary {
                first: minutes.first().map(|m| m.m),
                last: last.map(|m| m.m),
                computer_total,
                now,
                now_share: now.and_then(|n| share(n, computer_total)),
                peak: peak.map(|m| m.sum),
                peak_at: peak.map(|m| m.m),
                peak_share: peak.and_then(|m| share(m.sum, computer_total)),
                series: per,
            },
            series,
            minutes,
            gaps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::Dir;

    const T0: i64 = 1_795_000_000_000;
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;

    fn proc(pid: u32, parent: Option<u32>, started: u64, mib: u64) -> Proc {
        Proc {
            pid,
            parent,
            started,
            private: Some(mib * MIB),
        }
    }

    fn live(id: &str, account: &str, cli: u32) -> Live {
        Live {
            provider: Provider::Claude,
            session_id: id.into(),
            parent_session_id: None,
            account_id: account.into(),
            account_label: Some(format!("{account} label")),
            other_sign_in: false,
            name: Some(format!("{id} name")),
            project: Some("Warden".into()),
            cli,
            last_event_at: 1,
        }
    }

    fn value(values: &[(SeriesInfo, u64)], kind: Kind, session: Option<&str>) -> Option<u64> {
        values
            .iter()
            .find(|(i, _)| i.kind == kind && i.session_id.as_deref() == session)
            .map(|(_, v)| *v / MIB)
    }

    fn sample(
        procs: &[Proc],
        live: &[Live],
        own: u32,
        left: &mut LeftRunning,
    ) -> Vec<(SeriesInfo, u64)> {
        sample_at(procs, live, own, left, 1_000_000)
    }

    fn sample_at(
        procs: &[Proc],
        live: &[Live],
        own: u32,
        left: &mut LeftRunning,
        sampled_at_ms: u64,
    ) -> Vec<(SeriesInfo, u64)> {
        let p = plan(procs, live, own, left, sampled_at_ms);
        let values = p.values(procs);
        left.update(&p);
        values
    }

    fn save_links(recorder: &mut Recorder, left: &LeftRunning) -> io::Result<()> {
        if let Some(write) = recorder.prepare_links(left)? {
            write.write()?;
            recorder.links_saved();
        }
        Ok(())
    }

    #[test]
    fn each_session_counts_its_tree_and_a_delegated_run_counts_once() {
        // Explorer 1 > main's CLI 10 > node 11 > delegated CLI 20 > its node 21; main's CLI 10 > shell 12.
        let procs = [
            proc(1, None, 1, 50),
            proc(10, Some(1), 10, 300),
            proc(11, Some(10), 11, 100),
            proc(12, Some(10), 12, 5),
            proc(20, Some(11), 20, 200),
            proc(21, Some(20), 21, 80),
            proc(99, Some(1), 5, 40),
            proc(98, Some(99), 6, 7),
            proc(300, Some(99), 300, 1),
        ];
        let sessions = [live("main-s", "main", 10), live("run", "a1", 20)];
        let mut left = LeftRunning::default();
        let p = plan(&procs, &sessions, 99, &left, 1_000_000);
        assert_eq!(p.pids(), vec![10, 11, 12, 20, 21, 98, 99, 300]);
        let values = sample(&procs, &sessions, 99, &mut left);
        assert_eq!(value(&values, Kind::Session, Some("main-s")), Some(405));
        assert_eq!(value(&values, Kind::Session, Some("run")), Some(280));
        // Warden and what it started (its WebView2), never a session's CLI it started.
        assert_eq!(value(&values, Kind::Warden, None), Some(48));
        let run = values
            .iter()
            .find(|(i, _)| i.session_id.as_deref() == Some("run"))
            .expect("run");
        assert_eq!(run.0.account_id.as_deref(), Some("a1"));
        assert_eq!(run.0.account.as_deref(), Some("a1 label"));
        assert_eq!(run.0.name.as_deref(), Some("run name"));
        // Explorer is nobody's.
        assert_eq!(values.len(), 3);
    }

    #[test]
    fn a_reused_id_is_never_followed() {
        // 12 names 10 as its parent but started before it: another program had that id.
        let procs = [
            proc(10, None, 100, 300),
            proc(12, Some(10), 50, 900),
            proc(13, Some(10), 100, 9),
        ];
        let mut left = LeftRunning::default();
        let values = sample(&procs, &[live("s", "main", 10)], 1, &mut left);
        assert_eq!(value(&values, Kind::Session, Some("s")), Some(309));
    }

    #[test]
    fn a_session_whose_cli_is_gone_counts_nothing() {
        let procs = [proc(10, None, 1, 300)];
        let mut left = LeftRunning::default();
        let values = sample(&procs, &[live("s", "main", 77)], 1, &mut left);
        assert!(values.is_empty());
    }

    #[test]
    fn another_sign_in_counts_for_no_account() {
        let procs = [proc(10, None, 1, 300)];
        let mut s = live("s", "a1", 10);
        s.other_sign_in = true;
        let mut left = LeftRunning::default();
        let values = sample(&procs, &[s], 1, &mut left);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].0.kind, Kind::OtherSignIn);
        assert_eq!(values[0].0.account_id, None);
        assert_eq!(values[0].0.account, None);
    }

    #[test]
    fn one_cli_counts_for_its_newest_session() {
        let procs = [proc(10, None, 1, 300)];
        let old = live("old", "main", 10);
        let mut new = live("new", "main", 10);
        new.last_event_at = 5;
        let mut left = LeftRunning::default();
        let values = sample(&procs, &[old, new], 1, &mut left);
        assert_eq!(values.len(), 1);
        assert_eq!(value(&values, Kind::Session, Some("new")), Some(300));
    }

    #[test]
    fn what_outlives_its_session_is_left_running_until_it_exits() {
        let mut procs = vec![
            proc(10, None, 1, 300),
            proc(11, Some(10), 2, 100),
            proc(12, Some(11), 3, 50),
        ];
        let mut left = LeftRunning::default();
        let s = [live("s", "main", 10)];
        let values = sample(&procs, &s, 1, &mut left);
        assert_eq!(value(&values, Kind::Session, Some("s")), Some(450));
        assert_eq!(left.len(), 3);
        // The CLI exits; its node keeps running, and starts another process.
        procs.remove(0);
        procs.push(proc(13, Some(12), 9, 25));
        let values = sample(&procs, &[], 1, &mut left);
        assert_eq!(value(&values, Kind::Session, Some("s")), None);
        assert_eq!(value(&values, Kind::Left, Some("s")), Some(175));
        let l = values
            .iter()
            .find(|(i, _)| i.kind == Kind::Left)
            .expect("left");
        assert_eq!(l.0.account_id.as_deref(), Some("main"));
        assert_eq!(l.0.name.as_deref(), Some("s name"));
        // The node exits: its orphan keeps counting.
        procs.retain(|p| p.pid != 11);
        let values = sample(&procs, &[], 1, &mut left);
        assert_eq!(value(&values, Kind::Left, Some("s")), Some(75));
        // An id reused by another program is not the process remembered.
        procs = vec![proc(12, None, 500, 999), proc(13, Some(12), 9, 25)];
        let values = sample(&procs, &[], 1, &mut left);
        assert_eq!(value(&values, Kind::Left, Some("s")), Some(25));
        assert_eq!(left.len(), 1);
        procs.clear();
        sample(&procs, &[], 1, &mut left);
        assert!(!left.any_running());
        assert_eq!(left.len(), 1);
        sample(&procs, &[], 1, &mut left);
        assert!(left.is_empty());
    }

    #[test]
    fn an_already_linked_child_counts_for_its_session_after_its_parent_exits() {
        let mut left = LeftRunning::default();
        let s = [live("s", "main", 10)];
        let cli = proc(10, None, 70, 300);
        let parent = proc(11, Some(10), 90, 5);
        let child = proc(12, Some(11), 95, 40);
        sample_at(&[cli, parent, child], &s, 1, &mut left, 100_000);

        let values = sample_at(&[cli, child], &s, 1, &mut left, 110_000);
        assert_eq!(value(&values, Kind::Session, Some("s")), Some(340));
        assert_eq!(value(&values, Kind::Left, Some("s")), None);

        let values = sample_at(&[child], &[], 1, &mut left, 120_000);
        assert_eq!(value(&values, Kind::Session, Some("s")), None);
        assert_eq!(value(&values, Kind::Left, Some("s")), Some(40));
    }

    #[test]
    fn processes_in_the_tree_count_for_the_clis_newest_session() {
        let procs = [proc(10, None, 1, 300), proc(11, Some(10), 2, 100)];
        let mut left = LeftRunning::default();
        sample(&procs, &[live("s", "main", 10)], 1, &mut left);
        // Claude's /clear: the same CLI, a new session.
        let mut next = live("t", "main", 10);
        next.last_event_at = 9;
        let values = sample(&procs, &[next], 1, &mut left);
        assert_eq!(value(&values, Kind::Session, Some("t")), Some(400));
        assert_eq!(value(&values, Kind::Left, Some("s")), None);
    }

    #[test]
    fn left_running_is_capped() {
        let mut procs = vec![proc(10, None, 1, 1)];
        for pid in 11..(11 + MAX_LEFT as u32 + 50) {
            procs.push(proc(pid, Some(10), 2, 1));
        }
        let mut left = LeftRunning::default();
        sample(&procs, &[live("s", "main", 10)], 1, &mut left);
        assert_eq!(left.len(), MAX_LEFT);

        // A running child gets the space before the parents that have exited.
        let child = proc(11 + MAX_LEFT as u32 + 50, Some(11), 1001, 2);
        let values = sample_at(&[child], &[], 1, &mut left, 1_010_000);
        assert_eq!(value(&values, Kind::Left, Some("s")), Some(2));
        assert_eq!(left.len(), MAX_LEFT);
        assert!(left.any_running());
        let values = sample_at(&[child], &[], 1, &mut left, 1_020_000);
        assert_eq!(value(&values, Kind::Left, Some("s")), Some(2));
        assert_eq!(left.len(), 1);
    }

    #[test]
    fn processes_with_no_start_time_are_not_remembered() {
        let mut left = LeftRunning::default();
        let known = proc(10, None, 1, 5);
        let unknown = proc(11, Some(10), 0, 10);
        let sessions = [live("s", "main", 10), live("t", "main", 11)];
        sample(&[known, unknown], &sessions, 1, &mut left);
        assert_eq!(left.len(), 1);
        assert!(!left.processes.contains_key(&11));

        let unopenable = proc(10, None, 0, 5);
        sample(&[unopenable, unknown], &sessions, 1, &mut left);
        assert!(left.is_empty());
    }

    #[test]
    fn processes_with_no_start_time_are_not_serialized() {
        let mut left = LeftRunning::default();
        let cli = proc(10, None, 1, 5);
        let child = proc(11, Some(10), 2, 15);
        sample(&[cli, child], &[live("s", "main", 10)], 1, &mut left);
        left.processes.get_mut(&11).expect("child").started = 0;
        let saved = LinkSnapshot::from_left(&left).expect("the CLI is still saved");
        assert_eq!(saved.processes, [(10, 1, 0)]);
    }

    #[test]
    fn saved_processes_with_no_start_time_are_skipped_and_removed() {
        let dir = Dir::new("unknown-saved-process");
        let mut recorder = Recorder::new_in(dir.path(), T0, 0);
        let mut left = LeftRunning::default();
        let cli = proc(10, None, 1, 5);
        sample(&[cli], &[live("s", "main", 10)], 1, &mut left);
        save_links(&mut recorder, &left).expect("save");
        let path = recorder.dir().join(LINK_FILE);
        let mut saved: LinkSnapshot =
            serde_json::from_slice(&fs::read(&path).expect("file")).expect("saved links");
        saved.processes.push((11, 0, 0));
        fs::write(&path, serde_json::to_vec(&saved).expect("json")).expect("plant unknown start");

        let mut restored = recorder.load_links().expect("load");
        assert_eq!(restored.len(), 1);
        let values = sample(&[cli, proc(11, None, 0, 40)], &[], 1, &mut restored);
        assert_eq!(value(&values, Kind::Left, Some("s")), Some(5));
        save_links(&mut recorder, &restored).expect("rewrite without the unknown process");
        let (saved, _) = LinkSnapshot::read(recorder.dir()).expect("clean links");
        assert_eq!(saved.processes, [(10, 1, 0)]);
    }

    #[test]
    fn long_session_ids_match_in_memory_and_by_the_saved_series_after_restart() {
        let dir = Dir::new("long-linked-session-id");
        let mut recorder = Recorder::new_in(dir.path(), T0, 0);
        let mut left = LeftRunning::default();
        let prefix = "s".repeat(MAX_NAME);
        let id = format!("{prefix}-full-id");
        let cli = proc(10, None, 1, 5);
        let child = proc(11, Some(10), 2, 25);
        sample(&[cli, child], &[live(&id, "main", 10)], 1, &mut left);
        let orphan = proc(11, None, 2, 25);
        let values = sample(&[cli, orphan], &[live(&id, "main", 10)], 1, &mut left);
        assert_eq!(value(&values, Kind::Session, Some(&id)), Some(30));

        save_links(&mut recorder, &left).expect("save");
        let (saved, mut restored) = LinkSnapshot::read(recorder.dir()).expect("load");
        assert_eq!(saved.series[0].session_id.as_deref(), Some(prefix.as_str()));
        sample(&[orphan], &[], 1, &mut restored);
        let values = sample(&[cli, orphan], &[live(&id, "main", 10)], 1, &mut restored);
        assert_eq!(value(&values, Kind::Session, Some(&id)), Some(30));

        let (_, mut restored) = LinkSnapshot::read(recorder.dir()).expect("load again");
        let other = format!("{prefix}-other-id");
        let another_cli = proc(12, None, 3, 10);
        let values = sample(
            &[cli, orphan, another_cli],
            &[live(&id, "main", 10), live(&other, "main", 12)],
            1,
            &mut restored,
        );
        assert_eq!(value(&values, Kind::Left, Some(&prefix)), Some(25));
    }

    #[test]
    fn process_links_round_trip_with_one_series_and_without_exits() {
        let dir = Dir::new("saved-process-links");
        let mut recorder = Recorder::new_in(dir.path(), T0, 0);
        let mut session = live("s", "main", 10);
        session.name = Some(r"Read the work\notes.txt".into());
        let cli = proc(10, None, 1, 5);
        let children = [proc(11, Some(10), 2, 30), proc(12, Some(10), 3, 40)];
        let mut left = LeftRunning::default();
        sample_at(
            &[cli, children[0], children[1]],
            &[session],
            1,
            &mut left,
            4_000,
        );
        sample_at(&children, &[], 1, &mut left, 5_000);
        assert_eq!(
            left.len(),
            3,
            "the missing CLI is still remembered in memory"
        );
        save_links(&mut recorder, &left).expect("save");

        let (saved, mut restored) = LinkSnapshot::read(recorder.dir()).expect("load");
        assert_eq!(saved.sampled_at_ms, 5_000);
        assert_eq!(saved.series.len(), 1);
        assert_eq!(saved.series[0].name.as_deref(), Some("Read the …"));
        assert_eq!(saved.processes, [(11, 2, 0), (12, 3, 0)]);
        assert!(restored.any_running());
        let replacement = proc(12, None, 6, 100);
        let values = sample_at(&[children[0], replacement], &[], 1, &mut restored, 7_000);
        assert_eq!(value(&values, Kind::Left, Some("s")), Some(30));
        assert_eq!(restored.len(), 1);

        let (_, mut restored) = LinkSnapshot::read(recorder.dir()).expect("load again");
        let new_cli = proc(20, None, 6, 50);
        let values = sample_at(
            &[children[0], new_cli],
            &[live("s", "main", 20)],
            1,
            &mut restored,
            7_000,
        );
        assert_eq!(value(&values, Kind::Session, Some("s")), Some(80));
        assert_eq!(value(&values, Kind::Left, Some("s")), None);
    }

    #[test]
    fn unchanged_links_are_written_only_when_finishing() {
        let dir = Dir::new("unchanged-process-links");
        let mut recorder = Recorder::new_in(dir.path(), T0, 0);
        let mut left = LeftRunning::default();
        let cli = proc(10, None, 1, 5);
        let sessions = [live("s", "main", 10)];
        sample_at(&[cli], &sessions, 1, &mut left, 2_000);
        save_links(&mut recorder, &left).expect("first save");
        let path = recorder.dir().join(LINK_FILE);
        let before = fs::read(&path).expect("saved links");
        sample_at(&[cli], &sessions, 1, &mut left, 3_000);
        assert!(recorder.prepare_links(&left).expect("same links").is_none());
        assert_eq!(fs::read(&path).expect("unchanged links"), before);
        recorder
            .finish_links()
            .expect("finish")
            .expect("pending links")
            .write()
            .expect("write on finish");
        let (saved, _) = LinkSnapshot::read(recorder.dir()).expect("load");
        assert_eq!(saved.sampled_at_ms, 3_000);
        assert_eq!(days(recorder.dir()), Vec::<String>::new());
        prune(recorder.dir(), "2026-11-24");
        assert!(path.exists());

        sample_at(&[], &[], 1, &mut left, 4_000);
        save_links(&mut recorder, &left).expect("remove empty links");
        assert!(!path.exists());
    }

    #[test]
    fn a_restored_missing_parent_uses_the_saved_cutoff() {
        let dir = Dir::new("restored-parent-cutoff");
        let mut recorder = Recorder::new_in(dir.path(), T0, 0);
        let mut left = LeftRunning::default();
        let cli = proc(10, None, 1, 5);
        sample_at(&[cli], &[live("s", "main", 10)], 1, &mut left, 100_500);
        save_links(&mut recorder, &left).expect("save");

        let mut restored = recorder.load_links().expect("load");
        let before = proc(11, Some(10), 99, 25);
        let straddles = proc(12, Some(10), 100, 30);
        let after = proc(13, Some(10), 102, 35);
        let values = sample_at(&[before, straddles, after], &[], 1, &mut restored, 900_000);
        assert_eq!(value(&values, Kind::Left, Some("s")), Some(25));
        assert_eq!(restored.processes[&10].first_missing_at_ms, Some(100_500));
        let another_before = proc(14, Some(10), 98, 20);
        let values = sample_at(
            &[before, another_before, after],
            &[],
            1,
            &mut restored,
            910_000,
        );
        assert_eq!(value(&values, Kind::Left, Some("s")), Some(45));
        assert!(!restored.processes.contains_key(&10));
    }

    #[test]
    fn a_clock_set_back_clamps_the_restored_cutoff() {
        let dir = Dir::new("restored-cutoff-clock-set-back");
        let mut recorder = Recorder::new_in(dir.path(), T0, 0);
        let mut left = LeftRunning::default();
        let cli = proc(10, None, 90, 5);
        sample_at(&[cli], &[live("s", "main", 10)], 1, &mut left, 200_500);
        save_links(&mut recorder, &left).expect("save");

        let mut restored = recorder.load_links().expect("load");
        let before = proc(11, Some(10), 99, 25);
        let straddles = proc(12, Some(10), 100, 30);
        let values = sample_at(&[before, straddles], &[], 1, &mut restored, 100_500);
        assert_eq!(value(&values, Kind::Left, Some("s")), Some(25));
        assert_eq!(restored.processes[&10].first_missing_at_ms, Some(100_500));
        let after = proc(13, Some(10), 101, 35);
        let values = sample_at(&[before, after], &[], 1, &mut restored, 110_000);
        assert_eq!(value(&values, Kind::Left, Some("s")), Some(25));
    }

    #[test]
    fn crashed_link_writes_are_cleaned_up_on_load_and_write() {
        let dir = Dir::new("leftover-process-link-temp");
        let mut recorder = Recorder::new_in(dir.path(), T0, 0);
        fs::create_dir(recorder.dir()).expect("memory folder");
        let temp = recorder.dir().join("linked.json.12-3.tmp");
        let unrelated = recorder.dir().join("settings.json.12-3.tmp");
        fs::write(&unrelated, "unrelated file").expect("unrelated file");
        fs::write(&temp, "interrupted write").expect("temp file");
        assert!(recorder.load_links().expect("missing links").is_empty());
        assert!(!temp.exists());

        fs::write(&temp, "interrupted write").expect("temp file");
        let mut left = LeftRunning::default();
        sample(
            &[proc(10, None, 1, 5)],
            &[live("s", "main", 10)],
            1,
            &mut left,
        );
        let write = recorder
            .prepare_links(&left)
            .expect("prepare")
            .expect("pending links");
        assert!(temp.is_file(), "preparation does no disk writes");
        write.write().expect("save");
        assert!(!temp.exists());
        assert!(unrelated.is_file());
    }

    #[test]
    fn bad_link_files_are_ignored_and_replaced_on_the_next_write() {
        let dir = Dir::new("bad-process-links");
        let mut recorder = Recorder::new_in(dir.path(), T0, 0);
        let path = recorder.dir().join(LINK_FILE);
        assert!(recorder.load_links().expect("missing is normal").is_empty());
        let mut left = LeftRunning::default();
        sample_at(
            &[proc(10, None, 1, 5)],
            &[live("s", "main", 10)],
            1,
            &mut left,
            2_000,
        );
        save_links(&mut recorder, &left).expect("save");
        let good = fs::read_to_string(&path).expect("saved links");
        for bad in [
            "{".to_string(),
            good.replace("\"version\":1", "\"version\":2"),
            good.replace("[10,1,0]", "[10,1,1]"),
            format!("{good}{}", " ".repeat(MAX_LINK_BYTES as usize)),
        ] {
            fs::write(&path, bad).expect("bad file");
            assert_eq!(
                recorder.load_links().expect_err("bad links").kind(),
                io::ErrorKind::InvalidData
            );
            save_links(&mut recorder, &left).expect("replace bad links");
            assert_eq!(fs::read_to_string(&path).expect("replacement"), good);
        }
        // A directory in place of the file cannot be read, and is left alone.
        fs::remove_file(&path).expect("remove file");
        fs::create_dir(&path).expect("directory in its place");
        assert!(recorder.load_links().is_err());
        assert!(save_links(&mut recorder, &left).is_err());
    }

    #[cfg(any(windows, unix))]
    #[test]
    fn a_linked_process_file_is_left_alone() {
        let dir = Dir::new("linked-process-file");
        let mut recorder = Recorder::new_in(dir.path(), T0, 0);
        let target = dir.path().join("outside.json");
        fs::write(&target, "untouched").expect("target");
        fs::create_dir(recorder.dir()).expect("memory folder");
        let path = recorder.dir().join(LINK_FILE);
        #[cfg(windows)]
        if let Err(e) = std::os::windows::fs::symlink_file(&target, &path) {
            // Without Developer Mode creating a file symlink requires administrator privileges.
            assert_eq!(e.raw_os_error(), Some(1314));
            return;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &path).expect("symlink");

        assert_eq!(
            recorder.load_links().expect_err("refuse read").to_string(),
            linked_error().to_string()
        );
        let mut left = LeftRunning::default();
        assert!(save_links(&mut recorder, &left).is_err(), "refuse removal");
        sample_at(
            &[proc(10, None, 1, 5)],
            &[live("s", "main", 10)],
            1,
            &mut left,
            2_000,
        );
        assert!(
            save_links(&mut recorder, &left).is_err(),
            "refuse overwrite"
        );
        assert!(linked(&path));
        assert_eq!(fs::read_to_string(&target).expect("target"), "untouched");
    }

    #[test]
    fn a_new_child_inherits_its_missing_parents_session() {
        let mut left = LeftRunning::default();
        let s = [live("s", "main", 10)];
        let cli = proc(10, None, 70, 300);
        let parent = proc(11, Some(10), 90, 5);
        sample_at(&[cli, parent], &s, 1, &mut left, 100_000);

        // The interval was longer than usual. The parent's grandchild follows the newly linked child.
        let child = proc(12, Some(11), 115, 40);
        let grandchild = proc(13, Some(12), 116, 30);
        let values = sample_at(&[cli, child, grandchild], &s, 1, &mut left, 123_250);
        assert_eq!(value(&values, Kind::Session, Some("s")), Some(370));
        assert_eq!(value(&values, Kind::Left, Some("s")), None);

        // The link outlives the parent's record. When the CLI exits, both become Left running.
        let values = sample_at(&[cli, child, grandchild], &s, 1, &mut left, 150_000);
        assert_eq!(value(&values, Kind::Session, Some("s")), Some(370));
        let values = sample_at(&[child, grandchild], &[], 1, &mut left, 160_000);
        assert_eq!(value(&values, Kind::Session, Some("s")), None);
        assert_eq!(value(&values, Kind::Left, Some("s")), Some(70));
        let values = sample_at(&[grandchild], &[], 1, &mut left, 170_000);
        assert_eq!(value(&values, Kind::Left, Some("s")), Some(30));
    }

    #[test]
    fn a_missing_parent_is_only_followed_up_to_the_first_snapshot_without_it() {
        let mut left = LeftRunning::default();
        let s = [live("s", "main", 10)];
        let cli = proc(10, None, 70, 300);
        let parent = proc(11, Some(10), 90, 5);
        sample_at(&[cli, parent], &s, 1, &mut left, 100_500);

        let too_early = proc(12, Some(11), 89, 100);
        let present_at_cutoff = proc(13, Some(11), 102, 40);
        let unknown_parent = proc(15, Some(14), 101, 100);
        let values = sample_at(
            &[cli, too_early, present_at_cutoff, unknown_parent],
            &s,
            1,
            &mut left,
            102_125,
        );
        assert_eq!(values.len(), 1);
        assert_eq!(value(&values, Kind::Session, Some("s")), Some(340));
        assert_eq!(left.processes[&11].first_missing_at_ms, Some(102_125));

        // The first missing sample fixes the cutoff, even though the next sample is only a few seconds away.
        let in_window = proc(16, Some(11), 101, 20);
        let straddles_cutoff = proc(17, Some(11), 102, 100);
        let too_late = proc(18, Some(11), 103, 100);
        let values = sample_at(
            &[
                cli,
                present_at_cutoff,
                in_window,
                straddles_cutoff,
                too_late,
            ],
            &s,
            1,
            &mut left,
            104_200,
        );
        assert_eq!(values.len(), 1);
        assert_eq!(value(&values, Kind::Session, Some("s")), Some(360));
        assert!(!left.processes.contains_key(&11));

        let old_enough = proc(19, Some(11), 101, 100);
        let values = sample_at(&[cli, old_enough], &s, 1, &mut left, 104_500);
        assert_eq!(value(&values, Kind::Session, Some("s")), Some(300));
    }

    #[test]
    fn a_child_can_inherit_from_a_remembered_parent_if_the_pid_holder_is_newer() {
        let mut left = LeftRunning::default();
        let s = [live("s", "main", 10)];
        let cli = proc(10, None, 70, 300);
        let parent = proc(11, Some(10), 90, 5);
        sample_at(&[cli, parent], &s, 1, &mut left, 100_000);

        let child = proc(12, Some(11), 105, 40);
        let replacement = proc(11, None, 108, 100);
        let replacements_child = proc(13, Some(11), 109, 100);
        let values = sample_at(
            &[cli, child, replacement, replacements_child],
            &s,
            1,
            &mut left,
            110_000,
        );
        assert_eq!(values.len(), 1);
        assert_eq!(value(&values, Kind::Session, Some("s")), Some(340));
    }

    #[test]
    fn an_observed_pid_reuse_invalidates_the_missing_parent_record() {
        let mut left = LeftRunning::default();
        let s = [live("s", "main", 10)];
        let cli = proc(10, None, 70, 300);
        let parent = proc(11, Some(10), 90, 5);
        sample_at(&[cli, parent], &s, 1, &mut left, 100_000);

        let replacement = proc(11, None, 103, 100);
        let its_child = proc(12, Some(11), 105, 100);
        let values = sample_at(&[cli, replacement, its_child], &s, 1, &mut left, 110_000);
        assert_eq!(values.len(), 1);
        assert_eq!(value(&values, Kind::Session, Some("s")), Some(300));
        let values = sample_at(&[cli, its_child], &s, 1, &mut left, 120_000);
        assert_eq!(values.len(), 1);
        assert_eq!(value(&values, Kind::Session, Some("s")), Some(300));
    }

    fn reading(at_ms: i64, values: Vec<(SeriesInfo, u64)>) -> Reading {
        Reading {
            at_ms,
            total: 32 * GIB,
            available: 20 * GIB,
            values,
        }
    }

    fn info(id: &str) -> SeriesInfo {
        SeriesInfo::of(&live(id, "main", 1), Kind::Session)
    }

    #[test]
    fn a_minute_keeps_each_series_highest_value() {
        let dir = Dir::new("memory-minute");
        // 10:00 UTC, in UTC.
        let at = 1_795_000_000_000 - 1_795_000_000_000 % DAY_MS + 10 * 3_600_000;
        let mut r = Recorder::new_in(dir.path(), at, 0);
        r.record_in(
            &reading(at, vec![(info("a"), 100 * MIB), (info("b"), 50 * MIB)]),
            0,
        )
        .expect("record");
        r.record_in(
            &reading(
                at + 10_000,
                vec![(info("a"), 80 * MIB), (info("b"), 90 * MIB)],
            ),
            0,
        )
        .expect("record");
        let (date, m) = local_minute(at, 0);
        assert_eq!(m, 600);
        // Nothing on disk before the minute is over; today's answer has it already.
        assert!(!day_file(r.dir(), &date).exists());
        let today = r.today_day(&date);
        assert_eq!(today.minutes.len(), 1);
        assert_eq!(today.minutes[0].s, vec![(1, 100 * 1024), (2, 90 * 1024)]);
        assert_eq!(today.minutes[0].sum, 170 * 1024);
        assert_eq!(today.minutes[0].total, 32 * 1024 * 1024);
        assert_eq!(today.minutes[0].used, 12 * 1024 * 1024);
        // The next minute writes it.
        r.record_in(&reading(at + 60_000, vec![(info("a"), 10 * MIB)]), 0)
            .expect("record");
        let day = read_day(r.dir(), &date);
        assert_eq!(day.series.len(), 2);
        assert_eq!(day.series[0].info.session_id.as_deref(), Some("a"));
        assert_eq!(day.minutes.len(), 1);
        assert_eq!(day.minutes[0].m, 600);
        r.finish().expect("finish");
        let day = read_day(r.dir(), &date);
        assert_eq!(
            day.minutes.iter().map(|m| m.m).collect::<Vec<_>>(),
            [600, 601]
        );
        assert_eq!(day.minutes[1].s, vec![(1, 10 * 1024)]);
        // A restart reads today back, and goes on with the same numbers.
        let mut again = Recorder::new_in(dir.path(), at + 120_000, 0);
        assert_eq!(again.today_day(&date).minutes.len(), 2);
        again
            .record_in(
                &reading(at + 120_000, vec![(info("b"), MIB), (info("c"), MIB)]),
                0,
            )
            .expect("record");
        again.finish().expect("finish");
        let day = read_day(again.dir(), &date);
        assert_eq!(day.series.len(), 3);
        assert_eq!(day.minutes[2].s, vec![(2, 1024), (3, 1024)]);
    }

    #[test]
    fn a_renamed_session_is_written_again_with_its_number() {
        let dir = Dir::new("memory-rename");
        let mut r = Recorder::new_in(dir.path(), T0, 0);
        r.record_in(&reading(T0, vec![(info("a"), MIB)]), 0)
            .expect("record");
        let mut named = info("a");
        named.name = Some("Fix the login page".into());
        r.record_in(&reading(T0 + 60_000, vec![(named, MIB)]), 0)
            .expect("record");
        r.finish().expect("finish");
        let (date, _) = local_minute(T0, 0);
        let day = read_day(r.dir(), &date);
        assert_eq!(day.series.len(), 1);
        assert_eq!(
            day.series[0].info.name.as_deref(),
            Some("Fix the login page")
        );
        let text = fs::read_to_string(day_file(r.dir(), &date)).expect("file");
        assert_eq!(text.matches("\"series\":1").count(), 2);
    }

    #[test]
    fn names_are_capped_and_warden_is_not_in_the_total() {
        let dir = Dir::new("memory-names");
        let mut r = Recorder::new_in(dir.path(), T0, 0);
        let mut long = info("a");
        long.name = Some("x".repeat(500));
        long.project = Some(format!("p\n{}", "y".repeat(500)));
        r.record_in(
            &reading(T0, vec![(long, MIB), (SeriesInfo::warden(), 9 * MIB)]),
            0,
        )
        .expect("record");
        r.finish().expect("finish");
        let (date, _) = local_minute(T0, 0);
        let day = read_day(r.dir(), &date);
        assert_eq!(
            day.series[0].info.name.as_deref().map(str::len),
            Some(MAX_NAME)
        );
        assert_eq!(
            day.series[0]
                .info
                .project
                .as_deref()
                .map(|p| p.chars().count()),
            Some(MAX_NAME)
        );
        assert!(
            !day.series[0]
                .info
                .project
                .as_deref()
                .unwrap_or_default()
                .contains('\n')
        );
        assert_eq!(day.minutes[0].sum, 1024);
        assert_eq!(day.series[1].info.kind, Kind::Warden);
    }

    #[test]
    fn a_day_holds_at_most_its_series_and_minutes() {
        let dir = Dir::new("memory-caps");
        let start = T0 - T0 % DAY_MS;
        let mut r = Recorder::new_in(dir.path(), start, 0);
        for minute in 0..(MAX_MINUTES as i64 + 30) {
            let at = start + minute * 60_000;
            // A new series every minute: more than a day may hold.
            let values = (0..2)
                .map(|k| (info(&format!("s{}", minute * 2 + k)), MIB))
                .collect();
            r.record_in(&reading(at.min(start + DAY_MS - 1), values), 0)
                .expect("record");
        }
        r.finish().expect("finish");
        let (date, _) = local_minute(start, 0);
        let day = read_day(r.dir(), &date);
        assert_eq!(day.series.len(), MAX_SERIES);
        assert_eq!(day.minutes.len(), MAX_MINUTES);
        let text = fs::read_to_string(day_file(r.dir(), &date)).expect("file");
        assert!(text.lines().filter(|l| l.starts_with("{\"m\":")).count() <= MAX_MINUTES);
        assert!(text.len() as u64 <= MAX_DAY_BYTES);
        assert!(
            day.minutes
                .iter()
                .all(|m| m.s.iter().all(|(n, _)| usize::from(*n) <= MAX_SERIES))
        );
    }

    #[test]
    fn a_new_day_starts_a_new_file() {
        let dir = Dir::new("memory-midnight");
        let midnight = T0 - T0 % DAY_MS + DAY_MS;
        let mut r = Recorder::new_in(dir.path(), midnight - 30_000, 0);
        r.record_in(&reading(midnight - 30_000, vec![(info("a"), MIB)]), 0)
            .expect("record");
        r.record_in(&reading(midnight + 5_000, vec![(info("b"), MIB)]), 0)
            .expect("record");
        r.finish().expect("finish");
        let before = local_minute(midnight - 30_000, 0).0;
        let after = local_minute(midnight, 0).0;
        assert_eq!(days(r.dir()), vec![before.clone(), after.clone()]);
        let day = read_day(r.dir(), &after);
        // Numbers start again each day.
        assert_eq!(day.series[0].series, 1);
        assert_eq!(day.series[0].info.session_id.as_deref(), Some("b"));
        assert_eq!(day.minutes[0].m, 0);
        assert_eq!(read_day(r.dir(), &before).minutes[0].m, 1439);
    }

    #[test]
    fn old_and_big_days_are_pruned() {
        let dir = Dir::new("memory-prune");
        let mem = dir.path().join(DIR);
        fs::create_dir_all(&mem).expect("mkdir");
        let today_n = parse_date("2026-11-23").expect("date");
        for back in 0..40 {
            let date = date_text(today_n - back);
            fs::write(day_file(&mem, &date), "x".repeat(400 * 1024)).expect("write");
        }
        // Not Warden's: never touched.
        fs::write(mem.join("notes.txt"), "keep").expect("write");
        fs::write(mem.join("memory-2026-1-1.jsonl"), "keep").expect("write");
        prune(&mem, "2026-11-23");
        let left = days(&mem);
        assert_eq!(left.len(), KEEP_DAYS as usize);
        assert_eq!(left.first().map(String::as_str), Some("2026-10-25"));
        assert!(mem.join("notes.txt").exists());
        assert!(mem.join("memory-2026-1-1.jsonl").exists());
        // Big days: the oldest go until the rest leave room for a full day.
        for date in &left {
            fs::write(day_file(&mem, date), "x".repeat(2 * 1024 * 1024)).expect("write");
        }
        prune(&mem, "2026-11-23");
        let total: u64 = days(&mem)
            .iter()
            .map(|d| fs::metadata(day_file(&mem, d)).expect("meta").len())
            .sum();
        assert!(total <= MAX_TOTAL_BYTES, "{total}");
        assert!(days(&mem).contains(&"2026-11-23".to_owned()));
        assert!(days(&mem).contains(&"2026-11-22".to_owned()));
        assert!(!days(&mem).contains(&"2026-10-25".to_owned()));
    }

    #[test]
    fn a_cut_last_line_is_skipped_and_the_next_starts_on_its_own() {
        let dir = Dir::new("memory-cut");
        let mem = dir.path().join(DIR);
        fs::create_dir_all(&mem).expect("mkdir");
        let (date, _) = local_minute(T0, 0);
        fs::write(
            day_file(&mem, &date),
            "{\"m\":1,\"total\":10,\"used\":5,\"sum\":1,\"s\":[]}\n{\"m\":2,\"tot",
        )
        .expect("write");
        let mut r = Recorder::new_in(dir.path(), T0, 0);
        assert_eq!(r.today_day(&date).minutes.len(), 1);
        r.record_in(&reading(T0, vec![]), 0).expect("record");
        r.finish().expect("finish");
        assert_eq!(read_day(&mem, &date).minutes.len(), 2);
    }

    #[test]
    fn a_day_answer_has_its_gaps_and_figures() {
        let s = |n: u16, kind: Kind| Series {
            series: n,
            info: SeriesInfo { kind, ..info("x") },
        };
        let minute = |m: u16, sum: u32, s: Vec<(u16, u32)>| Minute {
            m,
            total: 1000,
            used: 500,
            sum,
            s,
        };
        let day = Day {
            series: vec![s(1, Kind::Session), s(2, Kind::Left)],
            minutes: vec![
                minute(600, 100, vec![(1, 100)]),
                minute(601, 300, vec![(1, 200), (2, 100)]),
                minute(605, 150, vec![(2, 150)]),
            ],
        };
        let answer = MemoryDay::new("2026-11-23", true, vec!["2026-11-23".into()], day.clone());
        assert_eq!(answer.gaps, vec![Gap { from: 602, to: 605 }]);
        assert_eq!(answer.summary.first, Some(600));
        assert_eq!(answer.summary.last, Some(605));
        assert_eq!(answer.summary.peak, Some(300));
        assert_eq!(answer.summary.peak_at, Some(601));
        assert_eq!(answer.summary.peak_share, Some(30.0));
        assert_eq!(answer.summary.now, Some(150));
        assert_eq!(answer.summary.now_share, Some(15.0));
        assert_eq!(
            answer.summary.series,
            vec![
                SeriesSummary {
                    series: 1,
                    now: Some(0),
                    peak: 200,
                    peak_at: 601
                },
                SeriesSummary {
                    series: 2,
                    now: Some(150),
                    peak: 150,
                    peak_at: 605
                },
            ]
        );
        // A past day has no "now".
        let past = MemoryDay::new("2026-11-22", false, Vec::new(), day);
        assert_eq!(past.summary.now, None);
        assert!(past.summary.series.iter().all(|s| s.now.is_none()));
        let empty = MemoryDay::new("2026-11-21", false, Vec::new(), Day::default());
        assert_eq!(empty.summary.peak, None);
        assert!(empty.minutes.is_empty() && empty.gaps.is_empty());
    }

    fn minute_lines(r: &Recorder, date: &str) -> usize {
        fs::read_to_string(day_file(r.dir(), date))
            .expect("file")
            .lines()
            .filter(|l| l.starts_with("{\"m\":"))
            .count()
    }

    #[test]
    fn a_minute_is_written_once_however_often_it_comes() {
        let dir = Dir::new("memory-once");
        let at = T0 - T0 % DAY_MS + 10 * 3_600_000;
        let (date, _) = local_minute(at, 0);
        // Warden starts again within the same minute: one line, the higher values kept in memory.
        let mut r = Recorder::new_in(dir.path(), at, 0);
        r.record_in(&reading(at, vec![(info("a"), MIB)]), 0)
            .expect("record");
        r.finish().expect("finish");
        let mut again = Recorder::new_in(dir.path(), at + 20_000, 0);
        again
            .record_in(&reading(at + 20_000, vec![(info("a"), 5 * MIB)]), 0)
            .expect("record");
        again.finish().expect("finish");
        assert_eq!(minute_lines(&again, &date), 1);
        assert_eq!(again.today_day(&date).minutes[0].s, vec![(1, 5 * 1024)]);
        // Daylight saving ends: 02:00 to 03:00 comes twice. The repeated hour adds no line.
        let dst = Dir::new("memory-dst");
        let two = T0 - T0 % DAY_MS + 2 * 3_600_000;
        let mut r = Recorder::new_in(dst.path(), two, 0);
        for minute in 0..60 {
            r.record_in(&reading(two + minute * 60_000, vec![]), 0)
                .expect("record");
        }
        let (date, _) = local_minute(two, 0);
        assert_eq!(r.gathering_minute(&date), Some(2 * 60 + 59));
        // The clock goes back an hour: the same local minutes, an hour later in UTC; the minute being gathered comes
        // before minutes already recorded.
        r.record_in(&reading(two + 60 * 60_000, vec![]), -3600)
            .expect("record");
        assert_eq!(r.gathering_minute(&date), Some(2 * 60));
        assert_eq!(r.gathering_minute("1999-01-01"), None);
        for minute in 0..61 {
            r.record_in(
                &reading(two + (60 + minute) * 60_000, vec![(info("b"), MIB)]),
                -3600,
            )
            .expect("record");
        }
        r.finish().expect("finish");
        let (date, _) = local_minute(two, 0);
        assert_eq!(minute_lines(&r, &date), 61);
        let day = read_day(r.dir(), &date);
        assert_eq!(day.minutes.len(), 61);
        // A series first seen in the repeated hour is still defined in the file.
        assert!(
            day.series
                .iter()
                .any(|s| s.info.session_id.as_deref() == Some("b"))
        );
    }

    #[test]
    fn a_line_cut_inside_a_character_spoils_only_itself() {
        let dir = Dir::new("memory-utf8");
        let mem = dir.path().join(DIR);
        fs::create_dir_all(&mem).expect("mkdir");
        let (date, _) = local_minute(T0, 0);
        let mut text = b"{\"m\":1,\"total\":10,\"used\":5,\"sum\":1,\"s\":[]}\n".to_vec();
        let series = "{\"series\":1,\"kind\":\"session\",\"name\":\"Caf\u{e9}\u{1f600}";
        // Cut in the middle of the emoji.
        text.extend_from_slice(&series.as_bytes()[..series.len() - 2]);
        fs::write(day_file(&mem, &date), &text).expect("write");
        let mut r = Recorder::new_in(dir.path(), T0, 0);
        assert_eq!(r.today_day(&date).minutes.len(), 1);
        r.record_in(&reading(T0, vec![(info("a"), MIB)]), 0)
            .expect("record");
        r.finish().expect("finish");
        let day = read_day(&mem, &date);
        assert_eq!(day.minutes.len(), 2);
        assert_eq!(day.series.len(), 1);
    }

    #[test]
    fn after_a_failed_write_the_file_is_the_truth() {
        let dir = Dir::new("memory-fail");
        let mem = dir.path().join(DIR);
        let at = T0 - T0 % DAY_MS + 10 * 3_600_000;
        let (date, _) = local_minute(at, 0);
        // The day file cannot be opened for writing (a folder stands in its place).
        fs::create_dir_all(day_file(&mem, &date)).expect("in the way");
        let mut r = Recorder::new_in(dir.path(), at, 0);
        r.record_in(&reading(at, vec![(info("a"), MIB)]), 0)
            .expect("record");
        assert!(r.finish().is_err());
        fs::remove_dir(day_file(&mem, &date)).expect("out of the way");
        // The next minute names series "a" again, and its line is written with it.
        r.record_in(&reading(at + 60_000, vec![(info("a"), MIB)]), 0)
            .expect("record");
        r.finish().expect("finish");
        let day = read_day(&mem, &date);
        assert_eq!(day.series.len(), 1);
        assert_eq!(day.minutes.len(), 1);
        assert_eq!(day.minutes[0].s, vec![(1, 1024)]);
    }

    #[test]
    fn names_never_keep_a_path_or_a_program() {
        assert_eq!(
            masked(r"Investigate %USERPROFILE%\work\app and /srv/x then ..\app ~/notes"),
            "Investigate … and … then … …"
        );
        assert_eq!(masked("Fix warden.exe crash"), "Fix … crash");
        assert_eq!(
            masked("Fix the A/B test\non two lines"),
            "Fix the A/B test on two lines"
        );
        let mut s = live("s", "main", 1);
        s.name = Some(r"Look at data\payroll.csv".into());
        assert_eq!(
            SeriesInfo::of(&s, Kind::Session).name.as_deref(),
            Some("Look at …")
        );
    }

    #[test]
    fn the_local_day_and_minute() {
        let midnight = T0 - T0 % DAY_MS;
        assert_eq!(local_minute(midnight, 0).1, 0);
        assert_eq!(local_minute(midnight + 61_000, 0).1, 1);
        // An hour ahead of UTC, 23:30 UTC is 00:30 the next day.
        let late = midnight + DAY_MS - 30 * 60_000;
        assert_eq!(local_minute(late, 3600).1, 30);
        assert_ne!(local_minute(late, 3600).0, local_minute(late, 0).0);
    }
}
