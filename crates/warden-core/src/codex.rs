//! Codex accounts: identity from the login file, usage from the session logs.
//!
//! Codex keeps the signed-in email and organisation only inside its login file (`auth.json`), in the claims of the
//! ID token. [`read_identity_from_login_file`] is the one place that opens that file: it decodes those claims and
//! drops everything else, tokens included, before returning.
//!
//! Usage comes from the `token_count` events Codex writes to its session logs (`sessions/YYYY/MM/DD/*.jsonl`).
//! [`LogTail`] reads only what was appended since the last read, when the server's file watcher reports a change.

use crate::model::{Identity, Limit, Usage};
use crate::secret::Secret;
use crate::time::parse_rfc3339_ms;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

const LOGIN_FILE: &str = "auth.json";
pub const SESSIONS_DIR: &str = "sessions";

/// Whether the folder holds a Codex login (the login file exists). Does not open it.
pub fn has_login(folder: &Path) -> bool {
    folder.join(LOGIN_FILE).is_file()
}

/// Whether the folder looks like a Codex account folder at all.
pub fn looks_like_account(folder: &Path) -> bool {
    has_login(folder) || folder.join("config.toml").is_file() || folder.join(SESSIONS_DIR).is_dir()
}

#[derive(Deserialize)]
struct LoginFile {
    tokens: Option<LoginTokens>,
}

/// Only the ID token is read; the access and refresh tokens are skipped by the parser, never kept.
#[derive(Deserialize)]
struct LoginTokens {
    id_token: Option<Zeroizing<String>>,
}

#[derive(Deserialize)]
struct AccessFile {
    tokens: Option<AccessTokens>,
}

/// For the banked resets' request only: the access token and account id; the ID and refresh tokens are skipped.
#[derive(Deserialize)]
struct AccessTokens {
    access_token: Option<Secret>,
    account_id: Option<String>,
}

/// One reading of a Codex folder's login file: who it is signed in as, and the access the banked resets' request needs,
/// both from the same bytes, so the token sent is the one of the identity checked.
pub struct LoginWithAccess {
    /// `None`: signed out, or signed in with an API key.
    pub identity: Option<Identity>,
    /// The access token and ChatGPT account id, for the one request that asks for the account's banked resets
    /// (`codex_resets`); nothing else may use them, and they are not kept beyond that call. `None` without both.
    pub access: Option<(Secret, String)>,
}

/// Reads the folder's login file once for its identity and the resets request's access.
pub fn read_login_with_access(folder: &Path) -> io::Result<LoginWithAccess> {
    let Some(bytes) = read_login_bytes(folder)? else {
        return Ok(LoginWithAccess {
            identity: None,
            access: None,
        });
    };
    let identity = identity_from(&bytes)?;
    let access = serde_json::from_slice::<AccessFile>(&bytes)
        .ok()
        .and_then(|f| f.tokens)
        .and_then(|t| {
            let token = t.access_token.filter(|s| !s.is_empty())?;
            let account = t.account_id.filter(|a| !a.trim().is_empty())?;
            Some((token, account))
        });
    Ok(LoginWithAccess { identity, access })
}

/// The login file's bytes, wiped when dropped; `Ok(None)` when there is none.
fn read_login_bytes(folder: &Path) -> io::Result<Option<Zeroizing<Vec<u8>>>> {
    match fs::read(folder.join(LOGIN_FILE)) {
        Ok(b) => Ok(Some(Zeroizing::new(b))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[derive(Deserialize)]
struct IdClaims {
    email: Option<String>,
    #[serde(rename = "https://api.openai.com/profile")]
    profile: Option<ProfileClaims>,
    #[serde(rename = "https://api.openai.com/auth")]
    auth: Option<AuthClaims>,
}

#[derive(Deserialize)]
struct ProfileClaims {
    email: Option<String>,
}

#[derive(Deserialize)]
struct AuthClaims {
    chatgpt_account_id: Option<String>,
    chatgpt_plan_type: Option<String>,
    #[serde(default)]
    organizations: Vec<OrgClaim>,
}

#[derive(Deserialize)]
struct OrgClaim {
    id: Option<String>,
    #[serde(default)]
    is_default: bool,
}

/// Who the Codex folder is signed in as, read from the ID token's claims in the login file. `Ok(None)`: signed out,
/// or signed in with an API key (no identity to read).
///
/// With [`read_login_with_access`] (for the banked resets' one request), the only functions that open Codex's
/// login file. The token's signature is not checked: the claims only
/// tell accounts apart, they grant nothing.
pub fn read_identity_from_login_file(folder: &Path) -> io::Result<Option<Identity>> {
    match read_login_bytes(folder)? {
        Some(bytes) => identity_from(&bytes),
        None => Ok(None),
    }
}

fn identity_from(bytes: &[u8]) -> io::Result<Option<Identity>> {
    let invalid = |what: &str| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Codex login file: {what}"),
        )
    };
    let login: LoginFile = serde_json::from_slice(bytes).map_err(|_| invalid("unknown format"))?;
    let Some(id_token) = login.tokens.and_then(|t| t.id_token) else {
        return Ok(None);
    };
    let payload = id_token
        .split('.')
        .nth(1)
        .ok_or_else(|| invalid("unknown ID token"))?;
    let claims =
        Zeroizing::new(base64url_decode(payload).ok_or_else(|| invalid("unknown ID token"))?);
    drop(id_token);
    let claims: IdClaims =
        serde_json::from_slice(&claims).map_err(|_| invalid("unknown ID token claims"))?;
    let Some(email) = claims
        .email
        .or(claims.profile.and_then(|p| p.email))
        .filter(|e| !e.trim().is_empty())
    else {
        return Ok(None);
    };
    let auth = claims.auth;
    let workspace = auth.as_ref().and_then(|a| a.chatgpt_account_id.clone());
    // Codex parses no workspace-name claim. The default organization title (often "Personal")
    // is not the selected ChatGPT workspace and must not be shown as its name.
    let plan = auth
        .as_ref()
        .and_then(|a| a.chatgpt_plan_type.as_deref())
        .and_then(plan_label);
    let org = auth.and_then(|a| {
        let mut orgs = a.organizations;
        let i = orgs.iter().position(|o| o.is_default).unwrap_or(0);
        (i < orgs.len()).then(|| orgs.swap_remove(i))
    });
    Ok(Some(Identity {
        email,
        organization: plan,
        organization_id: workspace.or(org.and_then(|o| o.id)),
    }))
}

// Familiar plan families; future codes become words with the first word capitalised.
fn plan_label(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let code = raw.to_lowercase();
    let family = code.strip_prefix("self_serve_").unwrap_or(&code);
    let first = family.split('_').next().unwrap_or(family);
    let label = match first {
        "business" => Some("Business"),
        "team" => Some("Team"),
        "plus" => Some("Plus"),
        "pro" => Some("Pro"),
        "enterprise" => Some("Enterprise"),
        "free" => Some("Free"),
        "edu" | "education" => Some("Education"),
        _ => None,
    };
    if let Some(label) = label {
        return Some(label.into());
    }
    let words = code.replace(['_', '-'], " ");
    let mut chars = words.chars();
    Some(chars.next()?.to_uppercase().chain(chars).collect())
}

/// Decodes unpadded base64url (a JWT segment).
fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        };
        buf = (buf << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buf >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

#[derive(Deserialize)]
struct LogLine {
    timestamp: Option<String>,
    payload: Option<LogPayload>,
}

#[derive(Deserialize)]
struct LogPayload {
    #[serde(rename = "type")]
    kind: Option<String>,
    rate_limits: Option<RateLimits>,
}

#[derive(Deserialize)]
struct RateLimits {
    primary: Option<Window>,
    secondary: Option<Window>,
}

#[derive(Deserialize)]
struct Window {
    used_percent: Option<f64>,
    window_minutes: Option<i64>,
    /// Seconds since the epoch (newer Codex versions).
    resets_at: Option<i64>,
    /// Seconds after the event (older Codex versions).
    resets_in_seconds: Option<i64>,
}

const ONE_DAY_MIN: i64 = 24 * 60;

/// The usage in one session log line, if it is a `token_count` event with rate limits.
pub fn parse_log_line(line: &[u8]) -> Option<Usage> {
    // Most lines are messages and tool output; only token_count events carry limits.
    if !contains(line, b"\"token_count\"") || !contains(line, b"rate_limits") {
        return None;
    }
    let parsed: LogLine = serde_json::from_slice(line).ok()?;
    let at = parsed.timestamp.as_deref().and_then(parse_rfc3339_ms)?;
    let payload = parsed.payload?;
    if payload.kind.as_deref() != Some("token_count") {
        return None;
    }
    let limits = payload.rate_limits?;
    let mut five_hour = None;
    let mut weekly = None;
    for w in [limits.primary, limits.secondary].into_iter().flatten() {
        let Some(used) = w.used_percent else { continue };
        let limit = Limit {
            used_percent: used.clamp(0.0, 100.0),
            resets_at: w
                .resets_at
                .map(|s| s * 1000)
                .or(w.resets_in_seconds.map(|s| at + s * 1000)),
        };
        match w.window_minutes {
            Some(m) if m <= ONE_DAY_MIN => five_hour = five_hour.or(Some(limit)),
            Some(_) => weekly = weekly.or(Some(limit)),
            // Older logs leave the window out: primary was the 5-hour window, secondary the week.
            None if five_hour.is_none() && weekly.is_none() => five_hour = Some(limit),
            None => weekly = weekly.or(Some(limit)),
        }
    }
    if five_hour.is_none() && weekly.is_none() {
        return None;
    }
    Some(Usage {
        read_at: at,
        five_hour,
        weekly,
        per_model_weekly: Vec::new(),
        extra: None,
        resets: None,
    })
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// How much of a log file is read when Warden first sees it: the end, where the latest limits are.
const TAIL_BYTES: u64 = 256 * 1024;
/// A line longer than this (a large tool output) is skipped rather than buffered.
const MAX_LINE: usize = 1024 * 1024;
/// How many of the newest log files are looked at on start.
const START_FILES: usize = 8;
/// How much of an append is read at a time.
const READ_CHUNK: usize = 64 * 1024;

#[derive(Default)]
struct FileState {
    offset: u64,
    /// The start of a line whose end has not been read yet.
    partial: Vec<u8>,
    /// Inside a line that is skipped: the first, cut piece when reading starts mid-file, or one over `MAX_LINE`.
    skipping: bool,
}

impl FileState {
    /// Takes the next bytes read from the file and returns the usage in the lines they complete. Never holds more
    /// than `MAX_LINE` bytes of an unfinished line.
    fn feed(&mut self, bytes: &[u8]) -> Vec<Usage> {
        let mut found = Vec::new();
        let mut rest = bytes;
        while let Some(i) = rest.iter().position(|&b| b == b'\n') {
            let piece = &rest[..i];
            rest = &rest[i + 1..];
            if self.skipping {
                self.skipping = false;
                self.partial.clear();
                continue;
            }
            let usage = if self.partial.is_empty() {
                parse_log_line(piece)
            } else if self.partial.len() + piece.len() > MAX_LINE {
                self.partial.clear();
                None
            } else {
                self.partial.extend_from_slice(piece);
                let line = std::mem::take(&mut self.partial);
                parse_log_line(&line)
            };
            found.extend(usage);
        }
        if !self.skipping {
            if self.partial.len() + rest.len() > MAX_LINE {
                self.partial.clear();
                self.skipping = true;
            } else {
                self.partial.extend_from_slice(rest);
            }
        }
        found
    }
}

/// Reads Codex session logs incrementally: each file only from where it was last read.
#[derive(Default)]
pub struct LogTail {
    files: HashMap<PathBuf, FileState>,
}

impl LogTail {
    pub fn new() -> Self {
        LogTail::default()
    }

    /// Whether a path is a session log this reads.
    pub fn is_session_log(path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "jsonl")
            && path.components().any(|c| c.as_os_str() == SESSIONS_DIR)
    }

    /// The newest usage in the latest session logs, read from their ends. Called once when watching starts; after
    /// that only [`LogTail::read_appended`] reads, from where this stopped.
    pub fn start(&mut self, codex_home: &Path) -> Option<Usage> {
        let mut newest = Vec::new();
        collect_newest(&codex_home.join(SESSIONS_DIR), 3, &mut newest);
        newest.sort_by_key(|a| std::cmp::Reverse(a.1));
        newest.truncate(START_FILES);
        let mut best: Option<Usage> = None;
        for (path, _) in newest {
            if let Ok(Some(u)) = self.read_appended(&path) {
                best = newer(best, u);
            }
        }
        best
    }

    /// Reads what was appended to `path` since the last read and returns the newest usage in it. A file seen for
    /// the first time is read from near its end.
    pub fn read_appended(&mut self, path: &Path) -> io::Result<Option<Usage>> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        let state = self.files.entry(path.to_owned()).or_insert_with(|| {
            let offset = len.saturating_sub(TAIL_BYTES);
            // Starting mid-file, the first piece is the end of a line: skip it.
            FileState {
                offset,
                partial: Vec::new(),
                skipping: offset > 0,
            }
        });
        if len < state.offset {
            // The file was replaced or cut short: start again.
            *state = FileState::default();
        }
        if len == state.offset {
            return Ok(None);
        }
        file.seek(SeekFrom::Start(state.offset))?;
        // Read in fixed pieces, so a large append never needs more memory than one piece and one line.
        let mut reader = file.take(len - state.offset);
        let mut chunk = vec![0u8; READ_CHUNK];
        let mut best = None;
        loop {
            let n = reader.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            state.offset += n as u64;
            for u in state.feed(&chunk[..n]) {
                best = newer(best, u);
            }
        }
        Ok(best)
    }

    /// Forgets files no longer worth tracking, keeping memory bounded.
    pub fn forget_older_than(&mut self, keep: usize) {
        if self.files.len() <= keep {
            return;
        }
        let mut by_age: Vec<(PathBuf, std::time::SystemTime)> = self
            .files
            .keys()
            .map(|p| {
                (
                    p.clone(),
                    fs::metadata(p)
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::UNIX_EPOCH),
                )
            })
            .collect();
        by_age.sort_by_key(|a| std::cmp::Reverse(a.1));
        for (p, _) in by_age.into_iter().skip(keep) {
            self.files.remove(&p);
        }
    }
}

/// The newer of two readings.
pub fn newer(a: Option<Usage>, b: Usage) -> Option<Usage> {
    match a {
        Some(a) if a.read_at >= b.read_at => Some(a),
        _ => Some(b),
    }
}

/// Collects `.jsonl` files from the newest dated folders (`YYYY/MM/DD`), walking newest names first.
fn collect_newest(dir: &Path, depth: u32, out: &mut Vec<(PathBuf, std::time::SystemTime)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
    for e in entries {
        if out.len() >= START_FILES {
            return;
        }
        let path = e.path();
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() && depth > 0 {
            collect_newest(&path, depth - 1, out);
        } else if kind.is_file() && path.extension().is_some_and(|x| x == "jsonl") {
            let modified = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            out.push((path, modified));
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testdir::Dir;
    use std::io::Write;

    fn b64(bytes: &[u8]) -> String {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |acc, (i, &b)| acc | (u32::from(b) << (16 - 8 * i)));
            for i in 0..=chunk.len() {
                out.push(char::from(A[((n >> (18 - 6 * i)) & 63) as usize]));
            }
        }
        out
    }

    /// A Codex login file as `codex login` writes it, with the given ID token claims.
    pub(crate) fn login_file(claims: &str, access_token: &str) -> String {
        let jwt = format!(
            "{}.{}.sig",
            b64(br#"{"alg":"RS256"}"#),
            b64(claims.as_bytes())
        );
        format!(
            r#"{{"OPENAI_API_KEY": null, "tokens": {{"id_token": "{jwt}", "access_token": "{access_token}", "refresh_token": "{access_token}-r", "account_id": "ws-1"}}, "last_refresh": "2026-11-01T00:00:00Z"}}"#
        )
    }

    #[test]
    fn reads_the_identity_and_the_resets_access_from_one_reading() {
        let dir = crate::testdir::Dir::new("codex-access");
        let none = read_login_with_access(dir.path()).expect("no file");
        assert!(none.identity.is_none() && none.access.is_none());
        std::fs::write(
            dir.path().join(LOGIN_FILE),
            login_file(r#"{"email": "ann@example.com"}"#, "fixture-access"),
        )
        .expect("write");
        let login = read_login_with_access(dir.path()).expect("read");
        assert_eq!(
            login.identity.map(|i| i.email).as_deref(),
            Some("ann@example.com")
        );
        let (token, account) = login.access.expect("signed in");
        assert_eq!(token.expose(), "fixture-access");
        assert_eq!(account, "ws-1");
        // The token never shows in a debug print.
        assert!(!format!("{token:?}").contains("fixture-access"));
        // An API-key login has no tokens: nothing to ask with.
        std::fs::write(
            dir.path().join(LOGIN_FILE),
            r#"{"OPENAI_API_KEY": "sk-fixture", "tokens": null}"#,
        )
        .expect("write");
        let api_key = read_login_with_access(dir.path()).expect("read");
        assert!(api_key.identity.is_none() && api_key.access.is_none());
        // A login file of another shape is an error, with no access either.
        std::fs::write(dir.path().join(LOGIN_FILE), "[]").expect("write");
        assert!(read_login_with_access(dir.path()).is_err());
    }

    #[test]
    fn decodes_base64url() {
        assert_eq!(
            base64url_decode(&b64(b"hello world!?")).as_deref(),
            Some(&b"hello world!?"[..])
        );
        assert_eq!(base64url_decode("aGk"), Some(b"hi".to_vec()));
        assert_eq!(base64url_decode("a*b"), None);
    }

    #[test]
    fn identity_from_the_id_token() {
        let dir = Dir::new("codex");
        let claims = r#"{"email": "ann@example.com", "https://api.openai.com/auth": {"chatgpt_account_id": "ws-1", "chatgpt_plan_type": "team", "organizations": [{"id": "org-a", "title": "Personal", "is_default": false}, {"id": "org-b", "title": "Acme", "is_default": true}]}}"#;
        fs::write(dir.path().join(LOGIN_FILE), login_file(claims, "at-secret")).expect("write");
        let id = read_identity_from_login_file(dir.path())
            .expect("reads")
            .expect("signed in");
        assert_eq!(id.email, "ann@example.com");
        assert_eq!(id.organization.as_deref(), Some("Team"));
        assert_eq!(id.organization_id.as_deref(), Some("ws-1"));
    }

    #[test]
    fn plans_replace_the_default_organization_label() {
        let dir = Dir::new("codex");
        for (plan, expected) in [
            ("plus", Some("Plus")),
            ("pro", Some("Pro")),
            ("business", Some("Business")),
            ("self_serve_business_prolite", Some("Business")),
            ("self_serve_business", Some("Business")),
            ("business_prolite", Some("Business")),
            ("team_annual", Some("Team")),
            ("plus_annual", Some("Plus")),
            ("pro_annual", Some("Pro")),
            ("enterprise_annual", Some("Enterprise")),
            ("free_trial", Some("Free")),
            (" TEAM ", Some("Team")),
            ("edu", Some("Education")),
            ("education", Some("Education")),
            ("education_annual", Some("Education")),
            ("self_serve_edu", Some("Education")),
            ("self_serve_pro_annual", Some("Pro")),
            ("promo_trial", Some("Promo trial")),
            ("professional", Some("Professional")),
            ("businesslike", Some("Businesslike")),
            ("self_serve_promo_trial", Some("Self serve promo trial")),
            ("future", Some("Future")),
            ("future_plan-code", Some("Future plan code")),
            ("", None),
            (" ", None),
        ] {
            let claims =
                serde_json::json!({"email":"ann@example.com", "https://api.openai.com/auth": {
                    "chatgpt_account_id":"ws-1", "chatgpt_plan_type":plan,
                    "organizations":[{"id":"org-1", "title":"Personal", "is_default":true}]
                }})
                .to_string();
            fs::write(
                dir.path().join(LOGIN_FILE),
                login_file(&claims, "fixture-only"),
            )
            .expect("write");
            let id = read_identity_from_login_file(dir.path())
                .expect("reads")
                .expect("identity");
            assert_eq!(id.organization.as_deref(), expected);
            assert_eq!(id.organization_id.as_deref(), Some("ws-1"));
            let mut other = id.clone();
            other.organization = Some("Another plan".into());
            assert!(id.same_as(&other), "plan is not identity");
            other.organization_id = Some("ws-2".into());
            assert!(
                !id.same_as(&other),
                "same email in different workspace is distinct"
            );
        }
    }

    #[test]
    fn email_from_the_profile_claim() {
        let dir = Dir::new("codex");
        fs::write(
            dir.path().join(LOGIN_FILE),
            login_file(
                r#"{"https://api.openai.com/profile": {"email": "bob@example.com"}}"#,
                "x",
            ),
        )
        .expect("write");
        let id = read_identity_from_login_file(dir.path())
            .expect("reads")
            .expect("signed in");
        assert_eq!(id.email, "bob@example.com");
        assert_eq!(id.organization, None);
    }

    #[test]
    fn api_key_login_or_none_has_no_identity() {
        let dir = Dir::new("codex");
        assert_eq!(
            read_identity_from_login_file(dir.path()).expect("reads"),
            None
        );
        fs::write(
            dir.path().join(LOGIN_FILE),
            r#"{"OPENAI_API_KEY": "sk-x", "tokens": null}"#,
        )
        .expect("write");
        assert_eq!(
            read_identity_from_login_file(dir.path()).expect("reads"),
            None
        );
        fs::write(dir.path().join(LOGIN_FILE), "not json").expect("write");
        assert!(read_identity_from_login_file(dir.path()).is_err());
    }

    pub(crate) fn token_count(ts: &str, primary: &str, secondary: &str) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":10}}}},"rate_limits":{{"primary":{primary},"secondary":{secondary}}}}}}}"#
        )
    }

    #[test]
    fn parses_token_count_events() {
        let line = token_count(
            "2026-11-07T10:00:00Z",
            r#"{"used_percent":12.5,"window_minutes":300,"resets_at":1794060000}"#,
            r#"{"used_percent":31.0,"window_minutes":10080,"resets_in_seconds":3600}"#,
        );
        let u = parse_log_line(line.as_bytes()).expect("usage");
        let at = parse_rfc3339_ms("2026-11-07T10:00:00Z").expect("time");
        assert_eq!(u.read_at, at);
        assert_eq!(
            u.five_hour,
            Some(Limit {
                used_percent: 12.5,
                resets_at: Some(1_794_060_000_000)
            })
        );
        assert_eq!(
            u.weekly,
            Some(Limit {
                used_percent: 31.0,
                resets_at: Some(at + 3_600_000)
            })
        );
    }

    #[test]
    fn a_weekly_only_plan_has_no_five_hour_limit() {
        let line = token_count(
            "2026-11-07T10:00:00Z",
            r#"{"used_percent":4,"window_minutes":10080,"resets_at":1}"#,
            "null",
        );
        let u = parse_log_line(line.as_bytes()).expect("usage");
        assert_eq!(u.five_hour, None);
        assert_eq!(u.weekly.map(|l| l.used_percent), Some(4.0));
    }

    #[test]
    fn other_lines_are_skipped() {
        assert!(parse_log_line(br#"{"timestamp":"2026-11-07T10:00:00Z","type":"response_item","payload":{"type":"message"}}"#).is_none());
        assert!(parse_log_line(br#"{"timestamp":"2026-11-07T10:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":null}}"#).is_none());
        assert!(parse_log_line(b"garbage token_count rate_limits").is_none());
    }

    fn append(path: &Path, text: &str) {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open");
        f.write_all(text.as_bytes()).expect("append");
    }

    #[test]
    fn reads_only_what_was_appended() {
        let home = Dir::new("codex-home");
        let day = home.path().join("sessions/2026/11/07");
        fs::create_dir_all(&day).expect("mkdir");
        let log = day.join("rollout-2026-11-07T09-00-00-abc.jsonl");
        let w = |p: f64| format!(r#"{{"used_percent":{p},"window_minutes":300,"resets_at":1}}"#);
        append(
            &log,
            &format!(
                "{}\n",
                token_count("2026-11-07T09:00:00Z", &w(10.0), "null")
            ),
        );

        let mut tail = LogTail::new();
        let first = tail.start(home.path()).expect("a reading on start");
        assert_eq!(first.five_hour.map(|l| l.used_percent), Some(10.0));
        assert_eq!(
            tail.read_appended(&log).expect("reads"),
            None,
            "nothing new"
        );

        // Half a line, then the rest: read once complete.
        let line = token_count("2026-11-07T09:05:00Z", &w(20.0), "null");
        let (a, b) = line.split_at(40);
        append(&log, a);
        assert_eq!(tail.read_appended(&log).expect("reads"), None);
        append(&log, &format!("{b}\n"));
        let next = tail.read_appended(&log).expect("reads").expect("usage");
        assert_eq!(next.five_hour.map(|l| l.used_percent), Some(20.0));
    }

    #[test]
    fn a_large_append_is_read_in_pieces_and_an_overlong_line_skipped() {
        let w = |p: f64| format!(r#"{{"used_percent":{p},"window_minutes":300,"resets_at":1}}"#);
        let mut state = FileState::default();
        // A line far over the limit, fed in pieces, holds no more than the limit and yields nothing.
        let huge = format!(
            "{{\"type\":\"event_msg\",\"payload\":\"token_count rate_limits {}\"}}",
            "x".repeat(MAX_LINE * 2)
        );
        for piece in huge.as_bytes().chunks(READ_CHUNK) {
            assert!(state.feed(piece).is_empty());
            assert!(state.partial.len() <= MAX_LINE);
        }
        // The line after it is read again, across a piece boundary.
        let next = format!(
            "\n{}\n",
            token_count("2026-11-07T09:00:00Z", &w(30.0), "null")
        );
        let (a, b) = next.split_at(50);
        assert!(state.feed(a.as_bytes()).is_empty());
        let found = state.feed(b.as_bytes());
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].five_hour.as_ref().map(|l| l.used_percent),
            Some(30.0)
        );
        assert!(state.partial.is_empty() && !state.skipping);
    }

    #[test]
    fn a_new_file_is_read_from_its_start_and_a_large_one_from_its_end() {
        let home = Dir::new("codex-home");
        let day = home.path().join("sessions/2026/11/08");
        fs::create_dir_all(&day).expect("mkdir");
        let big = day.join("rollout-big.jsonl");
        let filler = format!(
            "{{\"type\":\"response_item\",\"payload\":\"{}\"}}\n",
            "x".repeat(1000)
        );
        append(
            &big,
            &format!(
                "{}\n",
                token_count(
                    "2026-11-08T08:00:00Z",
                    r#"{"used_percent":1,"window_minutes":300}"#,
                    "null"
                )
            ),
        );
        for _ in 0..400 {
            append(&big, &filler);
        }
        append(
            &big,
            &format!(
                "{}\n",
                token_count(
                    "2026-11-08T09:00:00Z",
                    r#"{"used_percent":2,"window_minutes":300}"#,
                    "null"
                )
            ),
        );
        let mut tail = LogTail::new();
        let u = tail.read_appended(&big).expect("reads").expect("usage");
        assert_eq!(u.five_hour.map(|l| l.used_percent), Some(2.0));
        assert!(LogTail::is_session_log(&big));
        assert!(!LogTail::is_session_log(
            &home.path().join("log/codex-tui.log")
        ));
    }

    #[test]
    fn no_sessions_gives_no_reading() {
        let home = Dir::new("codex-home");
        assert_eq!(LogTail::new().start(home.path()), None);
    }
}
