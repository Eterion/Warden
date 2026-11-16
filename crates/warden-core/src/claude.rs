//! Claude accounts: identity from the account's settings file, usage from Anthropic's usage endpoint.
//!
//! The login token is read from `.credentials.json` only inside [`read_usage`], for the one request to the usage
//! endpoint, and dropped (and wiped) when that call returns. Identity comes from `.claude.json` and never touches
//! the token.

use crate::codex_resets::UseRefused;
use crate::model::{Extra, Identity, Limit, ModelLimit, Usage, UsageError};
use crate::policy::ALLOWED_OUTBOUND_HOSTS;
use crate::secret::Secret;
use crate::time::parse_rfc3339_ms;
use serde::Deserialize;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::Zeroizing;

/// The usage endpoint `/usage` uses. Undocumented; its answer is parsed leniently.
pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Added to the request: the answer then also lists the account's limit resets (`cedar_ember`).
const RESETS_QUERY: &str = "cedar_ember=1";
const USAGE_BETA: &str = "oauth-2025-04-20";
/// The Claude CLI version the request names when no installed `claude` CLI is found. The endpoint lists an account's
/// limit resets only to the Claude CLI.
pub const FALLBACK_CLI_VERSION: &str = "2.1.323";
/// At most this many limit resets are kept for an account, however many the answer counts.
const MAX_RESETS: usize = 50;
const CREDENTIALS_FILE: &str = ".credentials.json";
const SETTINGS_FILE: &str = ".claude.json";

/// The preferred sign-in file: beside the default home folder, otherwise inside the account folder.
/// [`settings_files`] also includes the default folder's explicit CLAUDE_CONFIG_DIR layout as a fallback.
pub fn settings_file(folder: &Path, home: Option<&Path>) -> PathBuf {
    if let Some(home) = home
        && crate::detect::same_folder(folder, &home.join(".claude"))
    {
        return home.join(SETTINGS_FILE);
    }
    folder.join(SETTINGS_FILE)
}

/// Files that can hold the sign-in, in priority order. With no CLAUDE_CONFIG_DIR Claude uses the
/// home file; an explicit CLAUDE_CONFIG_DIR can also point at the default folder and use its inner file.
pub fn settings_files(folder: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let primary = settings_file(folder, home);
    let inner = folder.join(SETTINGS_FILE);
    if primary == inner {
        vec![primary]
    } else {
        vec![primary, inner]
    }
}

/// On Windows, both layouts keep the login at `<folder>/.credentials.json`; only settings move to the home.
/// Whether the folder holds a Claude login (the credentials file exists). Does not open it.
pub fn has_login(folder: &Path) -> bool {
    folder.join(CREDENTIALS_FILE).is_file()
}

/// Whether the folder looks like a Claude account folder at all.
pub fn looks_like_account(folder: &Path, home: Option<&Path>) -> bool {
    has_login(folder) || settings_files(folder, home).iter().any(|p| p.is_file())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsFile {
    oauth_account: Option<OauthAccount>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OauthAccount {
    email_address: Option<String>,
    organization_name: Option<String>,
    organization_uuid: Option<String>,
}

/// Who the folder is signed in as, from `oauthAccount` in the settings file. `Ok(None)`: signed out (no account
/// recorded). The settings file holds no token.
pub fn read_identity(folder: &Path, home: Option<&Path>) -> io::Result<Option<Identity>> {
    read_identity_from_files(&settings_files(folder, home))
}

/// Reads sign-in from already resolved files, so event watchers can reuse their generation's paths.
/// Falls back only when a file is absent or has no oauthAccount; a malformed primary file is an error.
pub fn read_identity_from_files(files: &[PathBuf]) -> io::Result<Option<Identity>> {
    for path in files {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        let parsed: SettingsFile = serde_json::from_str(&text)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let Some(a) = parsed.oauth_account else {
            continue;
        };
        return Ok(a
            .email_address
            .filter(|e| !e.trim().is_empty())
            .map(|email| Identity {
                email,
                organization: a.organization_name.filter(|o| !o.trim().is_empty()),
                organization_id: a.organization_uuid.filter(|o| !o.trim().is_empty()),
            }));
    }
    Ok(None)
}

/// Sends the requests Anthropic's API needs: the usage request, and the claim of one limit reset. A trait so tests can
/// stand in for Anthropic.
pub trait UsageEndpoint: Send + Sync {
    /// Calls the endpoint and returns parsed usage or a safe error, never a raw response body. `resets`: whether to ask
    /// for the account's limit resets as well ([`crate::codex_resets::resets_due`]); without, the answer has none.
    fn fetch(&self, token: &Secret, resets: bool) -> Result<Usage, UsageError>;

    /// Claims one limit reset of the grant `grant_id` for the organization, `request_id` new for this use; sent only
    /// when the user confirmed it in the app. `Ok` only when the answer's `result` is `reset`. A stand-in that does not
    /// claim refuses.
    fn use_reset(
        &self,
        _token: &Secret,
        _organization_id: &str,
        _grant_id: &str,
        _request_id: &str,
    ) -> Result<(), UseRefused> {
        Err(UseRefused::new(
            "not supported",
            "Warden cannot use this reset.",
        ))
    }
}

/// Where a limit reset is claimed: `{RESET_URL_START}{organization}{RESET_URL_END}`.
pub const RESET_URL_START: &str = "https://api.anthropic.com/api/organizations/";
pub const RESET_URL_END: &str = "/reset_rate_limits";
/// The program Claude's limit resets belong to (the usage answer's block of the same name).
const RESET_PROGRAM: &str = "cedar_ember";

/// The claim request's address for an organization.
pub fn reset_url(organization_id: &str) -> String {
    format!("{RESET_URL_START}{organization_id}{RESET_URL_END}")
}

/// An organization's UUID as it goes into the claim request's path: letters, digits and `-`, 1 to 64.
pub fn valid_organization_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Refuses an organization, grant or request id of another form before anything is sent.
pub fn check_claim_ids(
    organization_id: &str,
    grant_id: &str,
    request_id: &str,
) -> Result<(), UseRefused> {
    if !valid_organization_id(organization_id)
        || !crate::codex_resets::valid_grant_id(grant_id)
        || !crate::codex_resets::valid_request_id(request_id)
    {
        return Err(UseRefused::new(
            "the reset's id is not in the expected form",
            "Warden cannot use this reset.",
        ));
    }
    Ok(())
}

/// The claim request's body, as the Claude CLI sends it.
pub fn claim_body(grant_id: &str, request_id: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "program": RESET_PROGRAM,
        "grant_id": grant_id,
        "request_id": request_id,
    }))
    .expect("serialising string fields")
}

/// A failed request to use a reset: the short, token-free reason, and it in a sentence.
pub(crate) fn use_failed(reason: &str) -> UseRefused {
    UseRefused::new(reason, format!("The reset was not used ({reason})."))
}

/// An answer to a use of a reset in a shape Warden does not know.
pub(crate) fn unexpected_use_answer() -> UseRefused {
    UseRefused::new(
        "HTTP 200: unexpected answer",
        "The reset was not used: unexpected answer.",
    )
}

/// The answer to a use of a reset (Claude's claim or Codex's redeem), decided from the status before the body is
/// touched; only a successful answer is read, capped. Never any of an error's body.
pub(crate) fn use_answer_body(status: u16, reader: impl Read) -> Result<String, UseRefused> {
    match status {
        200 => {}
        401 => {
            return Err(UseRefused::new(
                "HTTP 401: the login has expired",
                "The account's login has expired.",
            ));
        }
        403 => {
            return Err(UseRefused::new(
                "HTTP 403: missing permission",
                "The account may not use this reset.",
            ));
        }
        429 => {
            return Err(UseRefused::new(
                "HTTP 429: rate limited",
                "Try again later.",
            ));
        }
        500..=599 => {
            return Err(UseRefused::new(
                format!("HTTP {status}: server error"),
                "The server had a problem. Try again later.",
            ));
        }
        _ => {
            return Err(UseRefused::new(
                format!("HTTP {status}: unexpected answer"),
                "The reset was not used: unexpected answer.",
            ));
        }
    }
    const MAX_BODY_BYTES: u64 = 64 * 1024;
    let mut body = String::new();
    reader
        .take(MAX_BODY_BYTES + 1)
        .read_to_string(&mut body)
        .map_err(|_| unexpected_use_answer())?;
    if body.len() as u64 > MAX_BODY_BYTES {
        return Err(unexpected_use_answer());
    }
    Ok(body)
}

/// The claim's answer: only `result: "reset"` is a success; each other result in plain words.
pub fn read_claim_response(status: u16, reader: impl Read) -> Result<(), UseRefused> {
    let body = use_answer_body(status, reader)?;
    #[derive(Deserialize)]
    struct Answer {
        result: String,
    }
    let answer: Answer = serde_json::from_str(&body).map_err(|_| unexpected_use_answer())?;
    let (reason, message) = match answer.result.as_str() {
        "reset" => return Ok(()),
        "already_used" => ("already used", "This reset was already used."),
        "not_limited" => (
            "not at a limit",
            "Nothing to reset: the account is not at a limit.",
        ),
        "cooldown" => ("cooling down", "Try again later."),
        "ineligible" => ("not eligible", "This account cannot use limit resets."),
        "unavailable" => ("unavailable", "This reset cannot be used right now."),
        _ => return Err(unexpected_use_answer()),
    };
    Err(UseRefused::new(reason, message))
}

/// Why a reset could not be used for the login: signed out, expired, someone else's.
pub fn login_refusal(e: &UsageError) -> UseRefused {
    match e {
        UsageError::SignedOut => UseRefused::new("signed out", "The account is signed out."),
        UsageError::LoginExpired { .. } => {
            UseRefused::new("the login has expired", "The account's login has expired.")
        }
        UsageError::IdentityChanged => UseRefused::new(
            "signed in as someone else",
            "The folder is signed in as someone else.",
        ),
        UsageError::NoLogs => use_failed("no login"),
        UsageError::Failed { message } => use_failed(message),
    }
}

/// Windows' own TLS with Windows' own trusted roots, as any Windows program. ureq's default roots (a bundled copy of
/// Mozilla's list) replace Windows' store, and Windows then fails to verify the endpoint's cross-signed chain.
pub(crate) fn tls_config() -> ureq::tls::TlsConfig {
    ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build()
}

/// The real endpoint, over HTTPS with Windows' own TLS. The request identifies as the installed Claude CLI.
pub struct AnthropicUsageEndpoint {
    agent: ureq::Agent,
    user_agent: String,
}

/// The installed `claude` CLI's version, read from the files beside it (nothing is started).
pub fn installed_cli_version(env: &crate::detect::Env) -> Option<String> {
    crate::detect::cli_path(env, crate::model::Provider::Claude)
        .and_then(|cli| crate::detect::cli_version(crate::model::Provider::Claude, &cli))
}

/// The request's `User-Agent`, as the Claude CLI sends it: the given version when it is a plain version (digits and
/// dots, at most 32 characters), otherwise [`FALLBACK_CLI_VERSION`].
pub fn cli_user_agent(version: Option<&str>) -> String {
    let plain = |v: &&str| {
        v.len() <= 32
            && v.split('.').count() >= 2
            && v.split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    let version = version
        .map(str::trim)
        .filter(plain)
        .unwrap_or(FALLBACK_CLI_VERSION);
    format!("claude-cli/{version} (external, cli)")
}

impl AnthropicUsageEndpoint {
    /// `cli_version`: the installed Claude CLI's ([`installed_cli_version`]); `None` uses [`FALLBACK_CLI_VERSION`].
    pub fn new(cli_version: Option<&str>) -> Self {
        let config = ureq::Agent::config_builder()
            .tls_config(tls_config())
            .timeout_global(Some(Duration::from_secs(10)))
            // A redirect could lead anywhere, and only the usage endpoint may be called: none is followed, so a
            // 3xx answer counts as a failed read.
            .max_redirects(0)
            .http_status_as_error(false)
            .build();
        AnthropicUsageEndpoint {
            agent: config.into(),
            user_agent: cli_user_agent(cli_version),
        }
    }

    /// The usage request, with [`RESETS_QUERY`] only when `resets` asks for the limit resets. The CLI headers are on
    /// every request.
    fn request(
        &self,
        bearer: &str,
        resets: bool,
    ) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        let url = if resets {
            format!("{USAGE_URL}?{RESETS_QUERY}")
        } else {
            USAGE_URL.to_owned()
        };
        self.agent
            .get(url)
            .header("Authorization", bearer)
            .header("anthropic-beta", USAGE_BETA)
            .header("User-Agent", self.user_agent.as_str())
            .header("x-app", "cli")
    }

    /// The claim of a limit reset: the usage request's headers, and the JSON body's type.
    fn claim_request(
        &self,
        bearer: &str,
        organization_id: &str,
    ) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        self.agent
            .post(reset_url(organization_id))
            .header("Authorization", bearer)
            .header("anthropic-beta", USAGE_BETA)
            .header("User-Agent", self.user_agent.as_str())
            .header("x-app", "cli")
            .header("Content-Type", "application/json")
    }
}

impl Default for AnthropicUsageEndpoint {
    fn default() -> Self {
        Self::new(None)
    }
}

/// The usage endpoint's host is the one allowed outbound host.
pub fn usage_url_is_allowed() -> bool {
    USAGE_URL
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| ALLOWED_OUTBOUND_HOSTS.contains(&host))
}

impl UsageEndpoint for AnthropicUsageEndpoint {
    fn fetch(&self, token: &Secret, resets: bool) -> Result<Usage, UsageError> {
        if !usage_url_is_allowed() {
            return Err(failed("the usage endpoint is not an allowed host"));
        }
        let bearer = Zeroizing::new(format!("Bearer {}", token.expose()));
        let mut response = self
            .request(bearer.as_str(), resets)
            .call()
            .map_err(|e| failed(&describe_transport_error(&e)))?;
        let status = response.status().as_u16();
        read_response(status, response.body_mut().as_reader())
    }

    fn use_reset(
        &self,
        token: &Secret,
        organization_id: &str,
        grant_id: &str,
        request_id: &str,
    ) -> Result<(), UseRefused> {
        if !reset_url_is_allowed() {
            return Err(UseRefused::new(
                "the claim endpoint is not an allowed host",
                "Warden cannot use this reset.",
            ));
        }
        check_claim_ids(organization_id, grant_id, request_id)?;
        let bearer = Zeroizing::new(format!("Bearer {}", token.expose()));
        let mut response = self
            .claim_request(bearer.as_str(), organization_id)
            .send(claim_body(grant_id, request_id).as_slice())
            .map_err(|e| use_failed(&describe_transport_error(&e)))?;
        let status = response.status().as_u16();
        read_claim_response(status, response.body_mut().as_reader())
    }
}

/// The claim endpoint's host is an allowed outbound host.
pub fn reset_url_is_allowed() -> bool {
    RESET_URL_START
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| ALLOWED_OUTBOUND_HOSTS.contains(&host))
}

/// Decides from the status before touching the body. Only a successful answer is read, capped and parsed.
/// The timestamp is filled by `read_usage` once the call completes.
pub fn read_response(status: u16, reader: impl Read) -> Result<Usage, UsageError> {
    match status {
        200 => {}
        401 => {
            return Err(UsageError::LoginExpired { status: Some(401) });
        }
        403 => return Err(failed("HTTP 403: missing permission")),
        429 => return Err(failed("HTTP 429: rate limited")),
        500..=599 => return Err(failed(&format!("HTTP {status}: server error"))),
        _ => return Err(failed(&format!("HTTP {status}: unexpected answer"))),
    }
    const MAX_BODY_BYTES: u64 = 256 * 1024;
    let mut body = String::new();
    reader
        .take(MAX_BODY_BYTES + 1)
        .read_to_string(&mut body)
        .map_err(|_| failed("HTTP 200: the answer was cut off or not text"))?;
    if body.len() as u64 > MAX_BODY_BYTES {
        return Err(failed("HTTP 200: unexpected answer"));
    }
    parse_usage(&body, 0).map_err(|_| failed("HTTP 200: unexpected answer"))
}

/// A short, token-free description of a failed request.
pub(crate) fn describe_transport_error(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => "network timeout".into(),
        ureq::Error::HostNotFound => "network error: host could not be found".into(),
        ureq::Error::Io(_) | ureq::Error::ConnectionFailed => {
            "network error: connection failed".into()
        }
        ureq::Error::Tls(_)
        | ureq::Error::NativeTls(_)
        | ureq::Error::Der(_)
        | ureq::Error::TlsRequired => "TLS error: secure connection failed".into(),
        ureq::Error::Http(_) => "the login token cannot be sent".into(),
        ureq::Error::Protocol(_)
        | ureq::Error::LargeResponseHeader(_, _)
        | ureq::Error::BodyStalled => "unexpected HTTP answer".into(),
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => "redirect refused".into(),
        ureq::Error::ConnectProxyFailed(_) | ureq::Error::InvalidProxyUrl => {
            "network proxy error".into()
        }
        ureq::Error::BodyExceedsLimit(_) => "answer too large".into(),
        _ => "unexpected request error".into(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialsFile {
    claude_ai_oauth: Option<OauthLogin>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
/// Only these two fields are read; every other field (the refresh token among them) is skipped, never kept.
struct OauthLogin {
    access_token: Option<Secret>,
    expires_at: Option<i64>,
}

/// Reads the account's usage: the login token is read, sent to the usage endpoint once, and dropped. `resets`: whether
/// the request also asks for the account's limit resets.
pub fn read_usage(
    folder: &Path,
    endpoint: &dyn UsageEndpoint,
    now_ms: i64,
    resets: bool,
) -> Result<Usage, UsageError> {
    let token = read_token(folder, now_ms)?;
    let result = endpoint.fetch(&token, resets);
    drop(token);
    result.map(|mut usage| {
        usage.read_at = now_ms;
        usage
    })
}

/// Uses one of the account's limit resets, after the user confirmed it: the login token is read, sent to the claim
/// endpoint once with the organization's id, and dropped. An expired login is not sent (Warden never renews a Warden
/// account's login).
pub fn use_reset(
    folder: &Path,
    endpoint: &dyn UsageEndpoint,
    now_ms: i64,
    (organization_id, grant_id, request_id): (&str, &str, &str),
) -> Result<(), UseRefused> {
    check_claim_ids(organization_id, grant_id, request_id)?;
    let token = read_token(folder, now_ms).map_err(|e| login_refusal(&e))?;
    let result = endpoint.use_reset(&token, organization_id, grant_id, request_id);
    drop(token);
    result
}

/// The folder's login token, unless it has expired; wiped when dropped.
fn read_token(folder: &Path, now_ms: i64) -> Result<Secret, UsageError> {
    let path = folder.join(CREDENTIALS_FILE);
    let bytes = match fs::read(&path) {
        Ok(b) => Zeroizing::new(b),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(UsageError::SignedOut),
        Err(_) => return Err(failed("the login file could not be read")),
    };
    let creds: CredentialsFile = serde_json::from_slice(&bytes)
        .map_err(|_| failed("the login file is not in a known format"))?;
    drop(bytes);
    let Some(login) = creds.claude_ai_oauth else {
        return Err(UsageError::SignedOut);
    };
    let Some(token) = login.access_token.filter(|t| !t.is_empty()) else {
        return Err(UsageError::SignedOut);
    };
    if login.expires_at.is_some_and(|exp| exp <= now_ms) {
        return Err(UsageError::LoginExpired { status: None });
    }
    Ok(token)
}

fn failed(message: &str) -> UsageError {
    UsageError::Failed {
        message: message.into(),
    }
}

/// Parses the usage endpoint's answer. Lenient: unknown fields are ignored and any limit may be missing, but an
/// answer with neither the 5-hour nor the weekly limit counts as a change in the endpoint.
pub fn parse_usage(body: &str, now_ms: i64) -> Result<Usage, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| "the usage endpoint answered in an unknown shape".to_owned())?;
    let obj = value
        .as_object()
        .ok_or("the usage endpoint answered in an unknown shape")?;
    if !obj.contains_key("five_hour") && !obj.contains_key("seven_day") {
        return Err("the usage endpoint answered without limits; it may have changed".into());
    }
    let limit = |v: &serde_json::Value| -> Option<Limit> {
        let used = v.get("utilization")?.as_f64()?;
        Some(Limit {
            used_percent: used.clamp(0.0, 100.0),
            resets_at: v
                .get("resets_at")
                .and_then(|r| r.as_str())
                .and_then(parse_rfc3339_ms),
        })
    };
    let mut per_model_weekly: Vec<ModelLimit> = obj
        .iter()
        .filter_map(|(k, v)| {
            let model = k.strip_prefix("seven_day_")?;
            Some(ModelLimit {
                model: model.to_owned(),
                limit: limit(v)?,
            })
        })
        .collect();
    per_model_weekly.sort_by(|a, b| a.model.cmp(&b.model));
    let extra = obj
        .get("extra_usage")
        .filter(|v| v.is_object())
        .map(|v| Extra {
            enabled: v
                .get("is_enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            used_credits: v.get("used_credits").and_then(serde_json::Value::as_f64),
            monthly_limit: v.get("monthly_limit").and_then(serde_json::Value::as_f64),
            used_percent: v.get("utilization").and_then(serde_json::Value::as_f64),
        });
    Ok(Usage {
        read_at: now_ms,
        five_hour: obj.get("five_hour").and_then(limit),
        weekly: obj.get("seven_day").and_then(limit),
        per_model_weekly,
        extra,
        resets: obj.get("cedar_ember").and_then(parse_resets),
    })
}

#[derive(Deserialize)]
struct ResetsBlock {
    eligible: Option<bool>,
    #[serde(default)]
    grants: Option<Vec<Grant>>,
}

#[derive(Deserialize)]
struct Grant {
    /// What claiming one of its resets needs; kept only in the claim request's form.
    #[serde(default)]
    id: Option<serde_json::Value>,
    resets_left: u64,
    ends_at: Option<String>,
    #[serde(default)]
    gives_back_weekly: bool,
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    clears: Vec<String>,
}

/// A grant's short name, as the Claude CLI names its kind: a Fable reset if it gives the weekly limit back; else its
/// `type`; else from the limits it clears (only the 5-hour one: session; no 5-hour one: weekly; both: full). A
/// `type` the CLI does not know is named from the limits it clears.
fn grant_name(g: &Grant) -> &'static str {
    if g.gives_back_weekly {
        return "Fable reset";
    }
    match g.kind.as_deref() {
        Some("session") => return "5-hour reset",
        Some("weekly") => return "Weekly reset",
        Some("full") => return "Full reset",
        _ => {}
    }
    let five_hour = g.clears.iter().any(|c| c == "five_hour");
    let other = g.clears.iter().any(|c| c != "five_hour");
    match (five_hour, other) {
        (true, false) => "5-hour reset",
        (false, _) => "Weekly reset",
        (true, true) => "Full reset",
    }
}

/// The account's limit resets in the answer's `cedar_ember` block: one per reset left, named by its kind, soonest to
/// expire first (those that never expire last). An account the endpoint calls not eligible has none. `None` for a
/// block of another shape or an expiry that is not a date, so the last list stays. Only each grant's id (in the claim
/// request's form, otherwise none) goes with its resets; nothing else is kept.
fn parse_resets(block: &serde_json::Value) -> Option<Vec<crate::codex_resets::Reset>> {
    let block: ResetsBlock = serde_json::from_value(block.clone()).ok()?;
    if block.eligible == Some(false) {
        return Some(Vec::new());
    }
    let mut resets = Vec::new();
    for g in block.grants.unwrap_or_default() {
        if g.resets_left == 0 {
            continue;
        }
        let expires_at = match g.ends_at.as_deref() {
            Some(at) => Some(parse_rfc3339_ms(at)?),
            None => None,
        };
        let left = usize::try_from(g.resets_left).unwrap_or(MAX_RESETS);
        let reset = crate::codex_resets::Reset {
            id: g
                .id
                .as_ref()
                .and_then(serde_json::Value::as_str)
                .filter(|id| crate::codex_resets::valid_grant_id(id))
                .map(str::to_owned),
            title: Some(grant_name(&g).to_owned()),
            expires_at,
        };
        resets.extend(std::iter::repeat_n(reset, left.min(MAX_RESETS)));
    }
    resets.sort_by_key(|r| r.expires_at.unwrap_or(i64::MAX));
    resets.truncate(MAX_RESETS);
    Some(resets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir as tempdir;
    use std::sync::Mutex;
    use warden_rules::hosts::assert_request_allowed;
    use warden_rules::secrets::{assert_no_token, canary_token};

    struct Fake {
        status: u16,
        body: String,
        seen: Mutex<Option<String>>,
        /// Whether each request asked for the limit resets.
        resets_asked: Mutex<Vec<bool>>,
    }

    impl UsageEndpoint for Fake {
        fn fetch(&self, token: &Secret, resets: bool) -> Result<Usage, UsageError> {
            *self.seen.lock().expect("lock") = Some(token.expose().to_owned());
            self.resets_asked.lock().expect("lock").push(resets);
            read_response(self.status, self.body.as_bytes())
        }
    }

    fn fake(status: u16, body: &str) -> Fake {
        Fake {
            status,
            body: body.into(),
            seen: Mutex::new(None),
            resets_asked: Mutex::new(Vec::new()),
        }
    }

    const ANSWER: &str = r#"{
        "five_hour": {"utilization": 61.0, "resets_at": "2026-11-07T12:10:00+00:00"},
        "seven_day": {"utilization": 11, "resets_at": "2026-11-13T05:00:00Z"},
        "seven_day_oauth_apps": null,
        "seven_day_opus": {"utilization": 4.5, "resets_at": null},
        "extra_usage": {"is_enabled": true, "monthly_limit": 5000, "used_credits": 1240.0, "utilization": 24.8},
        "something_new": [1, 2]
    }"#;

    fn folder_with(creds: &str) -> tempdir::Dir {
        let dir = tempdir::Dir::new("claude");
        fs::write(dir.path().join(CREDENTIALS_FILE), creds).expect("write");
        dir
    }

    #[test]
    fn the_usage_url_is_allowed() {
        assert!(usage_url_is_allowed());
        assert_request_allowed(USAGE_URL, ALLOWED_OUTBOUND_HOSTS);
    }

    #[test]
    fn parses_the_answer() {
        let u = parse_usage(ANSWER, 7).expect("parses");
        assert_eq!(u.read_at, 7);
        assert_eq!(
            u.five_hour,
            Some(Limit {
                used_percent: 61.0,
                resets_at: Some(1_794_053_400_000)
            })
        );
        assert_eq!(u.weekly.as_ref().map(|l| l.used_percent), Some(11.0));
        assert_eq!(u.per_model_weekly.len(), 1);
        assert_eq!(u.per_model_weekly[0].model, "opus");
        let extra = u.extra.expect("extra");
        assert!(extra.enabled);
        assert_eq!(extra.used_credits, Some(1240.0));
    }

    #[test]
    fn a_missing_five_hour_limit_is_none() {
        let u = parse_usage(r#"{"five_hour": null, "seven_day": {"utilization": 3}}"#, 0)
            .expect("parses");
        assert_eq!(u.five_hour, None);
        assert_eq!(
            u.weekly,
            Some(Limit {
                used_percent: 3.0,
                resets_at: None
            })
        );
    }

    #[test]
    fn an_unknown_answer_is_an_error() {
        assert!(parse_usage("<html>", 0).is_err());
        assert!(parse_usage(r#"{"limits": []}"#, 0).is_err());
    }

    #[test]
    fn reads_usage_with_the_token_and_returns_no_token() {
        let token = canary_token("claude");
        let dir = folder_with(&format!(
            r#"{{"claudeAiOauth": {{"accessToken": "{token}", "refreshToken": "{token}-r", "expiresAt": 9999999999999, "scopes": ["user:inference"]}}}}"#
        ));
        let endpoint = fake(200, ANSWER);
        let usage = read_usage(dir.path(), &endpoint, 1000, true).expect("reads");
        assert_eq!(*endpoint.resets_asked.lock().expect("lock"), vec![true]);
        read_usage(dir.path(), &endpoint, 1000, false).expect("reads");
        assert_eq!(
            *endpoint.resets_asked.lock().expect("lock"),
            vec![true, false]
        );
        assert_eq!(
            endpoint.seen.lock().expect("lock").as_deref(),
            Some(token.as_str())
        );
        let json = serde_json::to_string(&usage).expect("serialises");
        assert_no_token("usage reading", &json, &token);
    }

    #[test]
    fn an_expired_login_is_not_sent() {
        let dir = folder_with(
            r#"{"claudeAiOauth": {"accessToken": "sk-ant-oat01-x", "expiresAt": 500}}"#,
        );
        let endpoint = fake(200, ANSWER);
        assert_eq!(
            read_usage(dir.path(), &endpoint, 1000, true),
            Err(UsageError::LoginExpired { status: None })
        );
        assert!(endpoint.seen.lock().expect("lock").is_none());
    }

    #[test]
    fn statuses_map_to_errors() {
        let dir = folder_with(r#"{"claudeAiOauth": {"accessToken": "sk-ant-oat01-x"}}"#);
        let token = canary_token("error-body");
        for (status, expected) in [
            (401, UsageError::LoginExpired { status: Some(401) }),
            (403, failed("HTTP 403: missing permission")),
            (429, failed("HTTP 429: rate limited")),
            (500, failed("HTTP 500: server error")),
            (599, failed("HTTP 599: server error")),
            (302, failed("HTTP 302: unexpected answer")),
            (404, failed("HTTP 404: unexpected answer")),
            (200, failed("HTTP 200: unexpected answer")),
        ] {
            let error =
                read_usage(dir.path(), &fake(status, &token), 0, true).expect_err("failed read");
            assert_eq!(error, expected);
            assert_no_token(
                "usage error",
                &serde_json::to_string(&error).expect("json"),
                &token,
            );
        }
    }

    #[test]
    fn the_endpoint_trusts_the_roots_windows_trusts() {
        let tls = tls_config();
        assert_eq!(tls.provider(), ureq::tls::TlsProvider::NativeTls);
        assert!(matches!(
            tls.root_certs(),
            ureq::tls::RootCerts::PlatformVerifier
        ));
    }

    #[test]
    fn error_bodies_are_never_read_and_no_body_or_reader_error_escapes() {
        use std::cell::Cell;
        struct CanaryReader<'a> {
            bytes: &'a [u8],
            reads: &'a Cell<usize>,
            fail: bool,
        }
        impl Read for CanaryReader<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                self.reads.set(self.reads.get() + 1);
                if self.fail {
                    return Err(io::Error::other(
                        String::from_utf8_lossy(self.bytes).into_owned(),
                    ));
                }
                self.bytes.read(buffer)
            }
        }
        let canary = canary_token("response-reader");
        for status in [401, 403, 429, 500, 200] {
            for fail in [false, true] {
                let reads = Cell::new(0);
                let result = read_response(
                    status,
                    CanaryReader {
                        bytes: canary.as_bytes(),
                        reads: &reads,
                        fail,
                    },
                );
                assert!(result.is_err());
                assert_no_token("response result", &format!("{result:?}"), &canary);
                assert_no_token(
                    "response JSON",
                    &serde_json::to_string(&result).expect("json"),
                    &canary,
                );
                if status != 200 {
                    assert_eq!(reads.get(), 0, "HTTP {status} must not read any body bytes");
                } else if fail {
                    assert_eq!(
                        result,
                        Err(failed("HTTP 200: the answer was cut off or not text"))
                    );
                }
            }
        }
        assert_eq!(
            read_response(200, &[255][..]),
            Err(failed("HTTP 200: the answer was cut off or not text"))
        );
        assert_eq!(
            read_response(200, ANSWER.as_bytes())
                .expect("valid body")
                .weekly
                .expect("week")
                .used_percent,
            11.0
        );
        let oversized = " ".repeat(256 * 1024 + 1);
        assert_eq!(
            read_response(200, oversized.as_bytes()),
            Err(failed("HTTP 200: unexpected answer"))
        );
    }

    #[test]
    fn request_identifies_as_the_installed_claude_cli_and_asks_for_the_resets_only_when_told() {
        let endpoint = AnthropicUsageEndpoint::new(Some("2.1.400"));
        let request = endpoint.request("Bearer fixture-only", true);
        assert_eq!(
            request.uri_ref().expect("URI").to_string(),
            format!("{USAGE_URL}?cedar_ember=1")
        );
        assert_eq!(request.method_ref(), Some(&ureq::http::Method::GET));
        let headers = request.headers_ref().expect("headers");
        assert_eq!(headers["Authorization"], "Bearer fixture-only");
        assert_eq!(headers["anthropic-beta"], "oauth-2025-04-20");
        assert_eq!(headers["User-Agent"], "claude-cli/2.1.400 (external, cli)");
        assert_eq!(headers["x-app"], "cli");
        // Without the resets: the plain usage request, with the same headers.
        let plain = endpoint.request("Bearer fixture-only", false);
        assert_eq!(plain.uri_ref().expect("URI").to_string(), USAGE_URL);
        let plain_headers = plain.headers_ref().expect("headers");
        for name in ["Authorization", "anthropic-beta", "User-Agent", "x-app"] {
            assert_eq!(plain_headers[name], headers[name], "{name}");
        }
        // No CLI found, or a version that is not plain digits and dots: the fixed fallback.
        for version in [
            None,
            Some(""),
            Some("2"),
            Some("2.1.x"),
            Some("2.1.0\r\nx: y"),
            Some("2.1.0 (Claude Code)"),
        ] {
            let request =
                AnthropicUsageEndpoint::new(version).request("Bearer fixture-only", false);
            assert_eq!(
                request.headers_ref().expect("headers")["User-Agent"],
                "claude-cli/2.1.323 (external, cli)",
                "{version:?}"
            );
        }
        assert_eq!(
            AnthropicUsageEndpoint::default().user_agent,
            "claude-cli/2.1.323 (external, cli)"
        );
    }

    #[test]
    fn the_cli_version_comes_from_the_files_beside_the_installed_cli() {
        let dir = tempdir::Dir::new("cli");
        let bin = dir.path().join("npm");
        let package = bin.join("node_modules/@anthropic-ai/claude-code");
        fs::create_dir_all(&package).expect("mkdir");
        fs::write(bin.join("claude.cmd"), "").expect("cli");
        fs::write(package.join("package.json"), r#"{"version": "2.1.401"}"#).expect("package");
        let mut env = crate::detect::Env::from_process();
        env.path = vec![bin];
        assert_eq!(installed_cli_version(&env).as_deref(), Some("2.1.401"));
        env.path = vec![dir.path().join("nowhere")];
        assert_eq!(installed_cli_version(&env), None);
    }

    /// The `cedar_ember` block as the probe found it, with grants of every kind.
    const RESETS_ANSWER: &str = r#"{
        "five_hour": {"utilization": 10, "resets_at": null},
        "seven_day": {"utilization": 20, "resets_at": null},
        "cedar_ember": {
            "eligible": true, "ineligible_reason": null, "at_limit": false, "exhausted": [],
            "grants": [
                {"id": "grant-full", "label": "One full limit reset (November 2026)", "resets_total": 2,
                 "resets_left": 2, "starts_at": "2026-11-01T00:00:00Z", "ends_at": "2026-11-30T23:00:00Z",
                 "clears": ["five_hour", "seven_day"], "paused": false, "usable_now": true,
                 "use_requires_limit": false, "gives_back_weekly": false, "unavailable_reason": null},
                {"id": "grant-fable", "label": "New model launch: one Fable reset", "resets_total": 1,
                 "resets_left": 1, "starts_at": null, "ends_at": "2026-11-20T10:00:00Z",
                 "clears": ["seven_day"], "type": "weekly", "gives_back_weekly": true},
                {"id": "grant-session", "label": "Session", "resets_total": 1, "resets_left": 1,
                 "ends_at": null, "clears": ["five_hour"], "gives_back_weekly": false},
                {"id": "grant-typed", "label": "Typed", "resets_total": 1, "resets_left": 1,
                 "ends_at": "2026-12-05T00:00:00Z", "clears": ["five_hour"], "type": "full"},
                {"id": "grant-weekly", "label": "Weekly", "resets_total": 1, "resets_left": 1,
                 "ends_at": "2026-12-01T00:00:00Z", "clears": ["seven_day", "seven_day_overage_included"]},
                {"id": "grant-used", "label": "Used up", "resets_total": 1, "resets_left": 0,
                 "ends_at": "2026-11-17T00:00:00Z", "clears": ["five_hour"]}
            ],
            "next_grant_id": "grant-x", "weekly_resets_at": null, "cooldown_until": null,
            "event_props": {"campaign": "fixture-campaign"}
        }
    }"#;

    fn reset(id: &str, title: &str, at: Option<&str>) -> crate::codex_resets::Reset {
        crate::codex_resets::Reset {
            id: Some(id.into()),
            title: Some(title.into()),
            expires_at: at.map(|a| parse_rfc3339_ms(a).expect("date")),
        }
    }

    #[test]
    fn limit_resets_one_per_reset_left_with_short_names_soonest_first() {
        let resets = parse_usage(RESETS_ANSWER, 0)
            .expect("parses")
            .resets
            .expect("resets");
        assert_eq!(
            resets,
            vec![
                reset("grant-fable", "Fable reset", Some("2026-11-20T10:00:00Z")),
                reset("grant-full", "Full reset", Some("2026-11-30T23:00:00Z")),
                reset("grant-full", "Full reset", Some("2026-11-30T23:00:00Z")),
                reset("grant-weekly", "Weekly reset", Some("2026-12-01T00:00:00Z")),
                reset("grant-typed", "Full reset", Some("2026-12-05T00:00:00Z")),
                reset("grant-session", "5-hour reset", None),
            ]
        );
        // Nothing else from the block is kept: each grant's id (what claiming it needs), but no labels or event props.
        let saved = serde_json::to_string(&resets).expect("json");
        for gone in ["grant-x", "November", "Fable reset for", "fixture-campaign"] {
            assert!(!saved.contains(gone), "{gone} in {saved}");
        }
        // The usage reading itself never carries them.
        let usage =
            serde_json::to_string(&parse_usage(RESETS_ANSWER, 0).expect("parses")).expect("json");
        assert!(!usage.contains("\"resets\""), "{usage}");
        assert!(!usage.contains("Fable"), "{usage}");
    }

    #[test]
    fn an_ineligible_account_has_no_resets_and_a_missing_or_malformed_block_none_read() {
        let with = |block: &str| {
            parse_usage(
                &format!(r#"{{"seven_day": {{"utilization": 1}}, "cedar_ember": {block}}}"#),
                0,
            )
            .expect("usage still parses")
            .resets
        };
        let ineligible = r#"{"eligible": false, "ineligible_reason": "surface", "grants": [
            {"resets_left": 1, "ends_at": null, "clears": ["five_hour"]}]}"#;
        assert_eq!(with(ineligible), Some(vec![]));
        assert_eq!(with(r#"{"eligible": true, "grants": []}"#), Some(vec![]));
        assert_eq!(with(r#"{"eligible": true}"#), Some(vec![]));
        // Not read: the last list stays.
        assert_eq!(parse_usage(ANSWER, 0).expect("parses").resets, None);
        for malformed in [
            "null",
            "[]",
            r#""yes""#,
            r#"{"eligible": "yes"}"#,
            r#"{"grants": [{"ends_at": null}]}"#,
            r#"{"grants": [{"resets_left": -1, "ends_at": null}]}"#,
            r#"{"grants": [{"resets_left": 1, "ends_at": "next week"}]}"#,
            r#"{"grants": {"resets_left": 1}}"#,
        ] {
            assert_eq!(with(malformed), None, "{malformed}");
        }
        // However many the answer counts, at most this many are kept, soonest first.
        let many = with(
            r#"{"grants": [{"resets_left": 4000000000, "ends_at": null, "clears": ["five_hour"]},
                {"resets_left": 1, "ends_at": "2026-12-01T00:00:00Z", "clears": ["seven_day"]}]}"#,
        )
        .expect("resets");
        assert_eq!(many.len(), MAX_RESETS);
        assert_eq!(many[0].title.as_deref(), Some("Weekly reset"));
    }

    #[test]
    fn a_grant_id_not_in_the_claim_requests_form_is_not_kept() {
        let ids = |id: &str| {
            parse_usage(
                &format!(
                    r#"{{"seven_day": {{"utilization": 1}}, "cedar_ember": {{"eligible": true, "grants": [
                        {{"id": {id}, "resets_left": 1, "ends_at": null, "clears": ["five_hour"]}}]}}}}"#
                ),
                0,
            )
            .expect("parses")
            .resets
            .expect("resets")[0]
                .id
                .clone()
        };
        assert_eq!(ids(r#""grant_1-a""#).as_deref(), Some("grant_1-a"));
        let long = format!(r#""{}""#, "g".repeat(41));
        for bad in [
            r#""""#,
            r#""Grant-1""#,
            r#""grant 1""#,
            r#""grant/1""#,
            "7",
            "null",
            long.as_str(),
        ] {
            assert_eq!(ids(bad), None, "{bad}");
        }
    }

    /// Stands in for the claim endpoint: the token, organization and grant it was sent, and its answer.
    struct Claims {
        seen: Mutex<Vec<(String, String, String, String)>>,
        status: u16,
        body: &'static str,
    }

    impl UsageEndpoint for Claims {
        fn fetch(&self, _: &Secret, _: bool) -> Result<Usage, UsageError> {
            panic!("only the claim is sent")
        }
        fn use_reset(
            &self,
            token: &Secret,
            organization_id: &str,
            grant_id: &str,
            request_id: &str,
        ) -> Result<(), UseRefused> {
            self.seen.lock().expect("lock").push((
                token.expose().to_owned(),
                organization_id.to_owned(),
                grant_id.to_owned(),
                request_id.to_owned(),
            ));
            read_claim_response(self.status, self.body.as_bytes())
        }
    }

    fn claims(status: u16, body: &'static str) -> Claims {
        Claims {
            seen: Mutex::new(Vec::new()),
            status,
            body,
        }
    }

    #[test]
    fn the_claim_request_is_the_claude_clis() {
        assert_eq!(
            reset_url("org-1"),
            "https://api.anthropic.com/api/organizations/org-1/reset_rate_limits"
        );
        assert!(reset_url_is_allowed());
        assert_request_allowed(&reset_url("org-1"), ALLOWED_OUTBOUND_HOSTS);
        let endpoint = AnthropicUsageEndpoint::new(Some("2.1.323"));
        let request = endpoint.claim_request("Bearer fixture-only", "0e5b8b0c-org");
        assert_eq!(request.method_ref(), Some(&ureq::http::Method::POST));
        assert_eq!(
            request.uri_ref().expect("URI").to_string(),
            "https://api.anthropic.com/api/organizations/0e5b8b0c-org/reset_rate_limits"
        );
        let headers = request.headers_ref().expect("headers");
        assert_eq!(headers["Authorization"], "Bearer fixture-only");
        assert_eq!(headers["anthropic-beta"], "oauth-2025-04-20");
        assert_eq!(headers["User-Agent"], "claude-cli/2.1.323 (external, cli)");
        assert_eq!(headers["x-app"], "cli");
        assert_eq!(headers["Content-Type"], "application/json");
        let body: serde_json::Value =
            serde_json::from_slice(&claim_body("grant-full", "req-1")).expect("json");
        assert_eq!(
            body,
            serde_json::json!({"program": "cedar_ember", "grant_id": "grant-full", "request_id": "req-1"})
        );
        // The same client as the usage request: no redirect is followed.
        assert_eq!(endpoint.agent.config().max_redirects(), 0);
    }

    #[test]
    fn ids_of_another_form_are_refused_before_the_login_is_read_or_anything_sent() {
        let dir = folder_with(r#"{"claudeAiOauth": {"accessToken": "fixture-only"}}"#);
        let uuid = "0b6f3c1e-7a1d-4c5e-9f00-1a2b3c4d5e6f";
        let long_grant = "g".repeat(41);
        let long_request = "r".repeat(65);
        for (org, grant, request) in [
            ("", "grant", uuid),
            ("org/../x", "grant", uuid),
            ("org?x=1", "grant", uuid),
            ("org", "", uuid),
            ("org", "Grant", uuid),
            ("org", "grant.1", uuid),
            ("org", long_grant.as_str(), uuid),
            ("org", "grant", ""),
            ("org", "grant", "req 1"),
            ("org", "grant", long_request.as_str()),
        ] {
            let endpoint = claims(200, r#"{"result": "reset"}"#);
            let refused =
                use_reset(dir.path(), &endpoint, 0, (org, grant, request)).expect_err("refused");
            assert_eq!(refused.reason, "the reset's id is not in the expected form");
            assert!(endpoint.seen.lock().expect("lock").is_empty());
        }
        let endpoint = claims(200, r#"{"result": "reset"}"#);
        assert_eq!(
            use_reset(dir.path(), &endpoint, 0, ("org-1", "grant_1", uuid)),
            Ok(())
        );
        assert_eq!(
            *endpoint.seen.lock().expect("lock"),
            vec![(
                "fixture-only".to_owned(),
                "org-1".to_owned(),
                "grant_1".to_owned(),
                uuid.to_owned()
            )]
        );
    }

    #[test]
    fn an_expired_or_missing_login_is_not_sent_to_the_claim() {
        let expired = folder_with(
            r#"{"claudeAiOauth": {"accessToken": "sk-ant-oat01-x", "expiresAt": 500}}"#,
        );
        let endpoint = claims(200, r#"{"result": "reset"}"#);
        let ids = ("org-1", "grant", "req-1");
        assert_eq!(
            use_reset(expired.path(), &endpoint, 1000, ids)
                .expect_err("expired")
                .message,
            "The account's login has expired."
        );
        let none = tempdir::Dir::new("claude");
        assert_eq!(
            use_reset(none.path(), &endpoint, 1000, ids)
                .expect_err("signed out")
                .message,
            "The account is signed out."
        );
        assert!(endpoint.seen.lock().expect("lock").is_empty());
    }

    #[test]
    fn each_claim_result_and_status_is_said_in_plain_words() {
        let ok = r#"{"result": "reset", "reason": null, "resets_left": 1, "cleared": ["five_hour", "seven_day"],
            "weekly_resets_at": "2026-11-20T10:00:00Z", "cooldown_until": null}"#;
        assert_eq!(read_claim_response(200, ok.as_bytes()), Ok(()));
        for (result, message) in [
            ("already_used", "This reset was already used."),
            (
                "not_limited",
                "Nothing to reset: the account is not at a limit.",
            ),
            ("cooldown", "Try again later."),
            ("ineligible", "This account cannot use limit resets."),
            ("unavailable", "This reset cannot be used right now."),
            (
                "something_new",
                "The reset was not used: unexpected answer.",
            ),
        ] {
            let body = format!(
                r#"{{"result": "{result}", "reason": "fixture-reason", "resets_left": 0, "cleared": []}}"#
            );
            let refused = read_claim_response(200, body.as_bytes()).expect_err(result);
            assert_eq!(refused.message, message);
            // The answer's own reason text is never repeated.
            assert!(
                !refused.message.contains("fixture-reason")
                    && !refused.reason.contains("fixture-reason")
            );
        }
        let canary = canary_token("claim-body");
        for (status, reason, message) in [
            (
                401,
                "HTTP 401: the login has expired",
                "The account's login has expired.",
            ),
            (
                403,
                "HTTP 403: missing permission",
                "The account may not use this reset.",
            ),
            (429, "HTTP 429: rate limited", "Try again later."),
            (
                500,
                "HTTP 500: server error",
                "The server had a problem. Try again later.",
            ),
            (
                302,
                "HTTP 302: unexpected answer",
                "The reset was not used: unexpected answer.",
            ),
            (
                200,
                "HTTP 200: unexpected answer",
                "The reset was not used: unexpected answer.",
            ),
        ] {
            let refused = read_claim_response(status, canary.as_bytes()).expect_err("refused");
            assert_eq!(
                (refused.reason.as_str(), refused.message.as_str()),
                (reason, message)
            );
            assert_no_token("claim refusal", &format!("{refused:?}"), &canary);
        }
        let oversized = format!(r#"{{"result": "reset", "x": "{}"}}"#, " ".repeat(64 * 1024));
        assert!(read_claim_response(200, oversized.as_bytes()).is_err());
    }

    #[test]
    fn transport_errors_never_include_the_underlying_message() {
        let token = canary_token("transport");
        let header_error = ureq::http::Request::builder()
            .header("Authorization", format!("Bearer {token}\n"))
            .body(())
            .expect_err("invalid token characters");
        for (error, expected) in [
            (
                ureq::Error::Http(header_error),
                "the login token cannot be sent",
            ),
            (
                ureq::Error::Timeout(ureq::Timeout::Global),
                "network timeout",
            ),
            (
                ureq::Error::ConnectionFailed,
                "network error: connection failed",
            ),
            (
                ureq::Error::TlsRequired,
                "TLS error: secure connection failed",
            ),
            (ureq::Error::TooManyRedirects, "redirect refused"),
            (ureq::Error::RedirectFailed, "redirect refused"),
            (ureq::Error::InvalidProxyUrl, "network proxy error"),
            (
                ureq::Error::LargeResponseHeader(1, 2),
                "unexpected HTTP answer",
            ),
            (ureq::Error::BodyStalled, "unexpected HTTP answer"),
            (
                ureq::Error::HostNotFound,
                "network error: host could not be found",
            ),
            (
                ureq::Error::Io(io::Error::other(token.clone())),
                "network error: connection failed",
            ),
            (
                ureq::Error::Tls("fixture-only"),
                "TLS error: secure connection failed",
            ),
            (
                ureq::Error::ConnectProxyFailed(token.clone()),
                "network proxy error",
            ),
            (
                ureq::Error::BodyExceedsLimit(256 * 1024),
                "answer too large",
            ),
        ] {
            let message = describe_transport_error(&error);
            assert_eq!(message, expected);
            assert_no_token("transport error", &message, &token);
        }
    }

    #[test]
    fn the_usage_client_follows_no_redirects() {
        let endpoint = AnthropicUsageEndpoint::new(None);
        assert_eq!(endpoint.agent.config().max_redirects(), 0);
        assert_eq!(
            endpoint.agent.config().timeouts().global,
            Some(Duration::from_secs(10))
        );
    }

    #[test]
    fn no_login_is_signed_out() {
        let dir = tempdir::Dir::new("claude");
        assert_eq!(
            read_usage(dir.path(), &fake(200, ANSWER), 0, true),
            Err(UsageError::SignedOut)
        );
        let dir = folder_with(r#"{"other": {}}"#);
        assert_eq!(
            read_usage(dir.path(), &fake(200, ANSWER), 0, true),
            Err(UsageError::SignedOut)
        );
    }

    #[test]
    fn identity_from_the_settings_file() {
        let dir = tempdir::Dir::new("claude");
        fs::write(
            dir.path().join(SETTINGS_FILE),
            r#"{"numStartups": 3, "projects": {"x": {}}, "oauthAccount": {"accountUuid": "u1", "emailAddress": "ann@example.com", "organizationUuid": "org-1", "organizationName": "Acme", "displayName": "Ann"}}"#,
        )
        .expect("write");
        let id = read_identity(dir.path(), None)
            .expect("reads")
            .expect("signed in");
        assert_eq!(id.email, "ann@example.com");
        assert_eq!(id.organization.as_deref(), Some("Acme"));
        assert_eq!(id.organization_id.as_deref(), Some("org-1"));
    }

    #[test]
    fn identity_beside_the_default_folder() {
        let home = tempdir::Dir::new("home");
        let folder = home.path().join(".claude");
        fs::create_dir(&folder).expect("mkdir");
        fs::write(
            home.path().join(SETTINGS_FILE),
            r#"{"oauthAccount": {"emailAddress": "ann@example.com"}}"#,
        )
        .expect("write");
        fs::write(folder.join(SETTINGS_FILE), r#"{"numStartups": 1}"#).expect("inner file");
        let id = read_identity(&folder, Some(home.path()))
            .expect("reads")
            .expect("signed in");
        assert_eq!(id.email, "ann@example.com");
        assert_eq!(id.organization, None);
    }

    #[test]
    fn the_default_folder_can_also_be_an_explicit_config_dir() {
        let home = tempdir::Dir::new("home");
        let folder = home.path().join(".claude");
        fs::create_dir(&folder).expect("mkdir");
        let primary = home.path().join(SETTINGS_FILE);
        let inner = folder.join(SETTINGS_FILE);
        fs::write(
            &inner,
            r#"{"oauthAccount":{"emailAddress":"inner@example.com"}}"#,
        )
        .expect("write");
        assert_eq!(
            settings_files(&folder, Some(home.path())),
            vec![primary.clone(), inner]
        );
        for text in [None, Some("{}"), Some(r#"{"oauthAccount":null}"#)] {
            if let Some(text) = text {
                fs::write(&primary, text).expect("write");
            }
            assert_eq!(
                read_identity(&folder, Some(home.path()))
                    .expect("reads")
                    .expect("identity")
                    .email,
                "inner@example.com"
            );
        }
        fs::write(
            &primary,
            r#"{"oauthAccount":{"emailAddress":"home@example.com"}}"#,
        )
        .expect("write");
        assert_eq!(
            read_identity(&folder, Some(home.path()))
                .expect("reads")
                .expect("identity")
                .email,
            "home@example.com"
        );
        fs::write(&primary, "{").expect("write");
        assert!(
            read_identity(&folder, Some(home.path())).is_err(),
            "a malformed primary is not a fallback signal"
        );
    }

    #[test]
    fn a_custom_folder_named_claude_does_not_use_its_parent_identity() {
        let home = tempdir::Dir::new("home");
        let folder = home.path().join("other/.claude");
        fs::create_dir_all(&folder).expect("mkdir");
        fs::write(
            folder.parent().expect("parent").join(SETTINGS_FILE),
            r#"{"oauthAccount":{"emailAddress":"wrong@example.com"}}"#,
        )
        .expect("write");
        assert_eq!(
            read_identity(&folder, Some(home.path())).expect("reads"),
            None
        );
        fs::write(
            folder.join(SETTINGS_FILE),
            r#"{"oauthAccount":{"emailAddress":"right@example.com"}}"#,
        )
        .expect("write");
        assert_eq!(
            read_identity(&folder, Some(home.path()))
                .expect("reads")
                .expect("identity")
                .email,
            "right@example.com"
        );
    }

    #[test]
    fn credentials_stay_inside_both_layouts() {
        let home = tempdir::Dir::new("home");
        // A stray credentials file in the home must never be used.
        fs::write(home.path().join(CREDENTIALS_FILE), "invalid").expect("write");
        for name in [".claude", ".claude-work"] {
            let folder = home.path().join(name);
            fs::create_dir(&folder).expect("mkdir");
            assert!(!has_login(&folder));
            assert_eq!(
                read_usage(&folder, &fake(200, ANSWER), 0, true),
                Err(UsageError::SignedOut)
            );
            fs::write(
                folder.join(CREDENTIALS_FILE),
                r#"{"claudeAiOauth":{"accessToken":"fixture-only"}}"#,
            )
            .expect("write");
            assert!(has_login(&folder));
            assert!(read_usage(&folder, &fake(200, ANSWER), 0, true).is_ok());
        }
    }

    #[test]
    fn no_oauth_account_is_signed_out() {
        let dir = tempdir::Dir::new("claude");
        fs::write(dir.path().join(SETTINGS_FILE), r#"{"numStartups": 1}"#).expect("write");
        assert_eq!(read_identity(dir.path(), None).expect("reads"), None);
        let empty = tempdir::Dir::new("claude");
        assert_eq!(read_identity(empty.path(), None).expect("reads"), None);
    }
}
