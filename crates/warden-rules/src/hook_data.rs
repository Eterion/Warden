//! Rule: the status hook's prompts and tool inputs are never kept.
//!
//! Put [`canary_text`] in an event's prompt and tool input, send the events, then pass everything Warden gives back
//! or writes (API responses, the log, saved files) to [`assert_not_kept`].

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

/// A made-up prompt or tool input with a random marker, so it never matches real data.
pub fn canary_text(tag: &str) -> String {
    let mut h = RandomState::new().build_hasher();
    h.write(tag.as_bytes());
    format!("private {tag} canary {:016x}", h.finish())
}

/// Panics if `haystack` holds the canary's marker (the random part), whole or cut down to its first 8 digits.
///
/// `place` names where the text came from ("GET /api/state", "server log"), for the failure message.
pub fn assert_not_kept(place: &str, haystack: &str, canary: &str) {
    let marker = canary.rsplit(' ').next().unwrap_or(canary);
    let piece: String = marker.chars().take(8).collect();
    assert!(
        !haystack.contains(&piece),
        "rule broken: a prompt or tool input from the status hook appears in {place}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_passes() {
        let canary = canary_text("prompt");
        assert_not_kept("state", r#"{"doing":"Running commands"}"#, &canary);
    }

    #[test]
    #[should_panic(expected = "rule broken: a prompt or tool input")]
    fn a_kept_prompt_is_caught() {
        let canary = canary_text("prompt");
        assert_not_kept("state", &format!(r#"{{"doing":"{canary}"}}"#), &canary);
    }

    #[test]
    #[should_panic(expected = "rule broken: a prompt or tool input")]
    fn a_shortened_one_is_caught() {
        let canary = canary_text("tool input");
        let marker = canary.rsplit(' ').next().expect("marker");
        assert_not_kept("log", &format!("ran {}…", &marker[..10]), &canary);
    }
}
