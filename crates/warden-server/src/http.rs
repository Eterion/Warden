//! The development server's HTTP side: the Host, Origin and `Sec-Fetch-Site` checks, the per-run token, and the
//! answers from `api.rs` written out over HTTP. The desktop app does not use it: it answers in-process.

use crate::api::{self, Answer, MAX_BODY, SECURITY_HEADERS};
use crate::app::{ApiError, App};
use std::io::Read;
use std::path::Path;
use tiny_http::{Header, Request, Response, StatusCode};

/// The page's Content-Security-Policy in the browser: its own scripts, styles, fonts and images only, no inline
/// script, and requests only to this server.
pub const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

fn query_str<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    url.split_once('?')?
        .1
        .split('&')
        .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

/// Whether a request may be answered: the Host must be this server on loopback, an Origin, when sent, must be the
/// app's own page, and a browser's `Sec-Fetch-Site`, when sent, must say the request comes from that page or from
/// the address bar. Guards against DNS rebinding and against other web pages calling the server, including the GETs
/// (images, scripts) browsers send without an Origin.
pub fn allowed(
    host: Option<&str>,
    origin: Option<&str>,
    fetch_site: Option<&str>,
    port: u16,
) -> bool {
    let own_host = |h: &str| h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}");
    let host_ok = host.is_some_and(own_host);
    let origin_ok = origin.is_none_or(|o| o.strip_prefix("http://").is_some_and(own_host));
    let site_ok = fetch_site.is_none_or(|s| s == "same-origin" || s == "none");
    host_ok && origin_ok && site_ok
}

/// The random token one run of the development server is opened with: the address it prints carries it, the first
/// request trades it for a cookie, and every request after that must carry the cookie.
pub struct Token {
    value: String,
    cookie: String,
}

impl Token {
    /// A new token from the system's random source, for a server on this port (cookies are shared between ports, so
    /// the cookie's name carries it).
    pub fn new(port: u16) -> std::io::Result<Token> {
        let mut bytes = [0u8; 24];
        getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
        let value = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Token {
            value,
            cookie: format!("warden-token-{port}"),
        })
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// The cookie's name.
    pub fn cookie_name(&self) -> &str {
        &self.cookie
    }

    /// Compares in constant time, so the answer's timing does not give the token away.
    fn matches(&self, given: &str) -> bool {
        let (a, b) = (self.value.as_bytes(), given.as_bytes());
        a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }

    /// Whether a `Cookie` header carries this run's token.
    pub fn in_cookie(&self, cookie: Option<&str>) -> bool {
        cookie.is_some_and(|c| {
            c.split(';').any(|kv| {
                kv.trim()
                    .split_once('=')
                    .is_some_and(|(k, v)| k == self.cookie && self.matches(v))
            })
        })
    }

    /// Whether a URL's query carries this run's token (`?token=`).
    pub fn in_query(&self, url: &str) -> bool {
        query_str(url, "token").is_some_and(|t| self.matches(t))
    }

    fn set_cookie(&self) -> String {
        format!(
            "{}={}; Path=/; HttpOnly; SameSite=Strict",
            self.cookie, self.value
        )
    }
}

fn header<'a>(req: &'a Request, name: &'static str) -> Option<&'a str> {
    req.headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}

fn raw_header(name: &str, value: &str) -> Option<Header> {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).ok()
}

fn respond(req: Request, answer: Answer, extra: &[(&str, &str)]) {
    let mut response = Response::from_data(answer.body).with_status_code(StatusCode(answer.status));
    let headers = [("Content-Type", answer.content_type)]
        .into_iter()
        .chain(SECURITY_HEADERS)
        .chain([("Content-Security-Policy", CSP)])
        .chain(extra.iter().copied());
    for h in headers.filter_map(|(k, v)| raw_header(k, v)) {
        response.add_header(h);
    }
    let _ = req.respond(response);
}

fn refuse(req: Request, message: &str) {
    respond(req, Answer::error(ApiError(403, message.into())), &[]);
}

/// The URL without its `token` parameter, where the page goes once the token is in a cookie.
fn without_token(url: &str) -> String {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let rest: Vec<&str> = query
        .split('&')
        .filter(|kv| !kv.is_empty() && !kv.starts_with("token="))
        .collect();
    if rest.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{}", rest.join("&"))
    }
}

/// Answers one request.
pub fn handle(app: &App, mut req: Request, port: u16, token: &Token, ui_dir: Option<&Path>) {
    if !allowed(
        header(&req, "Host"),
        header(&req, "Origin"),
        header(&req, "Sec-Fetch-Site"),
        port,
    ) {
        refuse(
            req,
            "Requests are accepted only from this computer and Warden's own page.",
        );
        return;
    }
    let url = req.url().to_owned();
    let path = api::path_of(&url).to_owned();
    let method = req.method().as_str().to_ascii_uppercase();
    // The address the server printed (also when opened again): the token goes into a cookie and out of the address
    // bar.
    if method == "GET" && !path.starts_with("/api/") && token.in_query(&url) {
        let location = without_token(&url);
        let cookie = token.set_cookie();
        let answer = Answer {
            status: 303,
            content_type: "text/plain; charset=utf-8",
            body: Vec::new(),
        };
        respond(
            req,
            answer,
            &[("Location", &location), ("Set-Cookie", &cookie)],
        );
        return;
    }
    if !token.in_cookie(header(&req, "Cookie")) {
        refuse(
            req,
            "Open Warden with the address warden-server printed when it started (it carries this run's token).",
        );
        return;
    }
    let mut body = Vec::new();
    if req
        .as_reader()
        .take(MAX_BODY as u64 + 1)
        .read_to_end(&mut body)
        .is_err()
    {
        respond(
            req,
            Answer::error(ApiError::bad("The request could not be read.")),
            &[],
        );
        return;
    }
    // The long poll holds this worker; the state is written straight out once it answers.
    let answer = api::answer(app, ui_dir, &method, &url, &body);
    respond(req, answer, &[]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use warden_rules::net::{
        assert_fetch_site_checked, assert_host_header_checked, assert_origin_checked,
        assert_token_checked,
    };

    #[test]
    fn the_guard_checks_host_origin_and_fetch_site() {
        assert_host_header_checked(4777, |h| allowed(h, None, None, 4777));
        assert_origin_checked(4777, &[], |o| {
            allowed(Some("127.0.0.1:4777"), o, None, 4777)
        });
        assert_fetch_site_checked(|s| allowed(Some("127.0.0.1:4777"), None, s, 4777));
    }

    #[test]
    fn the_token_is_random_and_checked_whole() {
        let token = Token::new(4777).expect("token");
        assert_eq!(token.value().len(), 48);
        assert_ne!(token.value(), Token::new(4777).expect("token").value());
        assert_eq!(token.cookie_name(), "warden-token-4777");
        assert_token_checked(token.cookie_name(), token.value(), |c| token.in_cookie(c));
        assert!(token.in_query(&format!("/?a=1&token={}", token.value())));
        assert!(!token.in_query("/?token="));
        assert!(!token.in_query(&format!("/?token={}0", token.value())));
    }

    #[test]
    fn the_token_leaves_the_address() {
        assert_eq!(without_token("/?token=abc"), "/");
        assert_eq!(without_token("/x?a=1&token=abc&b=2"), "/x?a=1&b=2");
        assert_eq!(without_token("/"), "/");
    }

    #[test]
    fn the_browser_policy_is_strict() {
        warden_rules::csp::assert_strict(CSP, &[]);
    }
}
