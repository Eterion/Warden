//! Rule: outbound calls go only to Anthropic's usage and sign-in servers.

/// The usage endpoint's host.
pub const USAGE_ENDPOINT_HOST: &str = "api.anthropic.com";
pub const SIGN_IN_HOST: &str = "platform.claude.com";

/// Panics unless `allowed` holds exactly the usage and sign-in hosts.
pub fn assert_only_anthropic_hosts(allowed: &[&str]) {
    assert!(
        allowed.len() == 2
            && allowed.contains(&USAGE_ENDPOINT_HOST)
            && allowed.contains(&SIGN_IN_HOST),
        "rule broken: allowed outbound hosts are {allowed:?}, expected only the usage and sign-in servers"
    );
}

/// The host of an `https://` URL, or `None` for any other scheme.
///
/// URLs with a backslash or user info are refused outright: parsers disagree on where their host ends
/// (`https://evil.example\@api.anthropic.com/` connects to `evil.example` in a WHATWG parser).
fn https_host(url: &str) -> Option<&str> {
    if url.contains('\\') {
        return None;
    }
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') {
        return None;
    }
    authority.split(':').next()
}

/// Panics if `url` goes anywhere other than an allowed host over HTTPS.
///
/// Later steps call this for every URL the HTTP client is asked to fetch (for example from a recording test client).
pub fn assert_request_allowed(url: &str, allowed: &[&str]) {
    assert!(
        https_host(url).is_some_and(|h| allowed.contains(&h)),
        "rule broken: request to {url} is not an HTTPS call to an allowed host"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_the_two_anthropic_hosts_pass() {
        assert_only_anthropic_hosts(&[USAGE_ENDPOINT_HOST, SIGN_IN_HOST]);
    }

    #[test]
    #[should_panic(expected = "rule broken")]
    fn extra_host_is_caught() {
        assert_only_anthropic_hosts(&[USAGE_ENDPOINT_HOST, SIGN_IN_HOST, "telemetry.example.com"]);
    }

    #[test]
    #[should_panic(expected = "rule broken")]
    fn missing_sign_in_host_is_caught() {
        assert_only_anthropic_hosts(&[USAGE_ENDPOINT_HOST]);
    }

    #[test]
    fn sign_in_request_passes() {
        assert_request_allowed(
            "https://platform.claude.com/v1/oauth/token",
            &[USAGE_ENDPOINT_HOST, SIGN_IN_HOST],
        );
    }

    #[test]
    fn usage_request_passes() {
        assert_request_allowed(
            "https://api.anthropic.com/api/oauth/usage",
            &[USAGE_ENDPOINT_HOST],
        );
    }

    #[test]
    #[should_panic(expected = "rule broken")]
    fn other_host_is_caught() {
        assert_request_allowed("https://example.com/collect", &[USAGE_ENDPOINT_HOST]);
    }

    #[test]
    #[should_panic(expected = "rule broken")]
    fn userinfo_trick_is_caught() {
        assert_request_allowed(
            "https://api.anthropic.com@example.com/",
            &[USAGE_ENDPOINT_HOST],
        );
    }

    #[test]
    #[should_panic(expected = "rule broken")]
    fn backslash_trick_is_caught() {
        assert_request_allowed(
            "https://evil.example\\@api.anthropic.com/",
            &[USAGE_ENDPOINT_HOST],
        );
    }

    #[test]
    #[should_panic(expected = "rule broken")]
    fn plain_http_is_caught() {
        assert_request_allowed(
            "http://api.anthropic.com/api/oauth/usage",
            &[USAGE_ENDPOINT_HOST],
        );
    }
}
