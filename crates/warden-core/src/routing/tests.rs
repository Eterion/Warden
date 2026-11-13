use super::*;
use crate::model::{Identity, IdentityChange, Usage};
use ClaudeShell::{Bash, PowerShell};
use std::path::PathBuf;

const NOW: i64 = 1_800_000_000_000;
fn identity(email: &str) -> Identity {
    Identity {
        email: email.into(),
        organization: None,
        organization_id: None,
    }
}
fn account(id: &str, provider: Provider) -> AccountConfig {
    AccountConfig {
        id: id.into(),
        label: id.into(),
        provider,
        folder: PathBuf::from(format!("<home>/{id}")),
        enabled: true,
        identity: Some(identity(&format!("{id}@example.com"))),
    }
}
fn usage(r: &mut Readings, id: &str, five: Option<f64>, week: Option<f64>, reset: i64) {
    r.accounts.entry(id.into()).or_default().usage = Some(Usage {
        read_at: NOW,
        five_hour: five.map(|used_percent| Limit {
            used_percent,
            resets_at: Some(NOW + HOUR_MS),
        }),
        weekly: week.map(|used_percent| Limit {
            used_percent,
            resets_at: Some(reset),
        }),
        per_model_weekly: vec![],
        extra: None,
    });
}
fn setup() -> (Settings, Readings) {
    let mut s = Settings {
        accounts: vec![
            account("second", Provider::Claude),
            account("third", Provider::Codex),
        ],
        ..Settings::default()
    };
    s.main.identity = Some(identity("main@example.com"));
    let mut r = Readings::default();
    r.main_login.sign_in.state = SignInState::SignedIn;
    usage(&mut r, MAIN_ID, Some(10.0), Some(10.0), NOW + WEEK_MS);
    usage(&mut r, "second", Some(10.0), Some(45.0), NOW + 48 * HOUR_MS);
    usage(&mut r, "third", Some(10.0), Some(10.0), NOW + WEEK_MS);
    (s, r)
}
fn target(s: &Settings, r: &Readings) -> Option<String> {
    decide(s, r, NOW).delegated.map(|t| t.account_id)
}

#[test]
fn reserve_line_shrinks_at_start_middle_and_end_of_week() {
    // The hand-off point is the line rounded down, and the same whole number decides.
    for (left, line, hand_off_at) in [
        (WEEK_MS, 75.0, 75),
        (WEEK_MS / 2, 87.5, 87),
        (0, 100.0, 100),
    ] {
        assert_eq!(reserve_now(25, None, Some(NOW + left), NOW), 100.0 - line);
        let (s, mut r) = setup();
        usage(
            &mut r,
            MAIN_ID,
            Some(10.0),
            Some(f64::from(hand_off_at) - 0.01),
            NOW + left,
        );
        let d = decide(&s, &r, NOW);
        assert_eq!(d.hand_off_at, hand_off_at);
        assert!(d.main_works_itself);
        // A reset already passed makes the reading zero, irrespective of its previous share.
        if left > 0 {
            usage(
                &mut r,
                MAIN_ID,
                Some(10.0),
                Some(f64::from(hand_off_at)),
                NOW + left,
            );
            assert!(!decide(&s, &r, NOW).main_works_itself);
        }
    }
    assert_eq!(reserve_now(25, None, Some(NOW + 2 * WEEK_MS), NOW), 25.0);
    assert_eq!(reserve_now(25, None, Some(NOW - 1), NOW), 0.0);
    assert_eq!(reserve_now(25, None, None, NOW), 25.0);
}

/// Monday 2026-11-09 00:00 in local time (UTC+1, Central European winter time), as ms since the epoch.
const MONDAY_LOCAL_MIDNIGHT: i64 = 1_794_178_800_000;
const CET: i64 = 3600;
fn at(day: i64, hour: i64, minute: i64) -> i64 {
    MONDAY_LOCAL_MIDNIGHT + day * DAY_MS + hour * HOUR_MS + minute * MINUTE_MS
}
fn office() -> WorkHours {
    WorkHours::first()
}

#[test]
fn work_hours_shrink_the_reserve_only_while_working() {
    let cet = |_: i64| CET;
    // Reset Friday 05:00; Monday to Friday, 09:00 to 18:00 (45 hours a week).
    let reset = Some(at(4, 5, 0));
    let w = office();
    let reserve = |now| reserve_with(25, Some(&w), reset, now, &cet);
    // Monday 09:00: 36 of 45 work hours left, so 20% (hands off at 80%).
    assert!((reserve(at(0, 9, 0)) - 20.0).abs() < 1e-9);
    assert_eq!(hand_off(reserve(at(0, 9, 0))), 80);
    // Thursday 16:00: 2 hours left, about 1% (hands off at 98%).
    assert!((reserve(at(3, 16, 0)) - 25.0 * 2.0 / 45.0).abs() < 1e-9);
    assert_eq!(hand_off(reserve(at(3, 16, 0))), 98);
    // An evening does not shrink it: Monday 18:00 and 23:00 keep the same reserve.
    assert_eq!(reserve(at(0, 18, 0)), reserve(at(0, 23, 0)));
    // After the last work window before the reset it is gone: hands off at 100%.
    assert_eq!(reserve(at(3, 18, 0)), 0.0);
    assert_eq!(hand_off(reserve(at(3, 18, 0))), 100);
}

#[test]
fn weekends_do_not_shrink_it_and_a_reset_inside_a_window_counts_up_to_the_reset() {
    let cet = |_: i64| CET;
    let w = office();
    // Reset next Tuesday 12:00; Saturday and Sunday keep the same reserve.
    let reset = Some(at(8, 12, 0));
    let sat = reserve_with(25, Some(&w), reset, at(5, 10, 0), &cet);
    let sun = reserve_with(25, Some(&w), reset, at(6, 20, 0), &cet);
    assert_eq!(sat, sun);
    // Monday 9 hours, Tuesday 09:00 to the reset at 12:00: 12 of 45 hours.
    assert!((sat - 25.0 * 12.0 / 45.0).abs() < 1e-9);
    // A reset more than a week away counts as a week: the full reserve.
    assert_eq!(
        reserve_with(25, Some(&w), Some(at(20, 0, 0)), at(0, 0, 0), &cet),
        25.0
    );
    // No reset known: the full reserve.
    assert_eq!(reserve_with(25, Some(&w), None, at(0, 0, 0), &cet), 25.0);
}

#[test]
fn a_daylight_saving_change_inside_the_week_keeps_the_work_windows_in_local_time() {
    // Summer time (UTC+2) until Wednesday 00:00 local, then winter time (UTC+1).
    let change = at(2, 0, 0) - CET * 1000;
    let tz = move |utc: i64| if utc < change { 2 * CET } else { CET };
    let w = WorkHours {
        days: vec![1, 2, 3, 4, 5],
        from: 9 * 60,
        to: 10 * 60,
    };
    // Monday 00:00 local (summer) to Friday 00:00 local (winter): four one-hour windows, whatever the offset.
    let start = at(0, 0, 0) - CET * 1000;
    let end = at(4, 0, 0);
    assert_eq!(work_ms_between(&w, start, end, &tz), 4 * HOUR_MS);
}

/// Central Europe around the clocks going back (Sunday 2026-10-25, 03:00 summer time becomes 02:00 winter time) and
/// forward (Sunday 2026-03-29, 02:00 winter time becomes 03:00 summer time).
fn prague(utc: i64) -> i64 {
    const BACK: i64 = 1_792_890_000_000; // 2026-10-25T01:00:00Z
    const FORWARD: i64 = 1_774_746_000_000; // 2026-03-29T01:00:00Z
    if (FORWARD..BACK).contains(&utc) {
        2 * CET
    } else {
        CET
    }
}

#[test]
fn a_repeated_or_missing_hour_counts_as_the_ui_counts_it() {
    // Sunday 02:00 to 04:00: when the clocks go back, 02:00 is its first occurrence (as JavaScript's Date reads it),
    // so the window holds three real hours; when they go forward, 02:00 never occurs and reads as 03:00 summer
    // time, so it holds one. The UI's preview has the same cases (format.test.ts).
    let w = WorkHours {
        days: vec![7],
        from: 2 * 60,
        to: 4 * 60,
    };
    let back_day_start = 1_792_879_200_000; // 2026-10-24T22:00:00Z, Sunday 00:00 summer time
    assert_eq!(
        work_ms_between(&w, back_day_start, back_day_start + DAY_MS, &prague),
        3 * HOUR_MS
    );
    let forward_day_start = 1_774_738_800_000; // 2026-03-28T23:00:00Z, Sunday 00:00 winter time
    assert_eq!(
        work_ms_between(&w, forward_day_start, forward_day_start + DAY_MS, &prague),
        HOUR_MS
    );
}

#[test]
fn work_hours_decide_through_the_same_hand_off_point() {
    let (mut s, mut r) = setup();
    s.work_hours = Some(office());
    // Without a fixed clock the exact point varies, but what decides is always the shown point.
    usage(&mut r, MAIN_ID, Some(10.0), Some(50.0), NOW + 3 * DAY_MS);
    let d = decide(&s, &r, NOW);
    assert!(d.hand_off_at >= 75 && d.hand_off_at <= 100);
    assert_eq!(d.main_works_itself, 50.0 < f64::from(d.hand_off_at));
    assert!(
        d.main_reason
            .contains(&format!("keeps {}%", 100 - d.hand_off_at))
    );
}

#[test]
fn each_switched_on_account_is_listed_with_why_it_is_skipped() {
    let (mut s, mut r) = setup();
    let mut more = vec![
        account("near", Provider::Claude),
        account("full", Provider::Claude),
        account("out", Provider::Claude),
        account("moved", Provider::Claude),
        account("twin", Provider::Claude),
        account("off", Provider::Claude),
    ];
    more[4].identity = Some(identity("main@example.com"));
    more[5].enabled = false;
    s.accounts.extend(more);
    usage(&mut r, "near", Some(95.0), Some(10.0), NOW + WEEK_MS);
    usage(&mut r, "full", Some(10.0), Some(100.0), NOW + WEEK_MS);
    r.accounts.entry("out".into()).or_default().error = Some(UsageError::SignedOut);
    r.accounts
        .entry("moved".into())
        .or_default()
        .identity_change = Some(IdentityChange {
        now: Some(identity("else@example.com")),
        known_account_id: None,
    });
    let listed: Vec<_> = decide(&s, &r, NOW)
        .accounts
        .into_iter()
        .map(|l| (l.account_id, l.skip))
        .collect();
    assert_eq!(
        listed,
        vec![
            ("second".into(), None),
            ("third".into(), None),
            ("near".into(), Some(Skip::NearFiveHourLimit)),
            ("full".into(), Some(Skip::WeekUsedUp)),
            ("out".into(), Some(Skip::SignedOut)),
            ("moved".into(), Some(Skip::SignInChanged)),
            ("twin".into(), Some(Skip::SameAsMain)),
        ]
    );
    let text = guidance(&s, &r, Some(MAIN_ID), Bash, None, NOW).text;
    for line in [
        "\n- second (Warden's pick): CLAUDE_CONFIG_DIR=",
        "\n- third: CODEX_HOME=",
        "\n- near (skipped: near its 5-hour limit): CLAUDE_CONFIG_DIR=",
        "\n- full (skipped: its week is used up): CLAUDE_CONFIG_DIR=",
        // Someone else's login, or none: listed with why, and no command.
        "\n- out (skipped: signed out)\n",
        "\n- moved (skipped: signed in as someone else, waiting for the user to confirm)\n",
    ] {
        assert!(text.contains(line), "{line:?} in {text}");
    }
    assert!(
        text.ends_with("\n- twin (skipped: same account as main)\n"),
        "{text}"
    );
    assert!(!text.contains("- off"), "{text}");
    // A listed command comes with the sentence about running it in the background, once.
    assert_eq!(text.matches(BACKGROUND_RUNS).count(), 1, "{text}");
    // Only accounts skipped for their limits keep a command: no pick, and still the sentence.
    for id in ["second", "third"] {
        s.accounts.retain(|a| a.id != id);
    }
    let text = guidance(&s, &r, Some(MAIN_ID), Bash, None, NOW).text;
    assert!(
        text.starts_with(&format!(
            "Warden: no other account can take delegated work now, so work in this session. If the user names an account, run it there.\n{BACKGROUND_RUNS}\n- near (skipped: near its 5-hour limit): CLAUDE_CONFIG_DIR="
        )),
        "{text}"
    );
    // Accounts listed without a command only: nothing to run, so no sentence.
    for id in ["near", "full"] {
        s.accounts.retain(|a| a.id != id);
    }
    let text = guidance(&s, &r, Some(MAIN_ID), Bash, None, NOW).text;
    assert!(
        text.starts_with(
            "Warden: no other account can take delegated work now, so work in this session. If the user names an account, run it there.\n- out (skipped: signed out)\n"
        ),
        "{text}"
    );
    assert!(!text.contains(BACKGROUND_RUNS), "{text}");
}
#[test]
fn main_and_other_five_hour_limits_hand_off_at_ninety() {
    let (s, mut r) = setup();
    usage(&mut r, MAIN_ID, Some(89.99), Some(10.0), NOW + WEEK_MS);
    assert!(decide(&s, &r, NOW).main_works_itself);
    usage(&mut r, MAIN_ID, Some(90.0), Some(10.0), NOW + WEEK_MS);
    assert!(!decide(&s, &r, NOW).main_works_itself);
    usage(&mut r, "second", Some(90.0), Some(45.0), NOW + HOUR_MS);
    assert_eq!(target(&s, &r).as_deref(), Some("third"));
    r.accounts
        .get_mut("second")
        .expect("second")
        .usage
        .as_mut()
        .expect("usage")
        .five_hour
        .as_mut()
        .expect("five")
        .resets_at = Some(NOW);
    assert_eq!(target(&s, &r).as_deref(), Some("second"));
}
#[test]
fn less_allowance_resetting_sooner_wins_and_ties_are_stable() {
    let (mut s, r) = setup();
    assert_eq!(target(&s, &r).as_deref(), Some("second"));
    s.accounts.swap(0, 1);
    assert_eq!(target(&s, &r).as_deref(), Some("second"));
    let mut r = r;
    usage(&mut r, "third", Some(10.0), Some(45.0), NOW + 48 * HOUR_MS);
    assert_eq!(target(&s, &r).as_deref(), Some("third"));
}

#[test]
fn a_full_week_is_skipped_but_ninety_nine_percent_is_still_ranked() {
    let (s, mut r) = setup();
    usage(&mut r, "second", Some(10.0), Some(100.0), NOW + HOUR_MS);
    usage(&mut r, "third", Some(10.0), None, NOW + WEEK_MS);
    // Even an account with no weekly limit must outrank an exhausted weekly allowance.
    assert_eq!(target(&s, &r).as_deref(), Some("third"));
    usage(&mut r, "third", Some(10.0), Some(10.0), NOW + WEEK_MS);
    usage(&mut r, "second", Some(10.0), Some(99.0), NOW + HOUR_MS);
    // Its one percent resetting in an hour outranks third's ninety percent over a week.
    assert_eq!(target(&s, &r).as_deref(), Some("second"));
}

#[test]
fn all_other_weeks_used_up_leave_the_work_with_main() {
    let (s, mut r) = setup();
    for id in [MAIN_ID, "second", "third"] {
        usage(&mut r, id, Some(10.0), Some(100.0), NOW + WEEK_MS);
    }
    let d = decide(&s, &r, NOW);
    assert!(d.main_works_itself);
    assert!(d.delegated.is_none());
    assert_eq!(
        d.main_reason,
        "No account can take delegated work, so main keeps it."
    );
    assert_eq!(d.delegated_reason, d.main_reason);
}

#[test]
fn an_expired_full_week_that_reset_and_an_unread_account_remain_eligible() {
    let (mut s, mut r) = setup();
    s.accounts[1].enabled = false;
    usage(&mut r, "second", Some(10.0), Some(100.0), NOW + HOUR_MS);
    r.accounts.get_mut("second").expect("second").error =
        Some(UsageError::LoginExpired { status: Some(401) });
    assert!(target(&s, &r).is_none());
    usage(&mut r, "second", Some(10.0), Some(100.0), NOW - 1);
    assert_eq!(target(&s, &r).as_deref(), Some("second"));
    r.accounts.remove("second");
    assert_eq!(target(&s, &r).as_deref(), Some("second"));
}
#[test]
fn each_unavailable_account_is_skipped_including_same_identity_as_main() {
    for why in ["off", "signedOut", "changed", "same", "unknownIdentity"] {
        let (mut s, mut r) = setup();
        match why {
            "off" => s.accounts[0].enabled = false,
            "signedOut" => {
                r.accounts.get_mut("second").expect("second").error = Some(UsageError::SignedOut)
            }
            "changed" => {
                r.accounts
                    .get_mut("second")
                    .expect("second")
                    .identity_change = Some(IdentityChange {
                    now: Some(identity("changed@example.com")),
                    known_account_id: None,
                })
            }
            "same" => s.accounts[0].identity = s.main.identity.clone(),
            _ => s.accounts[0].identity = None,
        }
        assert_eq!(target(&s, &r).as_deref(), Some("third"), "{why}");
    }
}

#[test]
fn a_codex_login_with_the_same_email_is_not_the_claude_main_account() {
    let (mut settings, readings) = setup();
    settings.accounts[0].enabled = false;
    settings.accounts[1].identity = settings.main.identity.clone();
    assert!(!same_as_main(&settings, &settings.accounts[1]));
    assert_eq!(target(&settings, &readings).as_deref(), Some("third"));
    settings.accounts[1].provider = Provider::Claude;
    assert!(same_as_main(&settings, &settings.accounts[1]));
    assert!(target(&settings, &readings).is_none());
}
#[test]
fn expired_logins_keep_the_last_reading_until_its_reset() {
    let (s, mut r) = setup();
    r.accounts.get_mut("second").expect("second").error =
        Some(UsageError::LoginExpired { status: Some(401) });
    r.main_login.sign_in.state = SignInState::Expired;
    r.accounts.get_mut(MAIN_ID).expect("main").error =
        Some(UsageError::LoginExpired { status: None });
    assert_eq!(target(&s, &r).as_deref(), Some("second"));
    assert!(decide(&s, &r, NOW).main_works_itself);
    usage(&mut r, "third", Some(10.0), Some(95.0), NOW - 1);
    usage(&mut r, "second", Some(10.0), Some(99.0), NOW + WEEK_MS);
    assert_eq!(target(&s, &r).as_deref(), Some("third"));
}
#[test]
fn unknown_main_delegates_and_nothing_available_keeps_work_on_main() {
    for state in [
        SignInState::SignedOut,
        SignInState::OtherAccount,
        SignInState::SignedIn,
    ] {
        let (mut s, mut r) = setup();
        r.main_login.sign_in.state = state;
        if state == SignInState::SignedIn {
            r.accounts.remove(MAIN_ID);
        }
        assert!(!decide(&s, &r, NOW).main_works_itself);
        for a in &mut s.accounts {
            a.enabled = false;
        }
        let d = decide(&s, &r, NOW);
        assert!(d.main_works_itself);
        assert!(d.delegated.is_none());
        assert_eq!(
            d.main_reason,
            "No account can take delegated work, so main keeps it."
        );
    }
}
#[test]
fn new_readings_count_as_zero_and_accounts_without_weekly_limits_rank_last() {
    let (s, mut r) = setup();
    r.accounts.remove("second");
    assert_eq!(target(&s, &r).as_deref(), Some("second"));
    usage(&mut r, "third", Some(0.0), None, NOW + WEEK_MS);
    assert_eq!(target(&s, &r).as_deref(), Some("second"));
    usage(&mut r, "second", Some(20.0), None, NOW + WEEK_MS);
    assert_eq!(target(&s, &r).as_deref(), Some("third"));
    assert!(
        decide(&s, &r, NOW)
            .delegated_reason
            .contains("no weekly limit")
    );
}
#[test]
fn advice_is_only_for_main_and_changes_only_with_the_decision_or_command() {
    let (s, mut r) = setup();
    let g = guidance(&s, &r, Some(MAIN_ID), Bash, Some("session-1"), NOW);
    assert!(g.text.starts_with(
        "Warden: work in this session. If the user asks to delegate, Warden's pick is second. The user can always choose: work here, or run on whichever account they name.\nRun a delegated command in the background and read its result when it finishes; a long run in the foreground is cut off.\n- second (Warden's pick): WARDEN_PARENT_SESSION='session-1' CLAUDE_CONFIG_DIR="
    ), "{}", g.text);
    assert!(
        g.text
            .contains("\n- third: WARDEN_PARENT_SESSION='session-1' CODEX_HOME=")
    );
    // A change in an account's percentages alone (no change of pick or skip) says nothing new.
    usage(&mut r, "second", Some(20.0), Some(50.0), NOW + 48 * HOUR_MS);
    assert_eq!(
        g.key,
        guidance(&s, &r, Some(MAIN_ID), Bash, Some("session-1"), NOW).key
    );
    // A skip changes the advice.
    usage(&mut r, "second", Some(95.0), Some(50.0), NOW + 48 * HOUR_MS);
    let skipped = guidance(&s, &r, Some(MAIN_ID), Bash, Some("session-1"), NOW);
    assert_ne!(g.key, skipped.key);
    assert!(
        skipped
            .text
            .contains("- second (skipped: near its 5-hour limit):")
    );
    usage(&mut r, "second", Some(10.0), Some(45.0), NOW + 48 * HOUR_MS);
    assert_eq!(
        g.key,
        guidance(&s, &r, Some(MAIN_ID), Bash, Some("session-1"), NOW + 60_000).key
    );
    usage(&mut r, MAIN_ID, Some(0.0), Some(95.0), NOW + WEEK_MS);
    let after = guidance(&s, &r, Some(MAIN_ID), Bash, Some("session-1"), NOW);
    assert_ne!(g.key, after.key);
    assert!(after.text.starts_with(
        "Warden: main is handing work off now, so run each task on second (Warden's pick); here, only talk to the user and check results. Whenever the user asks, work in this session instead, or run on the account they name.\nRun a delegated command in the background"
    ), "{}", after.text);
    // Nothing to delegate to: work here, and the user may still name an account.
    let mut none = s.clone();
    for a in &mut none.accounts {
        a.enabled = false;
    }
    assert_eq!(
        guidance(&none, &r, Some(MAIN_ID), Bash, None, NOW).text,
        "Warden: no other account can take delegated work now, so work in this session. If the user names an account, run it there.\n"
    );
    for id in [None, Some("second"), Some("unknown")] {
        assert!(guidance(&s, &r, id, Bash, None, NOW).text.is_empty());
    }
    r.accounts.get_mut(MAIN_ID).expect("main").identity_change = Some(IdentityChange {
        now: None,
        known_account_id: None,
    });
    assert!(
        guidance(&s, &r, Some(MAIN_ID), Bash, None, NOW)
            .text
            .is_empty()
    );
}
#[test]
fn exact_official_commands_quote_paths_and_restore_both_variables() {
    let mut a = account("second", Provider::Claude);
    a.folder = PathBuf::from("<home>/it's my ’acc’");
    assert_eq!(
        command(&a, Shell::Bash, Some("parent-1")),
        "WARDEN_PARENT_SESSION='parent-1' CLAUDE_CONFIG_DIR='<home>/it'\\''s my ’acc’' claude -p --permission-mode auto \"<task>\""
    );
    assert_eq!(
        command(&a, Shell::PowerShell, Some("parent-1")),
        "& { $was = $env:CLAUDE_CONFIG_DIR; $parentWas = $env:WARDEN_PARENT_SESSION; try { $env:CLAUDE_CONFIG_DIR = '<home>/it''s my ’’acc’’'; $env:WARDEN_PARENT_SESSION = 'parent-1'; claude -p --permission-mode auto \"<task>\" } finally { $env:CLAUDE_CONFIG_DIR = $was; $env:WARDEN_PARENT_SESSION = $parentWas; } }"
    );
    a.provider = Provider::Codex;
    assert!(command(&a, Shell::Bash, None).ends_with("codex exec --approve-for-me \"<task>\""));
    assert!(command(&a, Shell::PowerShell, None).contains("$env:CODEX_HOME"));
    for bad in ["", "bad'id", "../bad", "bad;exit", &"a".repeat(129)] {
        for shell in [Shell::Bash, Shell::PowerShell] {
            assert!(!command(&a, shell, Some(bad)).contains("WARDEN_PARENT_SESSION"));
        }
    }
    for quote in ['\'', '\u{2018}', '\u{2019}', '\u{201a}', '\u{201b}'] {
        assert_eq!(
            Shell::PowerShell.literal(&quote.to_string()),
            format!("'{quote}{quote}'")
        );
    }
    assert_eq!(Shell::of(Provider::Codex, Bash), Shell::PowerShell);
    assert_eq!(Shell::of(Provider::Claude, Bash), Shell::Bash);
    assert_eq!(Shell::of(Provider::Claude, PowerShell), Shell::PowerShell);
}
#[test]
fn reading_staleness_uses_interval_plus_one_minute() {
    let (s, mut r) = setup();
    assert!(readings_are_stale(&s, &r, NOW));
    r.checked_at = Some(NOW);
    assert!(!readings_are_stale(&s, &r, NOW));
    assert!(readings_are_stale(
        &s,
        &r,
        NOW + (i64::from(s.usage_interval_min) + 2) * 60_000
    ));
    assert!(readings_are_stale(&s, &r, NOW - 1));
}
