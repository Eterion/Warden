//! Rule: login tokens never appear in logs or API responses.
//!
//! Plant [`canary_token`] where a login token would be read (a fake credentials file), drive the code, then pass
//! everything it wrote (log output, response bodies, saved files) to [`assert_no_token`].

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

/// The prefix every Claude OAuth access token starts with; it identifies nothing on its own.
const PREFIX: &str = "sk-ant-oat01-";
const REFRESH_PREFIX: &str = "sk-ant-ort01-";

/// Any run of this many characters from the token's body counts as a leak.
const WINDOW: usize = 12;

/// A fake login token with a random body, so it never matches real data and any piece of it is recognisable.
///
/// It has the shape of a Claude OAuth access token, so code that recognises tokens by shape still treats it as one.
pub fn canary_token(tag: &str) -> String {
    let random = |salt: u64| {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(salt);
        h.finish()
    };
    format!(
        "{PREFIX}{tag}-{:016x}{:016x}{:016x}",
        random(1),
        random(2),
        random(3)
    )
}

/// A fake rotating refresh token, distinct from the access token canary.
pub fn canary_refresh_token(tag: &str) -> String {
    canary_token(tag).replacen(PREFIX, REFRESH_PREFIX, 1)
}

/// Panics if `haystack` contains any [`WINDOW`]-character run of the token's body (a token cut short, for
/// example truncated or masked in a log line, still leaks).
///
/// `place` names where the text came from ("server log", "GET /api/accounts"), for the failure message.
pub fn assert_no_token(place: &str, haystack: &str, token: &str) {
    let body: Vec<char> = token
        .strip_prefix(PREFIX)
        .or_else(|| token.strip_prefix(REFRESH_PREFIX))
        .unwrap_or(token)
        .chars()
        .collect();
    let windows: Vec<String> = if body.len() <= WINDOW {
        vec![body.iter().collect()]
    } else {
        body.windows(WINDOW).map(|w| w.iter().collect()).collect()
    };
    for fragment in windows {
        assert!(
            !haystack.contains(&fragment),
            "rule broken: a login token (or part of it) appears in {place}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_passes() {
        let token = canary_token("a");
        assert_no_token(
            "log",
            "usage read for account Main, token sk-ant-oat01-…",
            &token,
        );
    }

    #[test]
    fn the_refresh_prefix_alone_identifies_no_login() {
        let token = canary_refresh_token("a");
        assert_no_token("log", "refresh token sk-ant-ort01-…", &token);
    }

    #[test]
    #[should_panic(expected = "rule broken: a login token")]
    fn whole_token_is_caught() {
        let token = canary_token("a");
        assert_no_token("log", &format!("Authorization: Bearer {token}"), &token);
    }

    #[test]
    #[should_panic(expected = "rule broken: a login token")]
    fn a_refresh_token_piece_is_caught() {
        let token = canary_refresh_token("refresh");
        let middle: String = token.chars().skip(30).take(16).collect();
        assert_no_token("saved readings", &middle, &token);
    }

    #[test]
    #[should_panic(expected = "rule broken: a login token")]
    fn middle_piece_is_caught() {
        let token = canary_token("a");
        let middle: String = token.chars().skip(30).take(16).collect();
        assert_no_token("log", &format!("token=…{middle}…"), &token);
    }
}
