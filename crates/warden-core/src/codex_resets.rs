//! Codex's banked resets: one-time resets of a Codex account's usage limits, saved to the account until used or
//! expired. Read the way Codex's own client reads them, from ChatGPT's backend with the account's own login: one
//! small request per Codex account, with a usage reading every few hours ([`RESETS_EVERY_MS`]) and on a usage
//! refresh by hand. Only each available reset's id (what using it needs), title and expiry are kept.
//!
//! A reset is used ([`ResetsEndpoint::redeem`]) only when the user confirms it in the app, the way Codex's own client
//! redeems one (`openai/codex`, `codex-rs/backend-client`, `consume_rate_limit_reset_credit_by_id`). Shared by both
//! providers: [`Reset`], [`UseRefused`] and the id checks.

use crate::claude::{describe_transport_error, tls_config};
use crate::policy::ALLOWED_OUTBOUND_HOSTS;
use crate::secret::Secret;
use crate::time::parse_rfc3339_ms;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::time::Duration;
use zeroize::Zeroizing;

pub const RESETS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
/// Where one banked reset is redeemed, by its credit id.
pub const REDEEM_URL: &str =
    "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume";

/// Titles longer than this are cut: the backend writes short ones ("Full reset (Weekly + 5 hr)").
const MAX_TITLE_CHARS: usize = 80;

/// One reset the account can use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reset {
    /// What using it needs: a Claude grant's id (the same for each reset the grant has left) or a Codex credit's id.
    /// `None` when the answer gave none in the expected form, or for a reading saved before ids were kept: such a reset
    /// is shown but cannot be used until it is read again. Not a secret.
    #[serde(default)]
    pub id: Option<String>,
    /// The backend's own short name for it, if it gives one.
    pub title: Option<String>,
    /// When it expires (ms since the epoch); `None` when it does not.
    pub expires_at: Option<i64>,
}

/// How often a usage reading on its interval also reads an account's limit resets (a Claude account's in its usage
/// answer, a Codex account's banked resets): they change rarely. A usage refresh by hand reads them every time.
pub const RESETS_EVERY_MS: i64 = 3 * 60 * 60 * 1000;

/// Whether a usage reading on its interval asks for an account's resets: never read since Warden started, last read
/// [`RESETS_EVERY_MS`] ago or more, or a clock set back since. `last_read` is the last read that gave the resets; a
/// failed one is not counted, so the next reading asks again.
pub fn resets_due(last_read: Option<i64>, now: i64) -> bool {
    last_read.is_none_or(|at| now < at || now - at >= RESETS_EVERY_MS)
}

/// Why using a reset gave none: a short, token-free reason for the log, and the plain sentence the app shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseRefused {
    pub reason: String,
    pub message: String,
}

impl UseRefused {
    pub fn new(reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            message: message.into(),
        }
    }
}

/// Whether `id` is made only of the allowed bytes, 1 to `max` of them.
fn id_of(id: &str, max: usize, allowed: impl Fn(u8) -> bool) -> bool {
    (1..=max).contains(&id.len()) && id.bytes().all(allowed)
}

/// A Claude grant id as the claim request takes it: `^[a-z0-9_-]{1,40}$`.
pub fn valid_grant_id(id: &str) -> bool {
    id_of(id, 40, |b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
    })
}

/// A request id for using a reset: `^[A-Za-z0-9_-]{1,64}$` (a new UUID for each use).
pub fn valid_request_id(id: &str) -> bool {
    id_of(id, 64, |b| {
        b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
    })
}

/// A Codex credit id as it goes into the redeem request's body: letters, digits, `_`, `-`, `.` and `:`, 1 to 128.
pub fn valid_credit_id(id: &str) -> bool {
    id_of(id, 128, |b| {
        b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':')
    })
}

/// Sends the requests the banked resets need. A trait so tests can stand in for ChatGPT's backend.
pub trait ResetsEndpoint: Send + Sync {
    /// The account's available resets, or a short, token-free reason.
    fn fetch(&self, token: &Secret, account_id: &str) -> Result<Vec<Reset>, String>;

    /// Redeems one banked reset by its credit id, `request_id` new for this use; sent only when the user confirmed it
    /// in the app. `Ok` only when the backend answers that it reset the limits. A stand-in that does not redeem
    /// refuses.
    fn redeem(
        &self,
        _token: &Secret,
        _account_id: &str,
        _credit_id: &str,
        _request_id: &str,
    ) -> Result<(), UseRefused> {
        Err(UseRefused::new(
            "not supported",
            "Warden cannot use this reset.",
        ))
    }
}

/// The redeem request's body, as Codex's own client sends it.
pub fn redeem_body(credit_id: &str, request_id: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "redeem_request_id": request_id,
        "credit_id": credit_id,
    }))
    .expect("serialising string fields")
}

/// Refuses a credit or request id of another form before anything is sent.
pub fn check_redeem_ids(credit_id: &str, request_id: &str) -> Result<(), UseRefused> {
    if !valid_credit_id(credit_id) || !valid_request_id(request_id) {
        return Err(UseRefused::new(
            "the reset's id is not in the expected form",
            "Warden cannot use this reset.",
        ));
    }
    Ok(())
}

/// The redeem answer, decided from the status before touching the body; only a successful answer is read, capped and
/// parsed. Only `code: "reset"` is a success.
pub fn read_redeem_response(status: u16, reader: impl Read) -> Result<(), UseRefused> {
    let body = crate::claude::use_answer_body(status, reader)?;
    #[derive(Deserialize)]
    struct Answer {
        code: String,
    }
    let answer: Answer =
        serde_json::from_str(&body).map_err(|_| crate::claude::unexpected_use_answer())?;
    match answer.code.as_str() {
        "reset" => Ok(()),
        "nothing_to_reset" => Err(UseRefused::new(
            "nothing to reset",
            "Nothing to reset: the account is not at a limit.",
        )),
        "no_credit" => Err(UseRefused::new(
            "no such reset",
            "This reset is no longer available.",
        )),
        "already_redeemed" => Err(UseRefused::new(
            "already used",
            "This reset was already used.",
        )),
        _ => Err(crate::claude::unexpected_use_answer()),
    }
}

/// ChatGPT's backend, over HTTPS with Windows' own TLS.
pub struct ChatGptResetsEndpoint {
    agent: ureq::Agent,
}

impl ChatGptResetsEndpoint {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .tls_config(tls_config())
            .timeout_global(Some(Duration::from_secs(10)))
            // Only this endpoint may be called: no redirect is followed, so a 3xx answer counts as a failed read.
            .max_redirects(0)
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Default for ChatGptResetsEndpoint {
    fn default() -> Self {
        Self::new()
    }
}

/// The resets endpoint's host is an allowed outbound host.
pub fn resets_url_is_allowed() -> bool {
    RESETS_URL
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| ALLOWED_OUTBOUND_HOSTS.contains(&host))
}

/// The redeem endpoint's host is an allowed outbound host.
pub fn redeem_url_is_allowed() -> bool {
    REDEEM_URL
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| ALLOWED_OUTBOUND_HOSTS.contains(&host))
}

impl ChatGptResetsEndpoint {
    /// The redeem request, with the list request's headers and the JSON body's type.
    fn redeem_request(
        &self,
        bearer: &str,
        account_id: &str,
    ) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        self.agent
            .post(REDEEM_URL)
            .header("Authorization", bearer)
            .header("chatgpt-account-id", account_id)
            .header("User-Agent", "codex-cli")
            .header("Content-Type", "application/json")
    }
}

impl ResetsEndpoint for ChatGptResetsEndpoint {
    fn redeem(
        &self,
        token: &Secret,
        account_id: &str,
        credit_id: &str,
        request_id: &str,
    ) -> Result<(), UseRefused> {
        if !redeem_url_is_allowed() {
            return Err(UseRefused::new(
                "the redeem endpoint is not an allowed host",
                "Warden cannot use this reset.",
            ));
        }
        check_redeem_ids(credit_id, request_id)?;
        let bearer = Zeroizing::new(format!("Bearer {}", token.expose()));
        let mut response = self
            .redeem_request(bearer.as_str(), account_id)
            .send(redeem_body(credit_id, request_id).as_slice())
            .map_err(|e| crate::claude::use_failed(&describe_transport_error(&e)))?;
        let status = response.status().as_u16();
        read_redeem_response(status, response.body_mut().as_reader())
    }

    fn fetch(&self, token: &Secret, account_id: &str) -> Result<Vec<Reset>, String> {
        if !resets_url_is_allowed() {
            return Err("the resets endpoint is not an allowed host".into());
        }
        let bearer = Zeroizing::new(format!("Bearer {}", token.expose()));
        let mut response = self
            .agent
            .get(RESETS_URL)
            .header("Authorization", bearer.as_str())
            .header("chatgpt-account-id", account_id)
            .header("User-Agent", "codex-cli")
            .call()
            .map_err(|e| describe_transport_error(&e))?;
        let status = response.status().as_u16();
        read_response(status, response.body_mut().as_reader())
    }
}

/// Decides from the status before touching the body; only a successful answer is read, capped and parsed.
pub fn read_response(status: u16, reader: impl Read) -> Result<Vec<Reset>, String> {
    match status {
        200 => {}
        401 => return Err("HTTP 401: the login has expired".into()),
        403 => return Err("HTTP 403: missing permission".into()),
        404 => return Err("HTTP 404: no resets for this account".into()),
        429 => return Err("HTTP 429: rate limited".into()),
        500..=599 => return Err(format!("HTTP {status}: server error")),
        _ => return Err(format!("HTTP {status}: unexpected answer")),
    }
    const MAX_BODY_BYTES: u64 = 256 * 1024;
    let mut body = String::new();
    reader
        .take(MAX_BODY_BYTES + 1)
        .read_to_string(&mut body)
        .map_err(|_| "HTTP 200: the answer was cut off or not text".to_owned())?;
    if body.len() as u64 > MAX_BODY_BYTES {
        return Err("HTTP 200: unexpected answer".into());
    }
    parse_resets(&body).ok_or_else(|| "HTTP 200: unexpected answer".to_owned())
}

#[derive(Deserialize)]
struct Answer {
    credits: Vec<Credit>,
}

#[derive(Deserialize)]
struct Credit {
    id: Option<String>,
    status: String,
    expires_at: Option<String>,
    title: Option<String>,
}

/// The available resets in the backend's answer, soonest to expire first (those that never expire last). `None` for
/// an answer of another shape, an expiry that is not a date among them (it would read as never expiring).
pub fn parse_resets(body: &str) -> Option<Vec<Reset>> {
    let answer: Answer = serde_json::from_str(body).ok()?;
    let mut resets: Vec<Reset> = answer
        .credits
        .into_iter()
        .filter(|c| c.status == "available")
        .map(|c| {
            let expires_at = match c.expires_at.as_deref() {
                Some(at) => Some(parse_rfc3339_ms(at)?),
                None => None,
            };
            Some(Reset {
                id: c.id.filter(|id| valid_credit_id(id)),
                title: c
                    .title
                    .map(|t| t.trim().chars().take(MAX_TITLE_CHARS).collect::<String>())
                    .filter(|t| !t.is_empty()),
                expires_at,
            })
        })
        .collect::<Option<_>>()?;
    resets.sort_by_key(|r| r.expires_at.unwrap_or(i64::MAX));
    Some(resets)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The backend's answer as Codex's own client tests it.
    const ANSWER: &str = r#"{
        "credits": [
            {"id": "credit-1", "reset_type": "codex_rate_limits", "status": "available",
             "granted_at": "2026-06-17T00:00:00Z", "expires_at": "2026-07-17T00:00:00Z",
             "redeem_started_at": null, "redeemed_at": null,
             "profile_image_url": "https://example.test/avatar.png", "profile_user_id": "@friend",
             "title": "Full reset (Weekly + 5 hr)", "description": "Ready to redeem"},
            {"id": "credit-2", "reset_type": "codex_rate_limits", "status": "available",
             "granted_at": "2026-06-18T00:00:00Z", "expires_at": null},
            {"id": "credit-3", "reset_type": "codex_rate_limits", "status": "redeemed",
             "granted_at": "2026-06-01T00:00:00Z", "expires_at": "2026-07-01T00:00:00Z", "title": "Used"},
            {"id": "credit-4", "reset_type": "codex_rate_limits", "status": "available",
             "granted_at": "2026-06-10T00:00:00Z", "expires_at": "2026-07-10T00:00:00Z", "title": "  "}
        ],
        "available_count": 3,
        "total_earned_count": 4
    }"#;

    #[test]
    fn keeps_only_available_resets_with_their_title_and_expiry_soonest_first() {
        let resets = parse_resets(ANSWER).expect("resets");
        assert_eq!(
            resets,
            vec![
                Reset {
                    id: Some("credit-4".into()),
                    title: None,
                    expires_at: parse_rfc3339_ms("2026-07-10T00:00:00Z"),
                },
                Reset {
                    id: Some("credit-1".into()),
                    title: Some("Full reset (Weekly + 5 hr)".into()),
                    expires_at: parse_rfc3339_ms("2026-07-17T00:00:00Z"),
                },
                Reset {
                    id: Some("credit-2".into()),
                    title: None,
                    expires_at: None,
                },
            ]
        );
        // Nothing else from the answer is kept: its id (what redeeming it needs), but no people or pictures.
        let saved = serde_json::to_string(&resets).expect("json");
        for gone in [
            "codex_rate_limits",
            "@friend",
            "example.test",
            "Ready to redeem",
            "granted",
        ] {
            assert!(!saved.contains(gone), "{gone} in {saved}");
        }
    }

    #[test]
    fn an_answer_of_another_shape_is_refused_and_titles_are_short() {
        assert_eq!(parse_resets("[]"), None);
        assert_eq!(parse_resets(r#"{"available_count": 1}"#), None);
        assert_eq!(parse_resets(r#"{"credits": []}"#), Some(vec![]));
        let long = format!(
            r#"{{"credits": [{{"status": "available", "expires_at": null, "title": "{}"}}]}}"#,
            "x".repeat(200)
        );
        let title = parse_resets(&long).expect("resets")[0]
            .title
            .clone()
            .expect("title");
        assert_eq!(title.chars().count(), MAX_TITLE_CHARS);
        // An expiry that is not a date fails the read (the last list stays), unlike one that is null.
        assert_eq!(
            parse_resets(
                r#"{"credits": [{"status": "available", "expires_at": "next week", "title": "x"}]}"#
            ),
            None
        );
        assert_eq!(
            parse_resets(r#"{"credits": [{"status": "redeemed", "expires_at": "next week"}]}"#),
            Some(vec![])
        );
    }

    #[test]
    fn failed_answers_say_why_without_their_body() {
        let body = "secret-looking body";
        for (status, why) in [
            (401, "HTTP 401: the login has expired"),
            (404, "HTTP 404: no resets for this account"),
            (429, "HTTP 429: rate limited"),
            (503, "HTTP 503: server error"),
            (302, "HTTP 302: unexpected answer"),
        ] {
            let e = read_response(status, body.as_bytes()).expect_err("failed");
            assert_eq!(e, why);
            assert!(!e.contains(body));
        }
        assert_eq!(
            read_response(200, "not json".as_bytes()),
            Err("HTTP 200: unexpected answer".into())
        );
        assert_eq!(
            read_response(200, ANSWER.as_bytes()).map(|r| r.len()),
            Ok(3)
        );
    }

    #[test]
    fn resets_are_due_when_never_read_or_three_hours_after_the_last_read() {
        const HOUR: i64 = 60 * 60 * 1000;
        assert_eq!(RESETS_EVERY_MS, 3 * HOUR);
        let read = 10 * HOUR;
        // Never read since Warden started.
        assert!(resets_due(None, read));
        // Under 3 hours since the last read.
        assert!(!resets_due(Some(read), read));
        assert!(!resets_due(Some(read), read + 3 * HOUR - 1));
        // 3 hours or more.
        assert!(resets_due(Some(read), read + 3 * HOUR));
        assert!(resets_due(Some(read), read + 30 * HOUR));
        // A clock set back since the last read does not hold them off for hours.
        assert!(resets_due(Some(read), read - 1));
    }

    #[test]
    fn the_resets_endpoint_is_an_allowed_host() {
        assert!(resets_url_is_allowed());
        assert!(redeem_url_is_allowed());
        assert!(RESETS_URL.starts_with("https://chatgpt.com/"));
        warden_rules::hosts::assert_request_allowed(REDEEM_URL, ALLOWED_OUTBOUND_HOSTS);
    }

    #[test]
    fn a_credit_id_of_another_form_is_not_kept() {
        let answer = |id: &str| {
            format!(r#"{{"credits": [{{"id": {id}, "status": "available", "expires_at": null}}]}}"#)
        };
        for bad in [r#""""#, r#""a b""#, r#""a/b""#, r#""a\"b""#, "null", "7"] {
            let resets = parse_resets(&answer(bad));
            // A number is another shape altogether; the rest keep the reset, without an id.
            if bad == "7" {
                assert_eq!(resets, None);
            } else {
                assert_eq!(resets.expect("resets")[0].id, None, "{bad}");
            }
        }
        let long = format!(r#""{}""#, "a".repeat(129));
        assert_eq!(parse_resets(&answer(&long)).expect("resets")[0].id, None);
        assert_eq!(
            parse_resets(&answer(r#""rlrc_A-1.b:2""#)).expect("resets")[0]
                .id
                .as_deref(),
            Some("rlrc_A-1.b:2")
        );
    }

    #[test]
    fn the_redeem_request_is_codexs_own() {
        // As `openai/codex`'s `consume_rate_limit_reset_credit_by_id`: POST to `/wham/rate-limit-reset-credits/consume`
        // with the list request's headers and the JSON body.
        assert_eq!(
            REDEEM_URL,
            "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume"
        );
        let endpoint = ChatGptResetsEndpoint::new();
        let request = endpoint.redeem_request("Bearer fixture-only", "ws-1");
        assert_eq!(request.method_ref(), Some(&ureq::http::Method::POST));
        assert_eq!(
            request.uri_ref().expect("URI").to_string(),
            REDEEM_URL.to_owned()
        );
        let headers = request.headers_ref().expect("headers");
        assert_eq!(headers["Authorization"], "Bearer fixture-only");
        assert_eq!(headers["chatgpt-account-id"], "ws-1");
        assert_eq!(headers["User-Agent"], "codex-cli");
        assert_eq!(headers["Content-Type"], "application/json");
        let body: serde_json::Value =
            serde_json::from_slice(&redeem_body("credit-1", "req-1")).expect("json");
        assert_eq!(
            body,
            serde_json::json!({"redeem_request_id": "req-1", "credit_id": "credit-1"})
        );
        // No redirect is followed: only the backend itself is called.
        assert_eq!(endpoint.agent.config().max_redirects(), 0);
    }

    #[test]
    fn ids_of_another_form_are_refused_before_anything_is_sent() {
        assert!(check_redeem_ids("credit-1", "0b6f3c1e-7a1d-4c5e-9f00-1a2b3c4d5e6f").is_ok());
        let long = "r".repeat(65);
        for (credit, request) in [
            ("", "req"),
            ("credit 1", "req"),
            ("credit\"1", "req"),
            ("credit-1", ""),
            ("credit-1", "req.1"),
            ("credit-1", long.as_str()),
        ] {
            assert_eq!(
                check_redeem_ids(credit, request)
                    .expect_err("refused")
                    .reason,
                "the reset's id is not in the expected form",
                "{credit:?} {request:?}"
            );
        }
    }

    #[test]
    fn each_redeem_answer_and_status_is_said_in_plain_words() {
        let ok = r#"{"code": "reset", "credit": {"id": "ignored"}, "windows_reset": 2}"#;
        assert_eq!(read_redeem_response(200, ok.as_bytes()), Ok(()));
        for (code, message) in [
            (
                "nothing_to_reset",
                "Nothing to reset: the account is not at a limit.",
            ),
            ("no_credit", "This reset is no longer available."),
            ("already_redeemed", "This reset was already used."),
            (
                "something_new",
                "The reset was not used: unexpected answer.",
            ),
        ] {
            let body = format!(r#"{{"code": "{code}"}}"#);
            assert_eq!(
                read_redeem_response(200, body.as_bytes())
                    .expect_err(code)
                    .message,
                message
            );
        }
        let body = "secret-looking body";
        for (status, reason) in [
            (401, "HTTP 401: the login has expired"),
            (403, "HTTP 403: missing permission"),
            (429, "HTTP 429: rate limited"),
            (503, "HTTP 503: server error"),
            (302, "HTTP 302: unexpected answer"),
            (200, "HTTP 200: unexpected answer"),
        ] {
            let e = read_redeem_response(status, body.as_bytes()).expect_err("failed");
            assert_eq!(e.reason, reason);
            assert!(!e.reason.contains(body) && !e.message.contains(body));
        }
    }
}
