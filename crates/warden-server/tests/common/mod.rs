//! Shared by the server's integration tests: fixture account folders, a stand-in usage endpoint, a captured log, and
//! a plain HTTP client. Nothing here touches a real account or the network.

#![allow(
    dead_code,
    reason = "each test file uses a different part of this module"
)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use warden_core::claude::UsageEndpoint;
use warden_core::detect::Env;
use warden_core::model::Provider;
use warden_core::secret::Secret;
use warden_core::testdir::Dir;
use warden_server::app::Desktop;
use warden_server::{Config, Running, start};

pub fn folder_bytes(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, at: &Path, files: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(at).expect("directory") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if entry.file_type().expect("type").is_dir() {
                walk(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).expect("relative").to_owned(),
                    std::fs::read(path).expect("file"),
                );
            }
        }
    }
    let mut files = std::collections::BTreeMap::new();
    walk(root, root, &mut files);
    files
}

pub const CLAUDE_ANSWER: &str = r#"{
    "five_hour": {"utilization": 61.0, "resets_at": "2026-11-07T12:10:00+00:00"},
    "seven_day": {"utilization": 11.0, "resets_at": "2026-11-13T05:00:00+00:00"},
    "extra_usage": {"is_enabled": false, "monthly_limit": null, "used_credits": null, "utilization": null}
}"#;

/// Stands in for Anthropic's usage endpoint and remembers the tokens it was sent and whether each request asked for the
/// limit resets. As the endpoint, it answers the `cedar_ember` block only when asked.
#[derive(Default)]
pub struct FakeEndpoint {
    pub tokens_seen: Mutex<Vec<String>>,
    pub resets_asked: Mutex<Vec<bool>>,
    pub answer: Mutex<Option<(u16, String)>>,
    /// Each claim of a limit reset: the token, organization id, grant id and request id.
    pub claims: Mutex<Vec<(String, String, String, String)>>,
    /// The claim's answer as Anthropic's status and body; `None`: `{"result": "reset"}`.
    pub claim_answer: Mutex<Option<(u16, String)>>,
    /// Runs once while the next request is in flight.
    #[allow(clippy::type_complexity)]
    pub during: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl UsageEndpoint for FakeEndpoint {
    fn fetch(
        &self,
        token: &Secret,
        resets: bool,
    ) -> Result<warden_core::model::Usage, warden_core::model::UsageError> {
        self.tokens_seen
            .lock()
            .expect("lock")
            .push(token.expose().to_owned());
        self.resets_asked.lock().expect("lock").push(resets);
        let during = self.during.lock().expect("lock").take();
        if let Some(during) = during {
            during();
        }
        let (status, body) = self
            .answer
            .lock()
            .expect("lock")
            .clone()
            .unwrap_or((200, CLAUDE_ANSWER.to_owned()));
        let body = if resets { body } else { without_resets(&body) };
        warden_core::claude::read_response(status, body.as_bytes())
    }

    fn use_reset(
        &self,
        token: &Secret,
        organization_id: &str,
        grant_id: &str,
        request_id: &str,
    ) -> Result<(), warden_core::codex_resets::UseRefused> {
        self.claims.lock().expect("lock").push((
            token.expose().to_owned(),
            organization_id.to_owned(),
            grant_id.to_owned(),
            request_id.to_owned(),
        ));
        let (status, body) = self.claim_answer.lock().expect("lock").clone().unwrap_or((
            200,
            r#"{"result": "reset", "resets_left": 0, "cleared": ["five_hour", "seven_day"]}"#
                .to_owned(),
        ));
        warden_core::claude::read_claim_response(status, body.as_bytes())
    }
}

/// The usage answer as it comes without `?cedar_ember=1`: the same, without the limit resets block. A body that is not
/// a JSON object stays as it is.
pub fn without_resets(body: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(serde_json::Value::Object(mut answer)) => {
            answer.remove("cedar_ember");
            serde_json::Value::Object(answer).to_string()
        }
        _ => body.to_owned(),
    }
}

pub struct FakeSignInServer {
    data: PathBuf,
    pub tokens_seen: Mutex<Vec<String>>,
    pub answers: Mutex<
        std::collections::VecDeque<
            Result<warden_core::mainlogin::Renewed, warden_core::mainlogin::RenewalError>,
        >,
    >,
}

impl warden_core::mainlogin::SignInServer for FakeSignInServer {
    fn renew(
        &self,
        token: &Secret,
    ) -> Result<warden_core::mainlogin::Renewed, warden_core::mainlogin::RenewalError> {
        assert!(
            warden_core::store::server_running(&self.data),
            "the renewer must hold the lock"
        );
        self.tokens_seen
            .lock()
            .expect("lock")
            .push(token.expose().to_owned());
        self.answers
            .lock()
            .expect("lock")
            .pop_front()
            .expect("a fake renewal answer must be queued")
    }
}

/// Stands in for ChatGPT's backend: records what each resets request was sent and answers the backend's own body.
#[derive(Default)]
pub struct FakeResetsEndpoint {
    pub seen: Mutex<Vec<(String, String)>>,
    pub answer: Mutex<Option<(u16, String)>>,
    /// Each redeem: the token, account id, credit id and request id.
    pub redeemed: Mutex<Vec<(String, String, String, String)>>,
    /// The redeem's answer as the backend's status and body; `None`: `{"code": "reset"}`.
    pub redeem_answer: Mutex<Option<(u16, String)>>,
    /// Runs once while the next request is in flight.
    #[allow(clippy::type_complexity)]
    pub during: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

/// The backend's answer for one available reset.
pub const RESETS_ANSWER: &str = r#"{"credits": [{"id": "credit-1", "reset_type": "codex_rate_limits", "status": "available", "granted_at": "2026-11-01T00:00:00Z", "expires_at": "2026-12-10T22:00:00Z", "title": "Full reset (Weekly + 5 hr)", "description": "Ready to redeem"}], "available_count": 1}"#;

impl warden_core::codex_resets::ResetsEndpoint for FakeResetsEndpoint {
    fn fetch(
        &self,
        token: &Secret,
        account_id: &str,
    ) -> Result<Vec<warden_core::codex_resets::Reset>, String> {
        self.seen
            .lock()
            .expect("lock")
            .push((token.expose().to_owned(), account_id.to_owned()));
        let during = self.during.lock().expect("lock").take();
        if let Some(during) = during {
            during();
        }
        let (status, body) = self
            .answer
            .lock()
            .expect("lock")
            .clone()
            .unwrap_or((200, r#"{"credits": [], "available_count": 0}"#.to_owned()));
        warden_core::codex_resets::read_response(status, body.as_bytes())
    }

    fn redeem(
        &self,
        token: &Secret,
        account_id: &str,
        credit_id: &str,
        request_id: &str,
    ) -> Result<(), warden_core::codex_resets::UseRefused> {
        self.redeemed.lock().expect("lock").push((
            token.expose().to_owned(),
            account_id.to_owned(),
            credit_id.to_owned(),
            request_id.to_owned(),
        ));
        let (status, body) = self
            .redeem_answer
            .lock()
            .expect("lock")
            .clone()
            .unwrap_or((200, r#"{"code": "reset", "windows_reset": 2}"#.to_owned()));
        warden_core::codex_resets::read_redeem_response(status, body.as_bytes())
    }
}

pub struct Fixture {
    pub home: Dir,
    pub data: Dir,
    pub endpoint: Arc<FakeEndpoint>,
    pub sign_in_server: Arc<FakeSignInServer>,
    pub resets: Arc<FakeResetsEndpoint>,
    pub log: Arc<Mutex<String>>,
    pub sign_ins: Arc<Mutex<Vec<(Provider, PathBuf, PathBuf)>>>,
    /// The `warden` binary the routing hook is written to run; it need not exist unless a test runs the hook.
    pub route_exe: PathBuf,
    /// The Git Bash Claude Code would run hooks in (`CLAUDE_CODE_GIT_BASH_PATH`); `None`: none, so PowerShell.
    pub claude_git_bash: Option<PathBuf>,
}

impl Fixture {
    pub fn new() -> Fixture {
        let data = Dir::new("data");
        let sign_in_server = Arc::new(FakeSignInServer {
            data: data.path().to_owned(),
            tokens_seen: Mutex::new(Vec::new()),
            answers: Mutex::new(std::collections::VecDeque::new()),
        });
        let home = Dir::new("home");
        std::fs::create_dir_all(home.path().join(".claude")).expect("main");
        std::fs::create_dir_all(home.path().join("AppData/Local/AnthropicClaude/app-1.0"))
            .expect("app");
        set_claude_identity(&home.path().join(".claude"), "fixture-main@example.com");
        Fixture {
            route_exe: data.path().join("bin").join("warden.exe"),
            home,
            data,
            endpoint: Arc::new(FakeEndpoint::default()),
            resets: Arc::new(FakeResetsEndpoint::default()),
            sign_in_server,
            log: Arc::new(Mutex::new(String::new())),
            sign_ins: Arc::new(Mutex::new(Vec::new())),
            claude_git_bash: None,
        }
    }

    pub fn env(&self) -> Env {
        let bin = self.home.path().join("bin");
        Env {
            #[cfg(debug_assertions)]
            no_hooks: false,
            home: Some(self.home.path().to_owned()),
            local_app_data: Some(self.home.path().join("AppData/Local")),
            path: vec![bin],
            claude_config_dir: None,
            codex_home: None,
            claude_git_bash: self.claude_git_bash.clone(),
            program_files: Vec::new(),
        }
    }

    pub fn start(&self) -> Running {
        self.start_with(None)
    }

    /// Starts the server as the desktop app does, with its own parts stood in for.
    pub fn start_with(&self, desktop: Option<Arc<dyn Desktop>>) -> Running {
        let log = Arc::clone(&self.log);
        let sign_ins = Arc::clone(&self.sign_ins);
        let running = start(
            Config {
                data_dir: self.data.path().to_owned(),
                ui_dir: None,
                env: self.env(),
                endpoint: self.endpoint.clone(),
                sign_in_server: self.sign_in_server.clone(),
                resets: self.resets.clone(),
                route_exe: Some(self.route_exe.clone()),
                sign_in: Arc::new(move |p, cli, folder, _done| {
                    sign_ins
                        .lock()
                        .expect("lock")
                        .push((p, cli.to_owned(), folder.to_owned()));
                    Ok(())
                }),
                log: Arc::new(move |line| {
                    let mut l = log.lock().expect("lock");
                    l.push_str(line);
                    l.push('\n');
                }),
                desktop,
            },
            0,
        )
        .expect("server starts");
        TOKENS
            .lock()
            .expect("lock")
            .insert(running.addr, cookie(&running));
        running
    }

    /// A Claude account folder signed in as `email`, with this access token.
    pub fn claude_folder(&self, name: &str, email: &str, token: &str) -> PathBuf {
        let folder = self.home.path().join(name);
        std::fs::create_dir_all(&folder).expect("mkdir");
        write(
            &folder.join(".credentials.json"),
            &format!(
                r#"{{"claudeAiOauth": {{"accessToken": "{token}", "refreshToken": "{token}-refresh", "expiresAt": 9999999999999, "scopes": ["user:inference"], "subscriptionType": "team"}}}}"#
            ),
        );
        set_claude_identity(&folder, email);
        folder
    }

    /// Warden's own made-up main sign-in. Never an account folder.
    pub fn main_sign_in(&self, email: &str, access: &str, refresh: &str, expires: i64) -> PathBuf {
        let folder = warden_core::mainlogin::folder(self.data.path());
        std::fs::create_dir_all(&folder).expect("own folder");
        set_claude_identity(&folder, email);
        write(
            &folder.join(".credentials.json"),
            &serde_json::json!({"claudeAiOauth": {
                "accessToken": access, "refreshToken": refresh, "expiresAt": expires
            }})
            .to_string(),
        );
        folder
    }

    /// A Codex account folder signed in as `email`, with this access token in its login file.
    pub fn codex_folder(&self, name: &str, email: &str, token: &str) -> PathBuf {
        let folder = self.home.path().join(name);
        std::fs::create_dir_all(folder.join("sessions")).expect("mkdir");
        set_codex_identity(&folder, email, token);
        folder
    }

    pub fn log_text(&self) -> String {
        self.log.lock().expect("lock").clone()
    }
}

/// Sends a hook event as `warden event` does: its JSON cut down to what the hook keeps, over the pipe to the server
/// whose data folder this is. Whether the server took it (it answers once the event is applied, known account or
/// not).
pub fn hook_event(data_dir: &Path, account: &str, event: &serde_json::Value) -> bool {
    let Some(event) = warden_core::pipe::HookEvent::from_hook_input(event.to_string().as_bytes())
    else {
        return false;
    };
    warden_core::pipe::send(
        data_dir,
        &warden_core::pipe::Message {
            account: account.to_owned(),
            event,
        },
    )
    .is_ok()
}

pub fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, text).expect("write");
}

pub fn claude_identity_file(folder: &Path) -> PathBuf {
    // Fixture homes own their .claude default folder; all other names use CLAUDE_CONFIG_DIR.
    let home = folder.parent().expect("fixture home");
    warden_core::claude::settings_file(folder, Some(home))
}

pub fn set_claude_identity(folder: &Path, email: &str) {
    write(
        &claude_identity_file(folder),
        &format!(
            r#"{{"numStartups": 4, "oauthAccount": {{"accountUuid": "acc-{email}", "emailAddress": "{email}", "organizationUuid": "org-acme", "organizationName": "Acme", "displayName": "Someone"}}}}"#
        ),
    );
}

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

pub fn set_codex_identity(folder: &Path, email: &str, token: &str) {
    set_codex_plan(folder, email, token, None);
}

pub fn set_codex_plan(folder: &Path, email: &str, token: &str, plan: Option<&str>) {
    let plan = serde_json::to_string(&plan).expect("fixture plan");
    let claims = format!(
        r#"{{"email": "{email}", "https://api.openai.com/auth": {{"chatgpt_account_id": "ws-{email}", "chatgpt_plan_type": {plan}, "organizations": [{{"id": "org-1", "title": "Acme", "is_default": true}}]}}}}"#
    );
    let jwt = format!(
        "{}.{}.signature",
        b64(br#"{"alg":"RS256"}"#),
        b64(claims.as_bytes())
    );
    write(
        &folder.join("auth.json"),
        &format!(
            r#"{{"OPENAI_API_KEY": null, "tokens": {{"id_token": "{jwt}", "access_token": "{token}", "refresh_token": "{token}-refresh", "account_id": "ws-1"}}, "last_refresh": "2026-11-01T00:00:00Z"}}"#
        ),
    );
}

/// A `token_count` event as Codex writes it to its session log.
pub fn codex_token_count(ts: &str, five: f64, week: f64) -> String {
    format!(
        r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":null,"rate_limits":{{"primary":{{"used_percent":{five},"window_minutes":300,"resets_at":1794060000}},"secondary":{{"used_percent":{week},"window_minutes":10080,"resets_at":1794400000}}}}}}}}"#
    )
}

pub struct Reply {
    pub status: u16,
    /// The status line and headers, as sent.
    pub head: String,
    pub body: String,
}

impl Reply {
    /// A header's value (the first one with this name, case ignored).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
        })
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).expect("JSON body")
    }
}

/// Sends one HTTP request with exactly these headers (no Host unless given) and reads the whole answer.
pub fn raw(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Reply {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    let mut req = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }
    stream.write_all(req.as_bytes()).expect("send");
    let mut out = Vec::new();
    let _ = stream.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .map(|(h, b)| (h.to_owned(), b.to_owned()))
        .unwrap_or_default();
    Reply { status, head, body }
}

/// Each running test server's `Cookie` header with its token, so [`call`] can send it.
static TOKENS: Mutex<std::collections::BTreeMap<SocketAddr, String>> =
    Mutex::new(std::collections::BTreeMap::new());

/// The `Cookie` header the app's own page sends once it has traded the server's token for its cookie.
pub fn cookie(running: &Running) -> String {
    format!("{}={}", running.token.cookie_name(), running.token.value())
}

/// A request as the app's own page sends it, with the token's cookie.
pub fn call(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> Reply {
    let host = format!("127.0.0.1:{}", addr.port());
    let cookie = TOKENS
        .lock()
        .expect("lock")
        .get(&addr)
        .cloned()
        .expect("a server this test started");
    raw(
        addr,
        method,
        path,
        &[("Host", &host), ("Cookie", &cookie)],
        body,
    )
}

/// Polls `f` until it holds (for the file watcher and the background threads). The limit is on the clock, not the
/// number of tries, and generous, so a machine busy with a parallel build only makes the wait longer.
pub fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !f() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

impl FakeEndpoint {
    /// Waits until the background usage reads (adding an account wakes the usage loop) are done and none is asked
    /// for, then returns how many calls the endpoint has seen, so a test can count only the calls that follow.
    pub fn settled_calls(&self, server: &Running) -> usize {
        eventually("the background usage reads to finish", || {
            server.app.usage_idle()
        });
        self.tokens_seen.lock().expect("lock").len()
    }

    /// The tokens the endpoint was sent after the first `from` calls.
    pub fn tokens_since(&self, from: usize) -> Vec<String> {
        self.tokens_seen.lock().expect("lock")[from..].to_vec()
    }
}
