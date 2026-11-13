//! Account identity checks: is the folder still signed in as the account Warden remembers?

use crate::model::{Identity, IdentityChange, Provider};
use crate::store::{AccountConfig, Settings};

/// What a fresh identity read means for an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// Same identity as remembered (or nothing to compare).
    Unchanged,
    /// No identity remembered yet: remember this one.
    FirstSeen(Identity),
    /// Signed in as someone else.
    Changed(IdentityChange),
}

// Codex's display label is its plan, never part of the sign-in identity, even without a workspace claim.
fn same_sign_in(provider: Provider, known: &Identity, now: &Identity) -> bool {
    match provider {
        Provider::Claude => known.same_as(now),
        Provider::Codex => {
            known.email.eq_ignore_ascii_case(&now.email)
                && match (&known.organization_id, &now.organization_id) {
                    (Some(a), Some(b)) => a == b,
                    _ => true,
                }
        }
    }
}

/// Compares the identity read now with the one the account remembers. Signed out (`now` is `None`) is not a change
/// of identity; usage reading reports it instead.
pub fn check(settings: &Settings, account: &AccountConfig, now: Option<Identity>) -> Check {
    let Some(now) = now else {
        return Check::Unchanged;
    };
    match &account.identity {
        None => Check::FirstSeen(now),
        Some(known) if same_sign_in(account.provider, known, &now) => Check::Unchanged,
        Some(_) => Check::Changed(IdentityChange {
            known_account_id: settings
                .accounts
                .iter()
                .find(|a| {
                    a.id != account.id
                        && a.provider == account.provider
                        && a.identity
                            .as_ref()
                            .is_some_and(|i| same_sign_in(account.provider, i, &now))
                })
                .map(|a| a.id.clone())
                .or_else(|| {
                    (account.id != crate::store::MAIN_ID
                        && account.provider == Provider::Claude
                        && settings
                            .main
                            .identity
                            .as_ref()
                            .is_some_and(|main| main.same_as(&now)))
                    .then(|| crate::store::MAIN_ID.to_owned())
                }),
            now: Some(now),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ident(email: &str) -> Identity {
        Identity {
            email: email.into(),
            organization: Some("Acme".into()),
            organization_id: None,
        }
    }

    fn account(id: &str, who: Option<&str>) -> AccountConfig {
        AccountConfig {
            id: id.into(),
            label: id.into(),
            // This fixture compares organization names, which are Claude identity metadata.
            provider: Provider::Claude,
            folder: PathBuf::from(id),
            enabled: true,
            identity: who.map(ident),
        }
    }

    #[test]
    fn an_older_codex_identity_can_gain_a_workspace_id() {
        let mut a = account("codex", Some("ann@example.com"));
        a.provider = Provider::Codex;
        let s = Settings {
            accounts: vec![a.clone()],
            ..Settings::default()
        };
        let mut now = a.identity.clone().expect("identity");
        assert_eq!(now.organization_id, None);
        now.organization_id = Some("workspace-1".into());
        assert_eq!(check(&s, &a, Some(now)), Check::Unchanged);
    }

    #[test]
    fn a_codex_plan_is_not_identity_even_without_a_workspace_claim() {
        let mut a = account("codex", Some("ann@example.com"));
        a.provider = Provider::Codex;
        let s = Settings {
            accounts: vec![a.clone()],
            ..Settings::default()
        };
        let mut now = a.identity.clone().expect("identity");
        now.organization = Some("Business".into());
        assert_eq!(check(&s, &a, Some(now.clone())), Check::Unchanged);
        now.organization_id = Some("another-workspace".into());
        assert_eq!(check(&s, &a, Some(now.clone())), Check::Unchanged);
        a.identity.as_mut().expect("identity").organization_id = Some("saved-workspace".into());
        assert!(matches!(
            check(&s, &a, Some(now.clone())),
            Check::Changed(_)
        ));
        now.organization_id = None;
        assert_eq!(check(&s, &a, Some(now)), Check::Unchanged);
    }

    #[test]
    fn a_change_names_another_account_only_for_the_same_person_in_the_same_organisation() {
        let mut other_org = account("other-org", Some("me@home.example"));
        other_org
            .identity
            .as_mut()
            .expect("an identity")
            .organization = Some("Elsewhere".into());
        let s = Settings {
            accounts: vec![
                account("work", Some("work@acme.example")),
                other_org,
                account("not-yet", None),
                account("personal", Some("ME@home.example")),
            ],
            ..Settings::default()
        };
        // The other-organisation account and the one with nothing remembered are passed over.
        assert_eq!(
            check(&s, &s.accounts[0], Some(ident("me@home.example"))),
            Check::Changed(IdentityChange {
                now: Some(ident("me@home.example")),
                known_account_id: Some("personal".into())
            })
        );
        // Signed in under another organisation is a change, even with the same email.
        let mut moved = ident("work@acme.example");
        moved.organization = Some("Elsewhere".into());
        assert_eq!(
            check(&s, &s.accounts[0], Some(moved.clone())),
            Check::Changed(IdentityChange {
                now: Some(moved),
                known_account_id: None
            })
        );
        // An account with nothing remembered takes what it reads, even if another account has it.
        assert_eq!(
            check(&s, &s.accounts[2], Some(ident("me@home.example"))),
            Check::FirstSeen(ident("me@home.example"))
        );
    }

    #[test]
    fn checks_identity() {
        let s = Settings {
            accounts: vec![
                account("work", Some("work@acme.example")),
                account("personal", Some("me@home.example")),
                account("new", None),
            ],
            ..Settings::default()
        };
        assert_eq!(
            check(&s, &s.accounts[0], Some(ident("WORK@acme.example"))),
            Check::Unchanged
        );
        assert_eq!(check(&s, &s.accounts[0], None), Check::Unchanged);
        assert_eq!(
            check(&s, &s.accounts[2], Some(ident("x@y.example"))),
            Check::FirstSeen(ident("x@y.example"))
        );
        assert_eq!(
            check(&s, &s.accounts[0], Some(ident("me@home.example"))),
            Check::Changed(IdentityChange {
                now: Some(ident("me@home.example")),
                known_account_id: Some("personal".into())
            })
        );
        assert_eq!(
            check(&s, &s.accounts[0], Some(ident("someone@else.example"))),
            Check::Changed(IdentityChange {
                now: Some(ident("someone@else.example")),
                known_account_id: None
            })
        );
    }
}
