//! Warden's own CLI sign-in. Renewal requires ownership of the data folder's server lock.
//! No account folder is accepted by this API: the folder is always derived from that lock.

use crate::claude::{self, UsageEndpoint};
use crate::model::{Identity, Usage, UsageError};
use crate::secret::Secret;
use crate::store::{self, ServerLock};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::Zeroizing;

pub const FOLDER: &str = "main-sign-in";
pub const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CREDENTIALS: &str = ".credentials.json";
const RENEW_AHEAD_MS: i64 = 5 * 60_000;

pub fn folder(data_dir: &Path) -> PathBuf {
    crate::detect::without_verbatim_prefix(data_dir.join(FOLDER))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SignInState {
    SignedIn,
    #[default]
    SignedOut,
    Expired,
    OtherAccount,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SignIn {
    pub state: SignInState,
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CredentialStamp {
    modified_ns: u64,
    bytes: u64,
}

/// No login data: only the public sign-in state, a refusal latch, and a pending official sign-in's file generation.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MainLogin {
    pub sign_in: SignIn,
    expired: bool,
    waiting_for: Option<CredentialStamp>,
    /// Set by the read that renewed the sign-in: until when the new one holds (ms). The caller logs it and takes it;
    /// never saved.
    #[serde(skip)]
    pub renewed_until: Option<i64>,
    /// Set by the read whose renewal was refused, turning the sign-in expired; the caller logs it once and takes it.
    #[serde(skip)]
    pub newly_refused: bool,
}

impl MainLogin {
    /// What the last read did to the sign-in, as lines for the log (no login data), taken so each is said once.
    pub fn take_log_lines(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(until) = self.renewed_until.take() {
            lines.push(format!(
                "renewed Warden's sign-in to main; it holds until {}",
                crate::routing::reset_text(Some(until))
            ));
        }
        if std::mem::take(&mut self.newly_refused) {
            lines.push("Warden's sign-in to main was refused on renewal; sign in again".to_owned());
        }
        lines
    }
}

fn checked_folder(lock: &ServerLock) -> Result<PathBuf, UsageError> {
    let own = folder(lock.data_dir());
    for path in [&own, &own.join(CREDENTIALS), &own.join(".claude.json")] {
        match fs::symlink_metadata(path) {
            Ok(metadata) if is_link(&metadata) => {
                return Err(failed("Warden's sign-in must not be a link"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(failed("Warden's sign-in could not be read")),
        }
    }
    Ok(own)
}

fn credential_stamp(own: &Path) -> Option<CredentialStamp> {
    let metadata = fs::metadata(own.join(CREDENTIALS)).ok()?;
    let modified_ns = u64::try_from(
        metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos(),
    )
    .ok()?;
    Some(CredentialStamp {
        modified_ns,
        bytes: metadata.len(),
    })
}

/// Prepare only Warden's own folder. Do not retry an old refused token while the official sign-in window is open.
pub fn begin_sign_in(lock: &ServerLock) -> Result<(PathBuf, MainLogin), UsageError> {
    let own = checked_folder(lock)?;
    fs::create_dir_all(&own).map_err(|_| failed("Warden's sign-in folder could not be created"))?;
    let waiting_for = credential_stamp(&own);
    Ok((
        own,
        MainLogin {
            waiting_for,
            ..MainLogin::default()
        },
    ))
}

/// Identity is checked before opening the login. Only the lock holder may renew this one sign-in.
pub fn read_for_main(
    lock: &ServerLock,
    expected: Option<&Identity>,
    main_changed: bool,
    login: &mut MainLogin,
    endpoint: &dyn UsageEndpoint,
    server: &dyn SignInServer,
    now: i64,
) -> Result<Usage, UsageError> {
    login.sign_in = SignIn::default();
    let Some(expected) = expected else {
        return Err(UsageError::SignedOut);
    };
    let own = checked_folder(lock)?;
    if login.waiting_for.is_some() {
        let current = credential_stamp(&own);
        if current.is_none() || current == login.waiting_for {
            return Err(UsageError::SignedOut);
        }
        login.waiting_for = None;
    }
    if !claude::has_login(&own) {
        return Err(UsageError::SignedOut);
    }
    let identity = claude::read_identity(&own, None)
        .map_err(|_| failed("Warden's sign-in identity could not be read"))?;
    let Some(identity) = identity else {
        return Err(UsageError::SignedOut);
    };
    login.sign_in.email = Some(identity.email.clone());
    if main_changed || !expected.same_as(&identity) {
        login.sign_in.state = SignInState::OtherAccount;
        return Err(UsageError::IdentityChanged);
    }
    login.sign_in.state = if login.expired {
        SignInState::Expired
    } else {
        SignInState::SignedIn
    };
    let was_expired = login.expired;
    let mut renewed = None;
    let result = read_usage_noting(
        lock,
        &mut login.expired,
        &mut renewed,
        endpoint,
        server,
        now,
    );
    login.renewed_until = renewed;
    login.newly_refused = !was_expired && login.expired;
    if login.expired || matches!(result, Err(UsageError::LoginExpired { .. })) {
        login.sign_in.state = SignInState::Expired;
    } else if result == Err(UsageError::SignedOut) {
        login.sign_in = SignIn::default();
    }
    result
}

#[derive(Debug)]
pub struct Renewed {
    pub access_token: Secret,
    pub refresh_token: Option<Secret>,
    pub expires_in: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RenewalError {
    Refused(u16),
    Failed(String),
}

/// Tests provide a fake; only the real implementation calls the sign-in server.
pub trait SignInServer: Send + Sync {
    fn renew(&self, refresh_token: &Secret) -> Result<Renewed, RenewalError>;
}

pub struct AnthropicSignInServer {
    agent: ureq::Agent,
}

impl Default for AnthropicSignInServer {
    fn default() -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .tls_config(claude::tls_config())
                .timeout_global(Some(Duration::from_secs(10)))
                .max_redirects(0)
                .http_status_as_error(false)
                .build()
                .into(),
        }
    }
}

#[derive(Serialize)]
struct Request<'a> {
    grant_type: &'static str,
    refresh_token: &'a str,
    client_id: &'static str,
}

fn request_body(token: &Secret) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(
        serde_json::to_vec(&Request {
            grant_type: "refresh_token",
            refresh_token: token.expose(),
            client_id: CLIENT_ID,
        })
        .expect("serialising string fields"),
    )
}

impl SignInServer for AnthropicSignInServer {
    fn renew(&self, token: &Secret) -> Result<Renewed, RenewalError> {
        if !crate::policy::ALLOWED_OUTBOUND_HOSTS.contains(&"platform.claude.com") {
            return Err(RenewalError::Failed(
                "the sign-in server is not allowed".into(),
            ));
        }
        let body = request_body(token);
        let mut response = self
            .agent
            .post(TOKEN_URL)
            .header("Content-Type", "application/json")
            .send(body.as_slice())
            .map_err(|e| RenewalError::Failed(claude::describe_transport_error(&e)))?;
        read_response(response.status().as_u16(), response.body_mut().as_reader())
    }
}

/// Error bodies are never read; neither parser nor transport errors expose response text.
fn read_response(status: u16, reader: impl Read) -> Result<Renewed, RenewalError> {
    match status {
        200 => {}
        400 | 401 => return Err(RenewalError::Refused(status)),
        _ => {
            return Err(RenewalError::Failed(format!(
                "HTTP {status}: sign-in renewal failed"
            )));
        }
    }
    let bad = || RenewalError::Failed("the sign-in server answered in an unknown shape".into());
    let mut bytes = Zeroizing::new(Vec::new());
    reader
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| bad())?;
    if bytes.len() > 256 * 1024 {
        return Err(bad());
    }
    #[derive(Deserialize)]
    struct Answer {
        access_token: Secret,
        refresh_token: Option<Secret>,
        expires_in: u64,
    }
    let answer: Answer = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    if answer.access_token.is_empty()
        || answer.refresh_token.as_ref().is_some_and(Secret::is_empty)
        || answer.expires_in == 0
    {
        return Err(bad());
    }
    Ok(Renewed {
        access_token: answer.access_token,
        refresh_token: answer.refresh_token,
        expires_in: answer.expires_in,
    })
}

fn failed(message: &str) -> UsageError {
    UsageError::Failed {
        message: message.into(),
    }
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// A refusal is remembered until the caller starts an explicit new sign-in. Callers persist `expired` with
/// their readings, so a restart or a background refresh cannot spend a refused token again.
pub fn read_usage(
    lock: &ServerLock,
    expired: &mut bool,
    endpoint: &dyn UsageEndpoint,
    server: &dyn SignInServer,
    now: i64,
) -> Result<Usage, UsageError> {
    read_usage_noting(lock, expired, &mut None, endpoint, server, now)
}

/// [`read_usage`], noting in `renewed` until when a renewed sign-in holds, for the log line.
fn read_usage_noting(
    lock: &ServerLock,
    expired: &mut bool,
    renewed_until: &mut Option<i64>,
    endpoint: &dyn UsageEndpoint,
    server: &dyn SignInServer,
    now: i64,
) -> Result<Usage, UsageError> {
    if *expired {
        return Err(UsageError::LoginExpired { status: None });
    }
    let own = checked_folder(lock)?;
    let path = own.join(CREDENTIALS);
    let read = || -> Result<Option<Zeroizing<Vec<u8>>>, UsageError> {
        match fs::read(&path) {
            Ok(b) => Ok(Some(Zeroizing::new(b))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(failed("Warden's sign-in could not be read")),
        }
    };
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Login {
        access_token: Option<Secret>,
        refresh_token: Option<Secret>,
        expires_at: Option<i64>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Credentials {
        claude_ai_oauth: Option<Login>,
    }
    let parse = |b: &[u8]| -> Result<Login, UsageError> {
        let c: Credentials = serde_json::from_slice(b)
            .map_err(|_| failed("Warden's sign-in is not in a known format"))?;
        c.claude_ai_oauth.ok_or(UsageError::SignedOut)
    };
    let Some(mut bytes) = read()? else {
        return Err(UsageError::SignedOut);
    };
    let mut login = parse(&bytes)?;
    if login
        .expires_at
        .is_some_and(|at| at <= now.saturating_add(RENEW_AHEAD_MS))
    {
        // The official sign-in window may have renewed it since the first read.
        bytes = read()?.ok_or(UsageError::SignedOut)?;
        login = parse(&bytes)?;
        if login
            .expires_at
            .is_some_and(|at| at <= now.saturating_add(RENEW_AHEAD_MS))
        {
            let refresh = login
                .refresh_token
                .as_ref()
                .filter(|t| !t.is_empty())
                .ok_or(UsageError::LoginExpired { status: None })?;
            let renewed = match server.renew(refresh) {
                Ok(answer) => answer,
                Err(RenewalError::Refused(status)) => {
                    // The official CLI can rotate this login while the request is in flight. A refusal of
                    // that old token must not latch the replacement login as expired.
                    let latest = read()?.ok_or(UsageError::SignedOut)?;
                    let current = parse(&latest)?;
                    if current.refresh_token.as_ref().map(Secret::expose) != Some(refresh.expose())
                    {
                        if current
                            .expires_at
                            .is_some_and(|at| at > now.saturating_add(RENEW_AHEAD_MS))
                        {
                            let access = current
                                .access_token
                                .as_ref()
                                .filter(|t| !t.is_empty())
                                .ok_or(UsageError::SignedOut)?;
                            return endpoint.fetch(access);
                        }
                        return Err(failed(
                            "Warden's sign-in changed during renewal; it will be read again",
                        ));
                    }
                    *expired = true;
                    return Err(UsageError::LoginExpired {
                        status: Some(status),
                    });
                }
                Err(RenewalError::Failed(_)) => {
                    return Err(failed("Warden's sign-in could not be renewed"));
                }
            };
            let expires = i64::try_from(renewed.expires_in)
                .ok()
                .and_then(|s| s.checked_mul(1000))
                .and_then(|ms| now.checked_add(ms))
                .ok_or_else(|| failed("the renewed sign-in has an invalid expiry"))?;
            // Keep all CLI fields, replacing only the three OAuth values. The serialized buffer is wiped.
            let mut value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|_| failed("Warden's sign-in is not in a known format"))?;
            let oauth = &mut value["claudeAiOauth"];
            use zeroize::Zeroize;
            if let Some(serde_json::Value::String(old)) = oauth.get_mut("accessToken") {
                old.zeroize();
            }
            oauth["accessToken"] = renewed.access_token.expose().into();
            oauth["expiresAt"] = expires.into();
            if let Some(refresh) = &renewed.refresh_token {
                if let Some(serde_json::Value::String(old)) = oauth.get_mut("refreshToken") {
                    old.zeroize();
                }
                oauth["refreshToken"] = refresh.expose().into();
            }
            let saved = Zeroizing::new(
                serde_json::to_vec_pretty(&value)
                    .map_err(|_| failed("the renewed sign-in could not be saved"))?,
            );
            // Wipe token string copies held by the JSON used to preserve unknown CLI fields.
            fn wipe(value: &mut serde_json::Value) {
                use zeroize::Zeroize;
                match value {
                    serde_json::Value::String(s) => s.zeroize(),
                    serde_json::Value::Array(a) => a.iter_mut().for_each(wipe),
                    serde_json::Value::Object(o) => o.values_mut().for_each(wipe),
                    _ => {}
                }
            }
            wipe(&mut value);
            store::write_atomic(&path, &saved)
                .map_err(|_| failed("the renewed sign-in could not be saved"))?;
            // Saving must succeed before the new access token can reach the usage endpoint.
            login.access_token = Some(renewed.access_token);
            *renewed_until = Some(expires);
        }
    }
    let token = login
        .access_token
        .filter(|t| !t.is_empty())
        .ok_or(UsageError::SignedOut)?;
    endpoint.fetch(&token).map(|mut usage| {
        usage.read_at = now;
        usage
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::Dir;
    use std::sync::Mutex;
    use warden_rules::secrets::{assert_no_token, canary_refresh_token, canary_token};

    #[test]
    fn a_refusal_of_a_token_rotated_by_the_cli_does_not_expire_the_new_login() {
        struct Rotating {
            path: PathBuf,
        }
        impl SignInServer for Rotating {
            fn renew(&self, _: &Secret) -> Result<Renewed, RenewalError> {
                fs::write(&self.path, br#"{"claudeAiOauth":{"accessToken":"fixture-new","refreshToken":"fixture-new-refresh","expiresAt":9999999}}"#).expect("CLI rotation");
                Err(RenewalError::Refused(400))
            }
        }
        let dir = Dir::new("rotation-refusal");
        let lock = store::hold_server_lock(dir.path()).expect("lock");
        let path = credentials(&lock, "fixture-old", "fixture-old-refresh", 0);
        let server = Rotating { path: path.clone() };
        let endpoint = FakeUsage {
            path,
            expected: "fixture-new".into(),
        };
        let mut expired = false;
        assert!(read_usage(&lock, &mut expired, &endpoint, &server, 100).is_ok());
        assert!(!expired);
        assert!(read_usage(&lock, &mut expired, &endpoint, &server, 200).is_ok());
    }

    #[test]
    fn a_signed_out_main_does_not_describe_wardens_login_as_another_account() {
        let dir = Dir::new("signed-out-main");
        let lock = store::hold_server_lock(dir.path()).expect("lock");
        let endpoint = FakeUsage {
            path: dir.path().join("unused"),
            expected: "unused".into(),
        };
        let server = FakeServer {
            seen: Mutex::new(Vec::new()),
            result: Mutex::new(None),
            locked_data: None,
        };
        let mut login = MainLogin::default();
        assert_eq!(
            read_for_main(&lock, None, false, &mut login, &endpoint, &server, 0),
            Err(UsageError::SignedOut)
        );
        assert_eq!(login.sign_in.state, SignInState::SignedOut);
        assert!(server.seen.lock().expect("seen").is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn the_official_sign_in_receives_an_ordinary_windows_path() {
        let dir = Dir::new("sign-in-path");
        let lock = store::hold_server_lock(dir.path()).expect("lock");
        let (path, _) = begin_sign_in(&lock).expect("sign-in");
        assert!(!path.to_string_lossy().starts_with(r"\\?\"));
        assert_eq!(
            fs::canonicalize(path).expect("folder"),
            lock.data_dir().join(FOLDER)
        );
    }

    struct FakeServer {
        seen: Mutex<Vec<String>>,
        result: Mutex<Option<Result<Renewed, RenewalError>>>,
        locked_data: Option<PathBuf>,
    }
    impl SignInServer for FakeServer {
        fn renew(&self, token: &Secret) -> Result<Renewed, RenewalError> {
            if let Some(data) = &self.locked_data {
                assert!(
                    store::server_running(data),
                    "renewal must hold the server lock"
                );
            }
            self.seen.lock().expect("lock").push(token.expose().into());
            self.result
                .lock()
                .expect("lock")
                .take()
                .expect("only one renewal")
        }
    }
    struct FakeUsage {
        path: PathBuf,
        expected: String,
    }
    impl UsageEndpoint for FakeUsage {
        fn fetch(&self, token: &Secret) -> Result<Usage, UsageError> {
            assert_eq!(token.expose(), self.expected);
            let saved: serde_json::Value =
                serde_json::from_slice(&fs::read(&self.path).expect("saved before use"))
                    .expect("JSON");
            assert_eq!(saved["claudeAiOauth"]["accessToken"], self.expected);
            claude::parse_usage(r#"{"seven_day":{"utilization":12}}"#, 0)
                .map_err(|_| failed("fixture"))
        }
    }
    fn credentials(lock: &ServerLock, access: &str, refresh: &str, expires: i64) -> PathBuf {
        let own = folder(lock.data_dir());
        fs::create_dir_all(&own).expect("folder");
        let path = own.join(CREDENTIALS);
        fs::write(&path, serde_json::to_vec(&serde_json::json!({
            "other": {"kept":true},
            "claudeAiOauth": {"accessToken":access,"refreshToken":refresh,"expiresAt":expires,"scopes":["user:profile"]}
        })).expect("JSON")).expect("write");
        path
    }
    fn server(result: Result<Renewed, RenewalError>) -> FakeServer {
        FakeServer {
            seen: Mutex::new(Vec::new()),
            result: Mutex::new(Some(result)),
            locked_data: None,
        }
    }
    #[test]
    fn renewal_rotates_and_saves_before_using_the_access_token() {
        for rotate in [false, true] {
            let dir = Dir::new("renew");
            let lock = store::hold_server_lock(dir.path()).expect("lock");
            let old = canary_token("old");
            let refresh = canary_refresh_token("old");
            let new = canary_token("new");
            let next = canary_refresh_token("new");
            let path = credentials(&lock, &old, &refresh, 10);
            let mut fake = server(Ok(Renewed {
                access_token: new.clone().into(),
                refresh_token: rotate.then(|| next.clone().into()),
                expires_in: 3600,
            }));
            fake.locked_data = Some(lock.data_dir().to_owned());
            let usage = FakeUsage {
                path: path.clone(),
                expected: new.clone(),
            };
            let mut expired = false;
            let result = read_usage(&lock, &mut expired, &usage, &fake, 100).expect("renewed");
            assert_eq!(result.read_at, 100);
            assert!(!expired);
            assert_eq!(
                fake.seen.lock().expect("lock").as_slice(),
                std::slice::from_ref(&refresh)
            );
            let saved: serde_json::Value =
                serde_json::from_slice(&fs::read(path).expect("read")).expect("JSON");
            assert_eq!(
                saved["claudeAiOauth"]["refreshToken"],
                if rotate {
                    next.as_str()
                } else {
                    refresh.as_str()
                }
            );
            assert_eq!(saved["claudeAiOauth"]["expiresAt"], 3_600_100);
            assert_eq!(saved["other"]["kept"], true);
            assert_eq!(saved["claudeAiOauth"]["scopes"][0], "user:profile");
            let public = serde_json::to_string(&result).expect("JSON");
            for token in [&old, &refresh, &new, &next] {
                assert_no_token("usage", &public, token);
            }
        }
    }
    #[test]
    fn renewal_is_due_five_minutes_before_expiry() {
        let dir = Dir::new("due");
        let lock = store::hold_server_lock(dir.path()).expect("lock");
        let path = credentials(&lock, "old", "refresh", 300_100);
        let fake = server(Ok(Renewed {
            access_token: "new".to_owned().into(),
            refresh_token: None,
            expires_in: 3600,
        }));
        read_usage(
            &lock,
            &mut false,
            &FakeUsage {
                path,
                expected: "new".into(),
            },
            &fake,
            100,
        )
        .expect("due");
        assert_eq!(fake.seen.lock().expect("lock").len(), 1);
    }
    #[test]
    fn a_current_login_does_not_renew() {
        let dir = Dir::new("current");
        let lock = store::hold_server_lock(dir.path()).expect("lock");
        let path = credentials(&lock, "current", "refresh", 300_101);
        let fake = server(Err(RenewalError::Refused(400)));
        read_usage(
            &lock,
            &mut false,
            &FakeUsage {
                path,
                expected: "current".into(),
            },
            &fake,
            100,
        )
        .expect("current");
        assert!(fake.seen.lock().expect("lock").is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn a_failed_atomic_save_never_sends_the_new_access_token() {
        let dir = Dir::new("save-failed");
        let lock = store::hold_server_lock(dir.path()).expect("lock");
        let path = credentials(&lock, "old", "refresh", 0);
        let before = fs::read(&path).expect("before");
        let original_permissions = fs::metadata(&path).expect("metadata").permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).expect("readonly");
        let fake = server(Ok(Renewed {
            access_token: "new".to_owned().into(),
            refresh_token: None,
            expires_in: 3600,
        }));
        let result = read_usage(
            &lock,
            &mut false,
            &FakeUsage {
                path: path.clone(),
                expected: "must not be sent".into(),
            },
            &fake,
            100,
        );
        fs::set_permissions(&path, original_permissions).expect("restore");
        assert_eq!(
            result,
            Err(failed("the renewed sign-in could not be saved"))
        );
        assert_eq!(fs::read(path).expect("after"), before);
    }
    #[test]
    fn refused_refresh_stops_further_attempts_and_keeps_the_file() {
        for status in [400, 401] {
            let dir = Dir::new("refused");
            let lock = store::hold_server_lock(dir.path()).expect("lock");
            let path = credentials(&lock, "old", "refresh", 0);
            let before = fs::read(&path).expect("read");
            let fake = server(Err(RenewalError::Refused(status)));
            let usage = FakeUsage {
                path: path.clone(),
                expected: "never sent".into(),
            };
            let mut expired = false;
            assert_eq!(
                read_usage(&lock, &mut expired, &usage, &fake, 100),
                Err(UsageError::LoginExpired {
                    status: Some(status)
                })
            );
            assert!(expired);
            assert!(read_usage(&lock, &mut expired, &usage, &fake, 200).is_err());
            assert_eq!(fake.seen.lock().expect("lock").len(), 1);
            assert_eq!(fs::read(path).expect("read"), before);
        }
    }
    #[test]
    fn transport_failure_keeps_login_and_allows_the_next_read_to_try() {
        let dir = Dir::new("transport");
        let lock = store::hold_server_lock(dir.path()).expect("lock");
        let path = credentials(&lock, "old", "refresh", 0);
        let before = fs::read(&path).expect("read");
        let fake = server(Err(RenewalError::Failed(canary_refresh_token("error"))));
        let mut expired = false;
        assert_eq!(
            read_usage(
                &lock,
                &mut expired,
                &FakeUsage {
                    path: path.clone(),
                    expected: "never sent".into()
                },
                &fake,
                100
            ),
            Err(failed("Warden's sign-in could not be renewed"))
        );
        assert!(!expired);
        assert_eq!(fs::read(path).expect("read"), before);
    }
    #[test]
    fn renewal_error_bodies_are_never_read() {
        struct NoRead;
        impl Read for NoRead {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("must not read");
            }
        }
        for status in [400, 401, 403, 429, 500, 302] {
            assert!(read_response(status, NoRead).is_err());
        }
        let token = canary_token("body");
        assert_no_token(
            "parse failure",
            &format!("{:?}", read_response(200, token.as_bytes())),
            &token,
        );
    }
    #[test]
    fn request_and_transport_match_the_approved_contract() {
        let token = Secret::from("fixture-refresh".to_owned());
        let body: serde_json::Value = serde_json::from_slice(&request_body(&token)).expect("JSON");
        assert_eq!(
            body,
            serde_json::json!({"grant_type":"refresh_token","refresh_token":"fixture-refresh","client_id":CLIENT_ID})
        );
        let real = AnthropicSignInServer::default();
        assert_eq!(real.agent.config().max_redirects(), 0);
        assert_eq!(
            real.agent.config().timeouts().global,
            Some(Duration::from_secs(10))
        );
    }
    #[test]
    fn response_validation_and_secret_debug_hide_tokens() {
        let a = canary_token("answer");
        let r = canary_refresh_token("answer");
        let body = serde_json::to_vec(
            &serde_json::json!({"access_token":a,"refresh_token":r,"expires_in":28800}),
        )
        .expect("JSON");
        let answer = read_response(200, body.as_slice()).expect("answer");
        assert_eq!(answer.access_token.expose(), a);
        assert_eq!(answer.refresh_token.as_ref().expect("refresh").expose(), r);
        for t in [&a, &r] {
            assert_no_token("debug", &format!("{answer:?}"), t);
        }
        for body in [
            r#"{"access_token":"","expires_in":5}"#,
            r#"{"access_token":"x","expires_in":0}"#,
            r#"{"access_token":"x","expires_in":-1}"#,
        ] {
            assert!(read_response(200, body.as_bytes()).is_err());
        }
    }
}
