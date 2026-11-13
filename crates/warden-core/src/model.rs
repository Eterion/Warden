//! The data shared by the server, the saved files and (from step 4.2) `warden route`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Provider::Claude => "Claude",
            Provider::Codex => "Codex",
        }
    }

    /// The environment variable that points the CLI at an account folder.
    pub fn folder_var(self) -> &'static str {
        match self {
            Provider::Claude => "CLAUDE_CONFIG_DIR",
            Provider::Codex => "CODEX_HOME",
        }
    }

    /// The desktop app's default folder name in the home folder.
    pub fn default_folder_name(self) -> &'static str {
        match self {
            Provider::Claude => ".claude",
            Provider::Codex => ".codex",
        }
    }
}

/// Who an account folder is signed in as. Read only to tell accounts apart; never holds a token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub email: String,
    /// Claude's organisation name, or Codex's ChatGPT plan, for people.
    pub organization: Option<String>,
    /// The organisation's (or workspace's) id, to compare by when the name is shared or changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
}

impl Identity {
    /// The same person in the same organisation. Emails compare without case; organisations by id when both
    /// sides have one, otherwise by name.
    pub fn same_as(&self, other: &Identity) -> bool {
        if !self.email.eq_ignore_ascii_case(&other.email) {
            return false;
        }
        match (&self.organization_id, &other.organization_id) {
            (Some(a), Some(b)) => a == b,
            _ => self.organization == other.organization,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limit {
    /// 0 to 100.
    pub used_percent: f64,
    /// Milliseconds since the epoch.
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelLimit {
    pub model: String,
    pub limit: Limit,
}

/// Pay-as-you-go usage beyond the plan's limits (Claude's "extra usage").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extra {
    pub enabled: bool,
    pub used_credits: Option<f64>,
    pub monthly_limit: Option<f64>,
    pub used_percent: Option<f64>,
}

/// One usage reading of one account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// When the reading was taken (for Codex, the time of the log event it came from).
    pub read_at: i64,
    /// `None`: the account has no 5-hour limit.
    pub five_hour: Option<Limit>,
    pub weekly: Option<Limit>,
    #[serde(default)]
    pub per_model_weekly: Vec<ModelLimit>,
    pub extra: Option<Extra>,
}

/// Why the latest attempt to read usage gave no reading. The last good reading is kept and shown with its age.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UsageError {
    /// No login in the folder.
    SignedOut,
    /// The stored login has expired or was refused; Warden waits for Claude to renew it and never renews it itself.
    LoginExpired {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<u16>,
    },
    /// The folder is signed in as someone else; readings are kept apart until the change is confirmed.
    IdentityChanged,
    /// Codex: no session log with limits yet.
    NoLogs,
    /// The endpoint could not be reached, refused the request or answered in a shape Warden does not know.
    /// HTTP refusals other than 401 keep their status in the safe message; never the answer's body.
    Failed { message: String },
}

/// The folder is signed in as someone other than the account's remembered identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityChange {
    /// `None`: signed out.
    pub now: Option<Identity>,
    /// The new identity belongs to this other known account.
    pub known_account_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(email: &str, org: Option<&str>, org_id: Option<&str>) -> Identity {
        Identity {
            email: email.into(),
            organization: org.map(Into::into),
            organization_id: org_id.map(Into::into),
        }
    }

    #[test]
    fn identity_comparison() {
        let a = id("Ann@Example.com", Some("Acme"), Some("org-1"));
        assert!(a.same_as(&id("ann@example.com", Some("Acme"), Some("org-1"))));
        assert!(a.same_as(&id("ann@example.com", Some("Acme renamed"), Some("org-1"))));
        assert!(!a.same_as(&id("ann@example.com", Some("Acme"), Some("org-2"))));
        assert!(!a.same_as(&id("bob@example.com", Some("Acme"), Some("org-1"))));
        assert!(a.same_as(&id("ann@example.com", Some("Acme"), None)));
        assert!(!a.same_as(&id("ann@example.com", None, None)));
    }

    #[test]
    fn usage_error_serialises_with_its_kind() {
        let json =
            serde_json::to_string(&UsageError::LoginExpired { status: None }).expect("serialises");
        assert_eq!(json, r#"{"kind":"loginExpired"}"#);
        assert_eq!(
            serde_json::from_str::<UsageError>(&json).expect("old saved error"),
            UsageError::LoginExpired { status: None }
        );
        let refused = UsageError::LoginExpired { status: Some(401) };
        let json = serde_json::to_string(&refused).expect("serialises");
        assert_eq!(json, r#"{"kind":"loginExpired","status":401}"#);
        assert_eq!(
            serde_json::from_str::<UsageError>(&json).expect("saved HTTP error"),
            refused
        );
        let json = serde_json::to_string(&UsageError::Failed {
            message: "HTTP 500".into(),
        })
        .expect("serialises");
        assert_eq!(json, r#"{"kind":"failed","message":"HTTP 500"}"#);
    }
}
