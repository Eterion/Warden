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
        resets: None,
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
/// Whether main is ranked by its own week: below its hand-off point and its 5-hour window.
fn main_ranked(d: &Decision) -> bool {
    d.order.iter().any(|e| e.account_id == MAIN_ID && !e.reset)
}
/// The ranking, by account id.
fn order(d: &Decision) -> Vec<&str> {
    d.order.iter().map(|e| e.account_id.as_str()).collect()
}

#[test]
fn reserve_line_shrinks_at_start_middle_and_end_of_week() {
    // The hand-off point is the line rounded down, and the same whole number decides.
    for (left, line, hand_off_at) in [
        (WEEK_MS, 75.0, 75),
        (WEEK_MS / 2, 87.5, 87),
        (0, 100.0, 100),
    ] {
        assert_eq!(reserve_now(25, None, Some(NOW + left), NOW).0, 100.0 - line);
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
        assert!(main_ranked(&d));
        // A reset already passed makes the reading zero, irrespective of its previous share.
        if left > 0 {
            usage(
                &mut r,
                MAIN_ID,
                Some(10.0),
                Some(f64::from(hand_off_at)),
                NOW + left,
            );
            assert!(!main_ranked(&decide(&s, &r, NOW)));
        }
    }
    assert_eq!(reserve_now(25, None, Some(NOW + 2 * WEEK_MS), NOW).0, 25.0);
    assert_eq!(reserve_now(25, None, Some(NOW - 1), NOW).0, 0.0);
    assert_eq!(reserve_now(25, None, None, NOW), (25.0, WEEK_MS, WEEK_MS));
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
    let reserve = |now| reserve_with(25, Some(&w), reset, now, &cet).0;
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
    assert!((sat.0 - 25.0 * 12.0 / 45.0).abs() < 1e-9);
    // With the time that counts: 12 of 45 work hours.
    assert_eq!((sat.1, sat.2), (12 * HOUR_MS, 45 * HOUR_MS));
    // A reset more than a week away counts as a week: the full reserve.
    assert_eq!(
        reserve_with(25, Some(&w), Some(at(20, 0, 0)), at(0, 0, 0), &cet).0,
        25.0
    );
    // No reset known: the full reserve.
    assert_eq!(
        reserve_with(25, Some(&w), None, at(0, 0, 0), &cet),
        (25.0, 45 * HOUR_MS, 45 * HOUR_MS)
    );
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
    assert_eq!(main_ranked(&d), 50.0 < f64::from(d.hand_off_at));
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
    assert!(
        text.contains(&format!(
            "{BACKGROUND_RUNS}\n{STARTED_FOR_YOU}\n{NAME_AND_TASK}\n"
        )),
        "{text}"
    );
    assert!(!text.contains(TASK), "{text}");
    // Only Codex commands listed: only the task is explained, as Codex's command has no name.
    let mut codex_only = s.clone();
    codex_only.accounts.retain(|a| {
        a.provider == Provider::Codex || !matches!(a.id.as_str(), "second" | "near" | "full")
    });
    let text = guidance(&codex_only, &r, Some(MAIN_ID), Bash, None, NOW).text;
    assert!(
        text.contains(&format!("{STARTED_FOR_YOU}\n{TASK}\n- third")),
        "{text}"
    );
    assert!(!text.contains(NAME_AND_TASK), "{text}");
    // Only accounts skipped for their limits keep a command: no pick, and still the sentence.
    for id in ["second", "third"] {
        s.accounts.retain(|a| a.id != id);
    }
    let text = guidance(&s, &r, Some(MAIN_ID), Bash, None, NOW).text;
    assert!(
        text.starts_with(&format!(
            "Warden: no other account can take delegated work now, so work in this session. If the user names an account, run it there.\n{BACKGROUND_RUNS}\n{STARTED_FOR_YOU}\n{NAME_AND_TASK}\n- near (skipped: near its 5-hour limit): CLAUDE_CONFIG_DIR="
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
    assert!(!text.contains(STARTED_FOR_YOU), "{text}");
    assert!(!text.contains("Replace `<"), "{text}");
}
#[test]
fn main_and_other_five_hour_limits_hand_off_at_ninety() {
    let (s, mut r) = setup();
    usage(&mut r, MAIN_ID, Some(89.99), Some(10.0), NOW + WEEK_MS);
    assert!(main_ranked(&decide(&s, &r, NOW)));
    usage(&mut r, MAIN_ID, Some(90.0), Some(10.0), NOW + WEEK_MS);
    let d = decide(&s, &r, NOW);
    assert!(!main_ranked(&d));
    assert_eq!(d.why, Why::MainFiveHourFull);
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
    assert_eq!(d.why, Why::NoOther);
    assert!(d.order.is_empty());
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
    // Main still counts on its expired sign-in, after second, whose week resets sooner.
    let d = decide(&s, &r, NOW);
    assert_eq!(order(&d), ["second", "third", MAIN_ID]);
    assert_eq!(d.why, Why::Soonest);
    assert!(!d.main_works_itself);
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
        assert_eq!(d.why, Why::NoOther);
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
    let d = decide(&s, &r, NOW);
    assert_eq!(order(&d), [MAIN_ID, "third", "second"]);
    assert_eq!(d.order[2].deadline, None);
}
#[test]
fn advice_is_only_for_main_and_changes_only_with_the_decision_or_command() {
    let (s, mut r) = setup();
    // Main's week resets soonest: it works itself.
    usage(&mut r, MAIN_ID, Some(10.0), Some(10.0), NOW + 24 * HOUR_MS);
    let g = guidance(&s, &r, Some(MAIN_ID), Bash, Some("session-1"), NOW);
    assert!(g.text.starts_with(
        "Warden: work in this session: main's week resets soonest of the accounts with room. If the user asks to delegate, Warden's pick is second. The user can always choose: work here, or run on whichever account they name.\nRun a delegated command in the background and read its result when it finishes; a long run in the foreground is cut off.\nAnything you start that runs Claude or Codex for you (a review script, say) is delegated work too: run it with the same variables as that account's command above.\nReplace `<name>` with a few words saying what the run does, and `<task>` with the task.\n- second (Warden's pick): WARDEN_PARENT_SESSION='session-1' CLAUDE_CONFIG_DIR="
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
        "Warden: main is handing work off now (main keeps the rest of its week for the user), so run each task on second (Warden's pick: its week resets soonest of the accounts with room); here, only talk to the user and check results. Whenever the user asks, work in this session instead, or run on the account they name.\nRun a delegated command in the background"
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
        "WARDEN_PARENT_SESSION='parent-1' CLAUDE_CONFIG_DIR='<home>/it'\\''s my ’acc’' claude -p --permission-mode auto --name \"<name>\" \"<task>\""
    );
    assert_eq!(
        command(&a, Shell::PowerShell, Some("parent-1")),
        "& { $was = $env:CLAUDE_CONFIG_DIR; $parentWas = $env:WARDEN_PARENT_SESSION; try { $env:CLAUDE_CONFIG_DIR = '<home>/it''s my ’’acc’’'; $env:WARDEN_PARENT_SESSION = 'parent-1'; claude -p --permission-mode auto --name \"<name>\" \"<task>\" } finally { $env:CLAUDE_CONFIG_DIR = $was; $env:WARDEN_PARENT_SESSION = $parentWas; } }"
    );
    // Without a parent session the run is still named.
    assert_eq!(
        command(&a, Shell::Bash, None),
        "CLAUDE_CONFIG_DIR='<home>/it'\\''s my ’acc’' claude -p --permission-mode auto --name \"<name>\" \"<task>\""
    );
    assert_eq!(
        command(&a, Shell::PowerShell, None),
        "& { $was = $env:CLAUDE_CONFIG_DIR; try { $env:CLAUDE_CONFIG_DIR = '<home>/it''s my ’’acc’’'; claude -p --permission-mode auto --name \"<name>\" \"<task>\" } finally { $env:CLAUDE_CONFIG_DIR = $was; } }"
    );
    a.provider = Provider::Codex;
    assert!(command(&a, Shell::Bash, None).ends_with("codex exec --approve-for-me \"<task>\""));
    assert!(!command(&a, Shell::PowerShell, None).contains("--name"));
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

/// The rule's example (2026-11-23): Claude 2 used 31% of its week and resets in 15 hours, Codex resets in 72 hours,
/// main used 24% and resets in 93 hours. The week lost soonest goes first, then the next, then main.
fn example() -> (Settings, Readings) {
    let (s, mut r) = setup();
    usage(&mut r, MAIN_ID, Some(10.0), Some(24.0), NOW + 93 * HOUR_MS);
    usage(&mut r, "second", Some(10.0), Some(31.0), NOW + 15 * HOUR_MS);
    usage(&mut r, "third", Some(10.0), Some(40.0), NOW + 72 * HOUR_MS);
    (s, r)
}

#[test]
fn the_week_lost_soonest_takes_the_work_first() {
    let (s, r) = example();
    let d = decide(&s, &r, NOW);
    assert_eq!(order(&d), ["second", "third", MAIN_ID]);
    assert_eq!(d.order[0].deadline, Some(NOW + 15 * HOUR_MS));
    assert!(!d.main_works_itself);
    assert_eq!(target(&s, &r).as_deref(), Some("second"));
    assert_eq!(d.why, Why::Soonest);
    assert_eq!(d.suggested_reset, None);
    // Once Claude 2's week has reset, a fresh week puts it last of the weekly ones: Codex next.
    let d = decide(&s, &r, NOW + 16 * HOUR_MS);
    assert_eq!(order(&d), ["third", MAIN_ID, "second"]);
    assert_eq!(d.delegated.map(|t| t.account_id).as_deref(), Some("third"));
    // Then main, up to its hand-off point.
    let d = decide(&s, &r, NOW + 73 * HOUR_MS);
    assert_eq!(order(&d)[0], MAIN_ID);
    assert!(d.main_works_itself);
    assert_eq!(d.why, Why::MainSoonest);
    // Warden's pick among the others stays known, for when the user asks to delegate.
    assert!(d.delegated.is_some());
}

#[test]
fn main_works_itself_as_the_pick_or_when_nothing_else_has_room() {
    let (s, mut r) = example();
    usage(&mut r, MAIN_ID, Some(10.0), Some(24.0), NOW + 10 * HOUR_MS);
    let d = decide(&s, &r, NOW);
    assert!(d.main_works_itself);
    assert_eq!(d.why, Why::MainSoonest);
    assert_eq!(d.delegated.map(|t| t.account_id).as_deref(), Some("second"));
    // Nothing else with room: main keeps the work, even past its hand-off point.
    for id in ["second", "third"] {
        usage(&mut r, id, Some(10.0), Some(100.0), NOW + WEEK_MS);
    }
    let d = decide(&s, &r, NOW);
    assert!(d.main_works_itself);
    assert_eq!((d.why, d.delegated.clone()), (Why::NoOther, None));
    assert_eq!(order(&d), [MAIN_ID]);
    usage(&mut r, MAIN_ID, Some(10.0), Some(99.0), NOW + 10 * HOUR_MS);
    let d = decide(&s, &r, NOW);
    assert!(d.main_works_itself && d.order.is_empty());
    assert_eq!(d.why, Why::NoOther);
}

#[test]
fn main_delegates_while_its_five_hour_window_is_full_even_resetting_soonest() {
    let (s, mut r) = example();
    usage(&mut r, MAIN_ID, Some(90.0), Some(24.0), NOW + 10 * HOUR_MS);
    let d = decide(&s, &r, NOW);
    assert!(!d.main_works_itself);
    assert_eq!(d.why, Why::MainFiveHourFull);
    assert_eq!(order(&d), ["second", "third"]);
    let text = guidance(&s, &r, Some(MAIN_ID), Bash, None, NOW).text;
    assert!(
        text.starts_with("Warden: main is handing work off now (main's 5-hour window is full), so run each task on second (Warden's pick: its week resets soonest of the accounts with room);"),
        "{text}"
    );
    // Above its hand-off point main keeps the rest of its week for the user.
    usage(&mut r, MAIN_ID, Some(10.0), Some(90.0), NOW + 93 * HOUR_MS);
    assert_eq!(decide(&s, &r, NOW).why, Why::MainKeeps);
}

#[test]
fn within_the_same_hour_more_room_wins_then_settings_order_with_main_first() {
    let (s, mut r) = setup();
    let hour = (NOW / HOUR_MS + 30) * HOUR_MS;
    usage(
        &mut r,
        "second",
        Some(10.0),
        Some(50.0),
        hour + 50 * MINUTE_MS,
    );
    usage(
        &mut r,
        "third",
        Some(10.0),
        Some(60.0),
        hour + 5 * MINUTE_MS,
    );
    // Third resets 45 minutes sooner, in the same hour, but second has more room.
    assert_eq!(target(&s, &r).as_deref(), Some("second"));
    usage(
        &mut r,
        "third",
        Some(10.0),
        Some(40.0),
        hour + 5 * MINUTE_MS,
    );
    assert_eq!(target(&s, &r).as_deref(), Some("third"));
    // The next hour is later, whatever the room.
    usage(&mut r, "third", Some(10.0), Some(0.0), hour + HOUR_MS);
    assert_eq!(target(&s, &r).as_deref(), Some("second"));
    // The same room: settings order.
    usage(
        &mut r,
        "third",
        Some(10.0),
        Some(50.0),
        hour + 10 * MINUTE_MS,
    );
    let d = decide(&s, &r, NOW);
    assert_eq!(order(&d)[..2], ["second", "third"]);
    // Main first among equals: with no reserve its room goes up to 100%, so 50% used leaves it 50 too.
    let (mut main_first, _) = setup();
    main_first.reserve = 0;
    usage(
        &mut r,
        MAIN_ID,
        Some(10.0),
        Some(50.0),
        hour + 20 * MINUTE_MS,
    );
    let d = decide(&main_first, &r, NOW);
    assert_eq!(d.hand_off_at, 100);
    assert_eq!(order(&d)[..3], [MAIN_ID, "second", "third"]);
    assert!(d.main_works_itself);
}

#[test]
fn accounts_without_a_weekly_limit_come_after_every_one_with_one() {
    let (s, mut r) = example();
    usage(&mut r, "second", Some(5.0), None, NOW + WEEK_MS);
    let d = decide(&s, &r, NOW);
    assert_eq!(order(&d), ["third", MAIN_ID, "second"]);
    assert_eq!(d.order[2].deadline, None);
}

fn resets(r: &mut Readings, id: &str, list: &[(&str, i64)]) {
    r.accounts.entry(id.into()).or_default().resets = list
        .iter()
        .map(|(title, at)| Reset {
            id: Some(format!("{id}-{title}")),
            title: Some((*title).into()),
            expires_at: Some(*at),
        })
        .collect();
}

#[test]
fn a_reset_that_expires_soonest_is_suggested_and_the_work_goes_on_meanwhile() {
    let (s, mut r) = example();
    // Codex has used its week (resets in 72 hours); its Full reset expires in 10 hours, before Claude 2's week.
    usage(&mut r, "third", Some(10.0), Some(100.0), NOW + 72 * HOUR_MS);
    resets(
        &mut r,
        "third",
        &[("Full reset (Weekly + 5 hr)", NOW + 10 * HOUR_MS)],
    );
    let d = decide(&s, &r, NOW);
    assert_eq!(order(&d), ["third", "second", MAIN_ID]);
    assert!(d.order[0].reset && !d.order[0].can_work);
    assert_eq!(
        d.suggested_reset,
        Some(Suggested {
            account_id: "third".into(),
            reset_id: "third-Full reset (Weekly + 5 hr)".into(),
            title: "Full reset (Weekly + 5 hr)".into(),
            expires_at: NOW + 10 * HOUR_MS,
        })
    );
    // Meanwhile the work goes to the next account.
    assert_eq!(target(&s, &r).as_deref(), Some("second"));
    assert!(!d.main_works_itself);
    let text = guidance(&s, &r, Some(MAIN_ID), Bash, None, NOW).text;
    assert!(
        text.contains("\nTell the user: Warden suggests using third's Full reset (Weekly + 5 hr) on Warden's Overview, as it expires before third's week resets. Using it gives third's week back, and Warden ranks third again; until then, work goes as above.\n"),
        "{text}"
    );
    // Its 5-hour window full too: a full reset refills both, so it still counts.
    usage(&mut r, "third", Some(95.0), Some(100.0), NOW + 72 * HOUR_MS);
    assert!(decide(&s, &r, NOW).suggested_reset.is_some());
    // Once used: the reset is gone and the readings refill, so it is ranked by its week as usual.
    r.use_reset("third", "third-Full reset (Weekly + 5 hr)");
    usage(&mut r, "third", Some(0.0), Some(0.0), NOW + 72 * HOUR_MS);
    let d = decide(&s, &r, NOW);
    assert_eq!(d.suggested_reset, None);
    assert_eq!(order(&d), ["second", "third", MAIN_ID]);
    assert!(!d.order.iter().any(|e| e.reset));
}

#[test]
fn an_account_with_room_spends_it_before_its_reset_is_suggested() {
    let (s, mut r) = example();
    // Both Warden accounts at 95%, each with a reset lost before Claude 2's week: they keep their own room and are
    // ranked by their resets' expiry, so the work goes to them first and nothing is suggested yet.
    usage(&mut r, "second", Some(10.0), Some(95.0), NOW + 15 * HOUR_MS);
    resets(&mut r, "second", &[("Full reset", NOW + 8 * HOUR_MS)]);
    usage(&mut r, "third", Some(10.0), Some(95.0), NOW + 72 * HOUR_MS);
    resets(&mut r, "third", &[("Weekly reset", NOW + 5 * HOUR_MS)]);
    let d = decide(&s, &r, NOW);
    assert_eq!(order(&d), ["third", "second", MAIN_ID]);
    assert!(d.order.iter().take(2).all(|e| e.reset && e.can_work));
    assert_eq!(d.suggested_reset, None);
    assert_eq!(target(&s, &r).as_deref(), Some("third"));
    // Once third's week is used up, its reset is suggested and second takes the work meanwhile.
    usage(&mut r, "third", Some(10.0), Some(100.0), NOW + 72 * HOUR_MS);
    let d = decide(&s, &r, NOW);
    assert_eq!(
        d.suggested_reset.map(|x| x.account_id).as_deref(),
        Some("third")
    );
    assert_eq!(d.delegated.map(|t| t.account_id).as_deref(), Some("second"));
}

#[test]
fn only_a_reset_that_lets_the_account_work_again_counts() {
    let (s, mut r) = example();
    let suggested = |r: &Readings| decide(&s, r, NOW).suggested_reset.map(|x| x.account_id);
    usage(&mut r, "third", Some(10.0), Some(100.0), NOW + 72 * HOUR_MS);
    resets(&mut r, "third", &[("Weekly reset", NOW + 10 * HOUR_MS)]);
    assert_eq!(suggested(&r).as_deref(), Some("third"));
    // A weekly reset leaves a full 5-hour window full: it does not count then.
    usage(&mut r, "third", Some(95.0), Some(100.0), NOW + 72 * HOUR_MS);
    assert_eq!(suggested(&r), None);
    usage(&mut r, "third", Some(10.0), Some(100.0), NOW + 72 * HOUR_MS);
    // Below 90%: ranked by its week, and the reset waits.
    usage(&mut r, "third", Some(10.0), Some(89.0), NOW + 72 * HOUR_MS);
    assert!(!decide(&s, &r, NOW).order.iter().any(|e| e.reset));
    usage(&mut r, "third", Some(10.0), Some(100.0), NOW + 72 * HOUR_MS);
    // A 5-hour or Fable reset gives no week back; one expiring after the week resets is not lost; an expired one is
    // gone.
    for (title, at) in [
        ("5-hour reset", NOW + 10 * HOUR_MS),
        ("Fable reset", NOW + 10 * HOUR_MS),
        ("Full reset", NOW + 80 * HOUR_MS),
        ("Full reset", NOW - 1),
    ] {
        resets(&mut r, "third", &[(title, at)]);
        assert_eq!(suggested(&r), None, "{title}");
    }
    // One without an id cannot be used.
    resets(&mut r, "third", &[("Full reset", NOW + 10 * HOUR_MS)]);
    r.accounts.get_mut("third").expect("third").resets[0].id = None;
    assert_eq!(suggested(&r), None);
    // Ranked after another account's week: listed, not suggested.
    resets(&mut r, "third", &[("Full reset", NOW + 20 * HOUR_MS)]);
    let d = decide(&s, &r, NOW);
    assert_eq!(order(&d), ["second", "third", MAIN_ID]);
    assert_eq!(d.suggested_reset, None);
    // An expired login cannot use one, and its last reading still counts as usual.
    resets(&mut r, "third", &[("Full reset", NOW + 10 * HOUR_MS)]);
    r.accounts.get_mut("third").expect("third").error =
        Some(UsageError::LoginExpired { status: Some(401) });
    assert_eq!(suggested(&r), None);
    usage(&mut r, "third", Some(10.0), Some(95.0), NOW + 72 * HOUR_MS);
    r.accounts.get_mut("third").expect("third").error =
        Some(UsageError::LoginExpired { status: Some(401) });
    assert_eq!(order(&decide(&s, &r, NOW)), ["second", "third", MAIN_ID]);
    // Signed out: no reset counts.
    usage(&mut r, "third", Some(10.0), Some(100.0), NOW + 72 * HOUR_MS);
    r.accounts.get_mut("third").expect("third").error = Some(UsageError::SignedOut);
    assert_eq!(suggested(&r), None);
    // Main's own reset counts the same way: its own sign-in uses it.
    usage(&mut r, MAIN_ID, Some(10.0), Some(95.0), NOW + 93 * HOUR_MS);
    resets(&mut r, MAIN_ID, &[("Full reset", NOW + 5 * HOUR_MS)]);
    let d = decide(&s, &r, NOW);
    assert_eq!(
        d.suggested_reset.map(|x| x.account_id).as_deref(),
        Some(MAIN_ID)
    );
    assert_eq!(d.why, Why::MainKeeps);
    assert_eq!(target(&s, &r).as_deref(), Some("second"));
}

#[test]
fn an_assumed_week_is_marked_and_the_reserve_carries_its_figures() {
    let (mut s, mut r) = example();
    r.accounts.remove("second");
    usage(&mut r, "third", Some(10.0), Some(40.0), NOW - 1);
    let d = decide(&s, &r, NOW);
    for e in &d.order {
        assert_eq!(e.estimated, e.account_id != MAIN_ID, "{}", e.account_id);
    }
    // Main resets in 93 hours of 168: 25% of its week shrinks to 13.8%, so it keeps 14% (takes work up to 86%).
    assert_eq!(
        d.reserve,
        Reserve {
            configured: 25,
            kept: 14,
            resets_at: Some(NOW + 93 * HOUR_MS),
            work_hours: false,
            left_min: 93 * 60,
            week_min: 168 * 60,
        }
    );
    assert_eq!(d.hand_off_at, 86);
    // Main's reset unknown: all of it is kept, all of the week counts.
    r.accounts.get_mut(MAIN_ID).expect("main").usage = None;
    let d = decide(&s, &r, NOW);
    assert_eq!(
        (d.reserve.kept, d.reserve.resets_at, d.reserve.left_min),
        (25, None, 168 * 60)
    );
    // Work hours: the week holds 45 of them.
    s.work_hours = Some(office());
    let d = decide(&s, &r, NOW);
    assert!(d.reserve.work_hours);
    assert_eq!(d.reserve.week_min, 45 * 60);
}
