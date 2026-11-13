//! A login token held only for the length of one call: never printed, wiped from memory when dropped.

use serde::Deserialize;
use std::fmt;
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(transparent)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    /// The token itself, to put in the one request header that needs it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Secret(Zeroizing::new(s))
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(hidden)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_value() {
        let s = Secret::from("sk-ant-oat01-abcdef".to_owned());
        assert_eq!(format!("{s:?}"), "Secret(hidden)");
        assert_eq!(s.expose(), "sk-ant-oat01-abcdef");
    }
}
