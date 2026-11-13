//! Rule: outbound calls go only to Anthropic's usage and sign-in servers, and to GitHub for the installed app's
//! updates (the release manifest and installer, and the asset hosts GitHub sends the download on to).

/// The usage endpoint's host.
pub const USAGE_ENDPOINT_HOST: &str = "api.anthropic.com";
pub const SIGN_IN_HOST: &str = "platform.claude.com";
/// Where the installed app's updates come from.
pub const UPDATE_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// Panics unless `allowed` holds exactly the usage and sign-in hosts and the update hosts, each once.
pub fn assert_only_known_hosts(allowed: &[&str]) {
    let mut expected = vec![USAGE_ENDPOINT_HOST, SIGN_IN_HOST];
    expected.extend(UPDATE_HOSTS);
    expected.sort_unstable();
    let mut got = allowed.to_vec();
    got.sort_unstable();
    assert!(
        got == expected,
        "rule broken: allowed outbound hosts are {allowed:?}, expected only the usage and sign-in servers and the update hosts {UPDATE_HOSTS:?}"
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

    fn known() -> Vec<&'static str> {
        let mut all = vec![SIGN_IN_HOST];
        all.extend(UPDATE_HOSTS.iter().rev());
        all.push(USAGE_ENDPOINT_HOST);
        all
    }

    #[test]
    fn exactly_the_known_hosts_pass_in_any_order() {
        assert_only_known_hosts(&known());
    }

    #[test]
    #[should_panic(expected = "rule broken")]
    fn extra_host_is_caught() {
        let mut hosts = known();
        hosts.push("telemetry.example.com");
        assert_only_known_hosts(&hosts);
    }

    #[test]
    #[should_panic(expected = "rule broken")]
    fn missing_sign_in_host_is_caught() {
        assert_only_known_hosts(&[USAGE_ENDPOINT_HOST]);
    }

    #[test]
    #[should_panic(expected = "rule broken")]
    fn a_host_swapped_for_another_is_caught() {
        let mut hosts = known();
        hosts[1] = "gist.github.com";
        assert_only_known_hosts(&hosts);
    }

    #[test]
    fn the_update_manifest_request_passes() {
        assert_request_allowed(
            "https://github.com/Eterion/Warden/releases/latest/download/latest.json",
            &known(),
        );
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
