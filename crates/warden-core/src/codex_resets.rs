//! Codex's banked resets: one-time resets of a Codex account's usage limits, saved to the account until used or
//! expired. Read the way Codex's own client reads them, from ChatGPT's backend with the account's own login: one
//! small request per Codex account on the usage interval. Only each available reset's title and expiry are kept.

use crate::claude::{describe_transport_error, tls_config};
use crate::policy::ALLOWED_OUTBOUND_HOSTS;
use crate::secret::Secret;
use crate::time::parse_rfc3339_ms;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::time::Duration;
use zeroize::Zeroizing;

pub const RESETS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";

/// Titles longer than this are cut: the backend writes short ones ("Full reset (Weekly + 5 hr)").
const MAX_TITLE_CHARS: usize = 80;

/// One reset the account can use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reset {
    /// The backend's own short name for it, if it gives one.
    pub title: Option<String>,
    /// When it expires (ms since the epoch); `None` when it does not.
    pub expires_at: Option<i64>,
}

/// Sends the one request the resets need. A trait so tests can stand in for ChatGPT's backend.
pub trait ResetsEndpoint: Send + Sync {
    /// The account's available resets, or a short, token-free reason.
    fn fetch(&self, token: &Secret, account_id: &str) -> Result<Vec<Reset>, String>;
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

impl ResetsEndpoint for ChatGptResetsEndpoint {
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
                    title: None,
                    expires_at: parse_rfc3339_ms("2026-07-10T00:00:00Z"),
                },
                Reset {
                    title: Some("Full reset (Weekly + 5 hr)".into()),
                    expires_at: parse_rfc3339_ms("2026-07-17T00:00:00Z"),
                },
                Reset {
                    title: None,
                    expires_at: None,
                },
            ]
        );
        // Nothing else from the answer is kept: no ids, people or pictures.
        let saved = serde_json::to_string(&resets).expect("json");
        for gone in [
            "credit-1",
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
    fn the_resets_endpoint_is_an_allowed_host() {
        assert!(resets_url_is_allowed());
        assert!(RESETS_URL.starts_with("https://chatgpt.com/"));
    }
}
