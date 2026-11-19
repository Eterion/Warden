//! Rule: the demo server listens only on 127.0.0.1 and checks the Host header, the Origin, `Sec-Fetch-Site`
//! and its per-run token.

use std::net::{Ipv4Addr, SocketAddr};

/// Panics unless the server's listening address is 127.0.0.1 (not `0.0.0.0`, not `::`, not a LAN address).
pub fn assert_loopback_v4(addr: SocketAddr) {
    assert!(
        addr.ip() == Ipv4Addr::LOCALHOST,
        "rule broken: the server listens on {addr}, not 127.0.0.1"
    );
}

/// Host header values a request may carry, and whether the server must accept it.
///
/// `port` is the port the server listens on. `None` means no Host header at all.
pub fn host_header_cases(port: u16) -> Vec<(Option<String>, bool)> {
    let p = port;
    vec![
        (Some(format!("127.0.0.1:{p}")), true),
        (Some(format!("localhost:{p}")), true),
        (None, false),
        (Some(String::new()), false),
        (Some("example.com".into()), false),
        (Some(format!("example.com:{p}")), false),
        (Some(format!("127.0.0.1.example.com:{p}")), false),
        (Some(format!("localhost.example.com:{p}")), false),
        (Some(format!("127.0.0.1:{p}@example.com")), false),
        (Some(format!("127.0.0.1:{}", p.wrapping_add(1))), false),
        (Some(format!("192.168.1.10:{p}")), false),
        (Some(format!("[::1]:{p}")), false),
    ]
}

/// Runs every Host header case through `accepts`, which sends a request with that header (or none) to the server
/// and returns whether the server answered it rather than refusing it. Panics on the first wrong answer.
pub fn assert_host_header_checked(port: u16, mut accepts: impl FnMut(Option<&str>) -> bool) {
    for (host, expected) in host_header_cases(port) {
        let got = accepts(host.as_deref());
        assert_eq!(
            got,
            expected,
            "rule broken: Host header {host:?} was {} but must be {}",
            verdict(got),
            verdict(expected)
        );
    }
}

/// Origin header values a request may carry, and whether the server must accept it.
///
/// A web page on any site can send requests to `http://127.0.0.1:<port>` with a valid Host header, so the server
/// also refuses requests whose Origin is not the app's own page. `None` (no Origin) is what `curl` from the status
/// hook sends, and is accepted. `own_origins` are the app's pages besides `http://127.0.0.1:<port>` and
/// `http://localhost:<port>` (for example the Tauri window's origin, from step 4.4).
pub fn origin_header_cases(port: u16, own_origins: &[&str]) -> Vec<(Option<String>, bool)> {
    let p = port;
    let mut cases = vec![
        (None, true),
        (Some(format!("http://127.0.0.1:{p}")), true),
        (Some(format!("http://localhost:{p}")), true),
        (Some("null".into()), false),
        (Some("https://example.com".into()), false),
        (
            Some(format!("http://127.0.0.1:{}", p.wrapping_add(1))),
            false,
        ),
        (Some(format!("http://127.0.0.1.example.com:{p}")), false),
        (Some(format!("https://127.0.0.1:{p}")), false),
    ];
    cases.extend(own_origins.iter().map(|o| (Some((*o).to_owned()), true)));
    cases
}

/// Runs every Origin case through `accepts`, like [`assert_host_header_checked`].
pub fn assert_origin_checked(
    port: u16,
    own_origins: &[&str],
    mut accepts: impl FnMut(Option<&str>) -> bool,
) {
    for (origin, expected) in origin_header_cases(port, own_origins) {
        let got = accepts(origin.as_deref());
        assert_eq!(
            got,
            expected,
            "rule broken: Origin {origin:?} was {} but must be {}",
            verdict(got),
            verdict(expected)
        );
    }
}

/// `Sec-Fetch-Site` values a request without an Origin may carry, and whether the server must accept it.
///
/// Browsers send no Origin on a cross-site GET from an image, script or link, so a web page could still reach the
/// server's GET routes (and hold the long poll's workers) without one. They do send `Sec-Fetch-Site`: only the app's
/// own page (`same-origin`) and an address the user typed in (`none`) may be answered; another site, or a page on
/// another port of this computer (`same-site`), may not. `None` (no header) is what `curl` sends.
pub fn fetch_site_cases() -> Vec<(Option<&'static str>, bool)> {
    vec![
        (None, true),
        (Some("same-origin"), true),
        (Some("none"), true),
        (Some("cross-site"), false),
        (Some("same-site"), false),
        (Some("CROSS-SITE"), false),
        (Some(""), false),
    ]
}

/// Runs every `Sec-Fetch-Site` case through `accepts`, like [`assert_host_header_checked`].
pub fn assert_fetch_site_checked(mut accepts: impl FnMut(Option<&str>) -> bool) {
    for (site, expected) in fetch_site_cases() {
        let got = accepts(site);
        assert_eq!(
            got,
            expected,
            "rule broken: Sec-Fetch-Site {site:?} was {} but must be {}",
            verdict(got),
            verdict(expected)
        );
    }
}

/// `Cookie` header values a request may carry, and whether the server must accept it: only the run's own token, in
/// the cookie named for its port. `None` means no `Cookie` header.
pub fn token_cases(cookie_name: &str, token: &str) -> Vec<(Option<String>, bool)> {
    let mut wrong = token.to_owned();
    let last = if wrong.pop() == Some('0') { '1' } else { '0' };
    wrong.push(last);
    let shorter = &token[..token.len().saturating_sub(1)];
    vec![
        (Some(format!("{cookie_name}={token}")), true),
        (
            Some(format!("theme=dark; {cookie_name}={token}; other=1")),
            true,
        ),
        (None, false),
        (Some(String::new()), false),
        (Some(format!("{cookie_name}=")), false),
        (Some(format!("{cookie_name}={wrong}")), false),
        (Some(format!("{cookie_name}={shorter}")), false),
        (Some(format!("{cookie_name}={token}0")), false),
        (Some(format!("other-{cookie_name}={token}")), false),
        (Some(format!("warden-token-1={token}")), false),
        (Some(format!("token={token}")), false),
    ]
}

/// Runs every `Cookie` case through `accepts`, like [`assert_host_header_checked`].
pub fn assert_token_checked(
    cookie_name: &str,
    token: &str,
    mut accepts: impl FnMut(Option<&str>) -> bool,
) {
    for (cookie, expected) in token_cases(cookie_name, token) {
        let got = accepts(cookie.as_deref());
        assert_eq!(
            got,
            expected,
            "rule broken: Cookie {cookie:?} was {} but must be {}",
            verdict(got),
            verdict(expected)
        );
    }
}

fn verdict(accepted: bool) -> &'static str {
    if accepted { "accepted" } else { "refused" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    /// A correct check, as the server should implement it.
    fn strict(port: u16) -> impl FnMut(Option<&str>) -> bool {
        move |host| {
            host.is_some_and(|h| {
                h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}")
            })
        }
    }

    #[test]
    fn loopback_passes() {
        assert_loopback_v4(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4777));
    }

    #[test]
    #[should_panic(expected = "rule broken")]
    fn all_interfaces_is_caught() {
        assert_loopback_v4(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 4777));
    }

    #[test]
    fn strict_host_check_passes() {
        assert_host_header_checked(4777, strict(4777));
    }

    #[test]
    #[should_panic(expected = "rule broken: Host header")]
    fn missing_host_check_is_caught() {
        assert_host_header_checked(4777, |_| true);
    }

    #[test]
    fn strict_origin_check_passes() {
        assert_origin_checked(4777, &[], |origin| {
            origin.is_none_or(|o| o == "http://127.0.0.1:4777" || o == "http://localhost:4777")
        });
    }

    #[test]
    #[should_panic(expected = "rule broken: Origin")]
    fn missing_origin_check_is_caught() {
        assert_origin_checked(4777, &[], |_| true);
    }

    #[test]
    fn strict_fetch_site_check_passes() {
        assert_fetch_site_checked(|site| site.is_none_or(|s| s == "same-origin" || s == "none"));
    }

    #[test]
    #[should_panic(expected = "rule broken: Sec-Fetch-Site")]
    fn missing_fetch_site_check_is_caught() {
        assert_fetch_site_checked(|_| true);
    }

    fn strict_token(c: Option<&str>) -> bool {
        c.is_some_and(|c| {
            c.split(';')
                .any(|kv| kv.trim() == "warden-token-4777=abc123")
        })
    }

    #[test]
    fn strict_token_check_passes() {
        assert_token_checked("warden-token-4777", "abc123", strict_token);
    }

    #[test]
    #[should_panic(expected = "rule broken: Cookie")]
    fn missing_token_check_is_caught() {
        assert_token_checked("warden-token-4777", "abc123", |_| true);
    }

    #[test]
    #[should_panic(expected = "rule broken: Cookie")]
    fn token_prefix_match_is_caught() {
        assert_token_checked("warden-token-4777", "abc123", |c| {
            c.is_some_and(|c| c.contains("warden-token-4777=abc12"))
        });
    }

    #[test]
    #[should_panic(expected = "rule broken: Host header")]
    fn prefix_match_is_caught() {
        assert_host_header_checked(4777, |h| h.is_some_and(|h| h.starts_with("127.0.0.1")));
    }
}
