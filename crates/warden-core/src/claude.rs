//! Claude accounts: identity from the account's settings file, usage from Anthropic's usage endpoint.
//!
//! The login token is read from `.credentials.json` only inside [`read_usage`], for the one request to the usage
//! endpoint, and dropped (and wiped) when that call returns. Identity comes from `.claude.json` and never touches
//! the token.

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
const USAGE_BETA: &str = "oauth-2025-04-20";
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

/// Sends the one request the usage endpoint needs. A trait so tests can stand in for Anthropic.
pub trait UsageEndpoint: Send + Sync {
    /// Calls the endpoint and returns parsed usage or a safe error, never a raw response body.
    fn fetch(&self, token: &Secret) -> Result<Usage, UsageError>;
}

/// Windows' own TLS with Windows' own trusted roots, as any Windows program. ureq's default roots (a bundled copy of
/// Mozilla's list) replace Windows' store, and Windows then fails to verify the endpoint's cross-signed chain.
pub(crate) fn tls_config() -> ureq::tls::TlsConfig {
    ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build()
}

/// The real endpoint, over HTTPS with Windows' own TLS.
pub struct AnthropicUsageEndpoint {
    agent: ureq::Agent,
}

impl AnthropicUsageEndpoint {
    pub fn new() -> Self {
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
        }
    }

    fn request(&self, bearer: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        self.agent
            .get(USAGE_URL)
            .header("Authorization", bearer)
            .header("anthropic-beta", USAGE_BETA)
            .header("User-Agent", "claude-code/2.1.0")
    }
}

impl Default for AnthropicUsageEndpoint {
    fn default() -> Self {
        Self::new()
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
    fn fetch(&self, token: &Secret) -> Result<Usage, UsageError> {
        if !usage_url_is_allowed() {
            return Err(failed("the usage endpoint is not an allowed host"));
        }
        let bearer = Zeroizing::new(format!("Bearer {}", token.expose()));
        let mut response = self
            .request(bearer.as_str())
            .call()
            .map_err(|e| failed(&describe_transport_error(&e)))?;
        let status = response.status().as_u16();
        read_response(status, response.body_mut().as_reader())
    }
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

/// Reads the account's usage: the login token is read, sent to the usage endpoint once, and dropped.
pub fn read_usage(
    folder: &Path,
    endpoint: &dyn UsageEndpoint,
    now_ms: i64,
) -> Result<Usage, UsageError> {
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
    let result = endpoint.fetch(&token);
    drop(token);
    result.map(|mut usage| {
        usage.read_at = now_ms;
        usage
    })
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
    })
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
    }

    impl UsageEndpoint for Fake {
        fn fetch(&self, token: &Secret) -> Result<Usage, UsageError> {
            *self.seen.lock().expect("lock") = Some(token.expose().to_owned());
            read_response(self.status, self.body.as_bytes())
        }
    }

    fn fake(status: u16, body: &str) -> Fake {
        Fake {
            status,
            body: body.into(),
            seen: Mutex::new(None),
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
        let usage = read_usage(dir.path(), &endpoint, 1000).expect("reads");
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
            read_usage(dir.path(), &endpoint, 1000),
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
            let error = read_usage(dir.path(), &fake(status, &token), 0).expect_err("failed read");
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
    fn request_matches_the_claude_oauth_client() {
        let endpoint = AnthropicUsageEndpoint::new();
        let request = endpoint.request("Bearer fixture-only");
        assert_eq!(request.uri_ref().expect("URI").to_string(), USAGE_URL);
        assert_eq!(request.method_ref(), Some(&ureq::http::Method::GET));
        let headers = request.headers_ref().expect("headers");
        assert_eq!(headers["Authorization"], "Bearer fixture-only");
        assert_eq!(headers["anthropic-beta"], "oauth-2025-04-20");
        assert_eq!(headers["User-Agent"], "claude-code/2.1.0");
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
        let endpoint = AnthropicUsageEndpoint::new();
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
            read_usage(dir.path(), &fake(200, ANSWER), 0),
            Err(UsageError::SignedOut)
        );
        let dir = folder_with(r#"{"other": {}}"#);
        assert_eq!(
            read_usage(dir.path(), &fake(200, ANSWER), 0),
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
                read_usage(&folder, &fake(200, ANSWER), 0),
                Err(UsageError::SignedOut)
            );
            fs::write(
                folder.join(CREDENTIALS_FILE),
                r#"{"claudeAiOauth":{"accessToken":"fixture-only"}}"#,
            )
            .expect("write");
            assert!(has_login(&folder));
            assert!(read_usage(&folder, &fake(200, ANSWER), 0).is_ok());
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
