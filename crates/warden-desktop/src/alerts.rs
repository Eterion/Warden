//! Which identity changes to notify about: each account once, when its folder starts being signed in as someone
//! else. The same words as the browser version's notifications.

use std::collections::HashSet;
use warden_core::store::Readings;
use warden_server::app::IdentityAlert;

/// A Windows notification's title and text.
#[derive(Debug, PartialEq, Eq)]
pub struct Message {
    pub title: String,
    pub body: String,
}

/// Remembers which accounts were already flagged, so each change is notified once.
#[derive(Debug, Default)]
pub struct Alerts {
    flagged: HashSet<String>,
}

impl Alerts {
    /// Starts from the accounts flagged in the saved readings, read before the core starts: those were notified in
    /// an earlier run. Anything the core finds once it runs, its first identity checks included, is new.
    pub fn from_saved(saved: &Readings) -> Alerts {
        Alerts {
            flagged: saved
                .accounts
                .iter()
                .filter(|(_, r)| r.identity_change.is_some())
                .map(|(id, _)| id.clone())
                .collect(),
        }
    }

    /// The notifications for accounts flagged since the last call. An account that is confirmed (or signs back in
    /// as itself) and changes again later is notified again.
    pub fn update(&mut self, now: &[IdentityAlert]) -> Vec<Message> {
        let fresh = now
            .iter()
            .filter(|a| !self.flagged.contains(&a.account_id))
            .map(message)
            .collect();
        self.flagged = now.iter().map(|a| a.account_id.clone()).collect();
        fresh
    }
}

fn message(a: &IdentityAlert) -> Message {
    let who = a.now.as_deref().unwrap_or("nobody (signed out)");
    Message {
        title: format!("{} is signed in as someone else", a.label),
        body: format!(
            "Its folder is now signed in as {who}. Warden keeps its usage apart until you confirm in Settings, Accounts."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(id: &str, now: Option<&str>) -> IdentityAlert {
        IdentityAlert {
            account_id: id.into(),
            label: format!("Account {id}"),
            now: now.map(Into::into),
        }
    }

    #[test]
    fn a_change_found_at_start_is_new_unless_it_was_saved() {
        use warden_core::model::IdentityChange;
        use warden_core::store::AccountReading;
        let mut saved = Readings::default();
        saved.accounts.insert(
            "a1".into(),
            AccountReading {
                identity_change: Some(IdentityChange {
                    now: None,
                    known_account_id: None,
                }),
                ..AccountReading::default()
            },
        );
        saved
            .accounts
            .insert("a2".into(), AccountReading::default());
        let mut alerts = Alerts::from_saved(&saved);
        // The core's first check flags a2 before the follower ever looks: still notified.
        let got = alerts.update(&[alert("a1", None), alert("a2", Some("bob@example.com"))]);
        assert_eq!(got.len(), 1);
        assert!(got[0].title.starts_with("Account a2"));
    }

    #[test]
    fn each_change_is_notified_once() {
        let mut alerts = Alerts {
            flagged: ["a1".to_owned()].into(),
        };
        assert_eq!(alerts.update(&[alert("a1", Some("old@example.com"))]), []);

        let got = alerts.update(&[alert("a1", Some("old@example.com")), alert("a2", None)]);
        assert_eq!(
            got,
            [Message {
                title: "Account a2 is signed in as someone else".into(),
                body: "Its folder is now signed in as nobody (signed out). Warden keeps its usage apart until you \
                       confirm in Settings, Accounts."
                    .into(),
            }]
        );
        assert_eq!(alerts.update(&[alert("a2", None)]), []);

        // Confirmed, then changed again: notified again.
        assert_eq!(alerts.update(&[]), []);
        let again = alerts.update(&[alert("a2", Some("bob@example.com"))]);
        assert_eq!(again.len(), 1);
        assert!(again[0].body.contains("bob@example.com"));
    }

    #[test]
    fn nothing_saved_means_every_change_found_is_new_and_named() {
        let mut alerts = Alerts::from_saved(&Readings::default());
        assert_eq!(alerts.update(&[]), []);
        let got = alerts.update(&[
            alert("a1", Some("ann@example.com")),
            alert("a2", Some("bob@example.com")),
        ]);
        assert_eq!(
            got,
            [
                Message {
                    title: "Account a1 is signed in as someone else".into(),
                    body: "Its folder is now signed in as ann@example.com. Warden keeps its usage apart until you \
                           confirm in Settings, Accounts."
                        .into(),
                },
                Message {
                    title: "Account a2 is signed in as someone else".into(),
                    body: "Its folder is now signed in as bob@example.com. Warden keeps its usage apart until you \
                           confirm in Settings, Accounts."
                        .into(),
                },
            ]
        );
    }

    #[test]
    fn a_saved_change_that_is_gone_at_start_is_notified_when_it_comes_back() {
        let mut saved = Readings::default();
        saved.accounts.insert(
            "a1".into(),
            warden_core::store::AccountReading {
                identity_change: Some(warden_core::model::IdentityChange {
                    now: None,
                    known_account_id: None,
                }),
                ..Default::default()
            },
        );
        let mut alerts = Alerts::from_saved(&saved);
        // Confirmed while Warden was closed: the first look finds nothing.
        assert_eq!(alerts.update(&[]), []);
        assert_eq!(alerts.update(&[alert("a1", None)]).len(), 1);
    }
}
