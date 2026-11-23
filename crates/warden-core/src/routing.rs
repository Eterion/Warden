//! Delegation from fixed main's reserve and the allowance that would otherwise go unused.
use crate::codex_resets::Reset;
use crate::hooks::ClaudeShell;
use crate::mainlogin::SignInState;
use crate::model::{Limit, Provider, UsageError};
use crate::store::{AccountConfig, AccountReading, MAIN_ID, Readings, Settings, WorkHours};
use serde::Serialize;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;
const WEEK_MS: i64 = 7 * DAY_MS;
const MINUTE_MS: i64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub account_id: String,
}

/// Why the work goes where it goes now, for the status bar and main's advice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Why {
    /// Main is the pick: its week resets soonest of the accounts with room.
    MainSoonest,
    /// Nothing but main can take the work: main keeps it, whatever its own usage.
    NoOther,
    /// Main has room, but the pick's week resets sooner.
    Soonest,
    /// Main is at its hand-off point: the rest of its week is kept for the user.
    MainKeeps,
    /// Main has used 90% or more of its 5-hour window.
    MainFiveHourFull,
    /// Main's usage is unknown (Warden is not signed in to it, or it is signed in as someone else).
    MainUnknown,
}

/// An account that can take work, as Warden ranks them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ranked {
    pub account_id: String,
    /// When its allowance is lost: its weekly reset, or a limit reset's expiry (`reset`). `None`: no weekly limit.
    pub deadline: Option<i64>,
    /// Ranked by a limit reset that gives it the work once used (its week is at 90% or more).
    pub reset: bool,
}

/// A limit reset Warden suggests using: it expires before its account's week resets, and before any other account's
/// allowance is lost. Only the user uses it, in the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggested {
    pub account_id: String,
    pub reset_id: String,
    pub title: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub main_works_itself: bool,
    /// Warden's pick among the other accounts: where main delegates now, or would if the user asks.
    pub delegated: Option<Target>,
    pub why: Why,
    /// Main's hand-off point: the whole percent of its week up to which it takes work (100 less the reserve now,
    /// rounded down). The same number decides, so what is shown is what Warden does.
    pub hand_off_at: u8,
    /// The accounts that can take work, main too, in Warden's order: soonest deadline first.
    pub order: Vec<Ranked>,
    pub suggested_reset: Option<Suggested>,
    /// Each switched-on Warden account, in settings order, and why Warden skips it (if it does): main's advice lists
    /// them so the user can always choose. Not part of the API.
    #[serde(skip)]
    pub accounts: Vec<Listed>,
}

/// A switched-on Warden account in main's advice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub account_id: String,
    pub skip: Option<Skip>,
}

/// Why Warden skips an account for delegated work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    NearFiveHourLimit,
    WeekUsedUp,
    SignedOut,
    SignInChanged,
    SameAsMain,
}

impl Skip {
    /// Whether the account can still run work if the user chooses it: only limits skip it, never someone else's login.
    pub fn can_run(self) -> bool {
        matches!(self, Skip::NearFiveHourLimit | Skip::WeekUsedUp)
    }

    fn words(self) -> &'static str {
        match self {
            Skip::NearFiveHourLimit => "near its 5-hour limit",
            Skip::WeekUsedUp => "its week is used up",
            Skip::SignedOut => "signed out",
            Skip::SignInChanged => "signed in as someone else, waiting for the user to confirm",
            Skip::SameAsMain => "same account as main",
        }
    }
}

fn used_now(limit: Option<&Limit>, now: i64) -> Option<f64> {
    limit.map(|l| {
        if l.resets_at.is_some_and(|r| r <= now) {
            0.0
        } else {
            l.used_percent
        }
    })
}

pub fn same_as_main(settings: &Settings, account: &AccountConfig) -> bool {
    account.provider == Provider::Claude
        && settings
            .main
            .identity
            .as_ref()
            .zip(account.identity.as_ref())
            .is_some_and(|(main, other)| main.same_as(other))
}

/// Why a switched-on account cannot run delegated work at all (not its limits): whose login it holds.
fn login_skip(
    settings: &Settings,
    account: &AccountConfig,
    reading: Option<&AccountReading>,
) -> Option<Skip> {
    if reading.is_some_and(|r| r.identity_change.is_some()) {
        Some(Skip::SignInChanged)
    } else if account.identity.is_none()
        || reading.is_some_and(|r| r.error == Some(UsageError::SignedOut))
    {
        Some(Skip::SignedOut)
    } else if same_as_main(settings, account) {
        Some(Skip::SameAsMain)
    } else {
        None
    }
}

/// Windows' offset from UTC at a moment, in seconds (UTC when the lookup fails, as `reset_text` does).
fn local_offset_s(at_ms: i64) -> i64 {
    time::OffsetDateTime::from_unix_timestamp(at_ms.div_euclid(1000))
        .ok()
        .and_then(|t| time::UtcOffset::local_offset_at(t).ok())
        .map_or(0, |o| i64::from(o.whole_seconds()))
}

/// The reserve now: it shrinks with what is left of main's week before its reset, never below zero or above the
/// configured reserve. Without work hours that is the time left; with them, the work hours left.
fn reserve_now(
    reserve: u8,
    work_hours: Option<&WorkHours>,
    resets_at: Option<i64>,
    now: i64,
) -> f64 {
    reserve_with(reserve, work_hours, resets_at, now, &local_offset_s)
}

/// [`reserve_now`] with the offset lookup given, so tests use fixed offsets (and a daylight-saving change).
fn reserve_with(
    reserve: u8,
    work_hours: Option<&WorkHours>,
    resets_at: Option<i64>,
    now: i64,
    offset_s: &dyn Fn(i64) -> i64,
) -> f64 {
    let Some(at) = resets_at else {
        return f64::from(reserve);
    };
    let end = at.min(now.saturating_add(WEEK_MS));
    let share = match work_hours {
        None => (end - now).clamp(0, WEEK_MS) as f64 / WEEK_MS as f64,
        Some(w) => {
            let week = w.days.len() as i64 * i64::from(w.to - w.from) * MINUTE_MS;
            if week == 0 {
                return f64::from(reserve);
            }
            (work_ms_between(w, now, end, offset_s) as f64 / week as f64).clamp(0.0, 1.0)
        }
    };
    f64::from(reserve) * share
}

/// How much of the work windows (each work day's `from` to `to`, local time) falls between `start` and `end`.
fn work_ms_between(w: &WorkHours, start: i64, end: i64, offset_s: &dyn Fn(i64) -> i64) -> i64 {
    if end <= start {
        return 0;
    }
    let to_utc = |local: i64| local_to_utc(local, offset_s);
    let first_day = (start + offset_s(start) * 1000).div_euclid(DAY_MS);
    let last_day = (end + offset_s(end) * 1000).div_euclid(DAY_MS);
    let mut total = 0;
    for day in first_day..=last_day {
        // 1970-01-01 was a Thursday (ISO 4).
        let weekday = u8::try_from((day + 3).rem_euclid(7) + 1).unwrap_or(1);
        if !w.days.contains(&weekday) {
            continue;
        }
        let midnight = day * DAY_MS;
        let from = to_utc(midnight + i64::from(w.from) * MINUTE_MS);
        let to = to_utc(midnight + i64::from(w.to) * MINUTE_MS);
        total += (to.min(end) - from.max(start)).max(0);
    }
    total
}

/// A local wall-clock moment (ms, as if UTC) back to UTC, as JavaScript's `Date` does it, so the UI's preview
/// agrees: a time that occurs twice (the clocks going back) is its first occurrence; a time that never occurs (the
/// clocks going forward) is read with the offset before the change.
fn local_to_utc(local: i64, offset_s: &dyn Fn(i64) -> i64) -> i64 {
    let before = offset_s(local - DAY_MS);
    let after = offset_s(local + DAY_MS);
    let fits = |o: i64| {
        let utc = local - o * 1000;
        (offset_s(utc) == o).then_some(utc)
    };
    match (fits(before), fits(after)) {
        (Some(a), Some(b)) => a.min(b),
        (Some(u), None) | (None, Some(u)) => u,
        (None, None) => local - before * 1000,
    }
}

/// Main's hand-off point for a reserve: 100 less it, rounded down to a whole percent.
fn hand_off(reserve_now: f64) -> u8 {
    (100.0 - reserve_now).floor().clamp(0.0, 100.0) as u8
}

/// A reset (or any moment) as the user reads it: "Fri 05:00" in local time, or "its weekly reset" when unknown.
pub fn reset_text(at: Option<i64>) -> String {
    let Some(at) = at.and_then(|at| time::OffsetDateTime::from_unix_timestamp(at / 1000).ok())
    else {
        return "its weekly reset".into();
    };
    // Windows supplies its current local offset; a failed lookup stays explicit and deterministic in UTC.
    let at = at.to_offset(time::UtcOffset::local_offset_at(at).unwrap_or(time::UtcOffset::UTC));
    format!("{} {:02}:{:02}", at.weekday(), at.hour(), at.minute())
}

/// The week share from which an account with a usable limit reset is ranked by that reset instead of its week.
const NEARLY_USED_UP: f64 = 90.0;
/// The 5-hour share from which an account takes no work until that window resets.
const FIVE_HOUR_FULL: f64 = 90.0;

/// Whether a limit reset gives a week back: a full or weekly reset (Claude's "Full reset" and "Weekly reset",
/// Codex's "Full reset (Weekly + 5 hr)"). A 5-hour reset, a Fable reset (only what Fable used) or one without a name
/// does not count.
fn refills_week(reset: &Reset) -> bool {
    reset
        .title
        .as_deref()
        .is_some_and(|t| t.starts_with("Full reset") || t == "Weekly reset")
}

/// The account's soonest usable reset that gives its week back and expires before its week resets, when its week is
/// at [`NEARLY_USED_UP`] or more: what ranks it then.
fn reset_to_use<'a>(
    reading: Option<&'a AccountReading>,
    weekly: Option<&Limit>,
    week: f64,
    now: i64,
) -> Option<&'a Reset> {
    let week_resets = weekly.and_then(|l| l.resets_at).filter(|at| *at > now)?;
    if week < NEARLY_USED_UP {
        return None;
    }
    reading?
        .resets
        .iter()
        .filter(|r| r.id.is_some() && refills_week(r))
        .filter(|r| r.expires_at.is_some_and(|at| at > now && at < week_resets))
        .min_by_key(|r| r.expires_at)
}

/// One account in the ranking.
struct Entry<'a> {
    account_id: &'a str,
    /// `None`: no weekly limit, ranked after every account with one.
    deadline: Option<i64>,
    /// Its room: what is left of its week (main: up to its hand-off point), or of its 5-hour window without a weekly
    /// limit; a reset gives a whole week.
    room: f64,
    /// Settings order, main first.
    order: usize,
    reset: Option<&'a Reset>,
}

/// Its weekly reset, or a fresh week from now when there is none ahead (no reading, or a reset that has passed).
fn week_deadline(weekly: Option<&Limit>, now: i64) -> i64 {
    weekly
        .and_then(|l| l.resets_at)
        .filter(|at| *at > now)
        .unwrap_or(now.saturating_add(WEEK_MS))
}

/// Earliest deadline first, the same hour counting as a tie; then more room; then settings order, main first.
fn rank(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        let hour = |e: &Entry| e.deadline.map(|d| d.div_euclid(HOUR_MS));
        a.deadline
            .is_none()
            .cmp(&b.deadline.is_none())
            .then_with(|| hour(a).cmp(&hour(b)))
            .then_with(|| b.room.total_cmp(&a.room))
            .then_with(|| a.order.cmp(&b.order))
    });
}

pub fn decide(settings: &Settings, readings: &Readings, now: i64) -> Decision {
    let mut entries = Vec::new();
    let mut listed = Vec::new();
    for (k, account) in settings.accounts.iter().filter(|a| a.enabled).enumerate() {
        let reading = readings.accounts.get(&account.id);
        let usage = reading.and_then(|r| r.usage.as_ref());
        let five = usage
            .and_then(|u| used_now(u.five_hour.as_ref(), now))
            .unwrap_or(0.0);
        let weekly = usage.and_then(|u| u.weekly.as_ref());
        let week = used_now(weekly, now).unwrap_or(0.0);
        let login = login_skip(settings, account, reading);
        let skip = login.or(if five >= FIVE_HOUR_FULL {
            Some(Skip::NearFiveHourLimit)
        } else if week >= 100.0 {
            Some(Skip::WeekUsedUp)
        } else {
            None
        });
        listed.push(Listed {
            account_id: account.id.clone(),
            skip,
        });
        if login.is_some() {
            continue;
        }
        if let Some(reset) = reset_to_use(reading, weekly, week, now) {
            entries.push(Entry {
                account_id: &account.id,
                deadline: reset.expires_at,
                room: 100.0,
                order: k + 1,
                reset: Some(reset),
            });
            continue;
        }
        if skip.is_some() {
            continue;
        }
        // No reading: zero usage and a full week.
        let has_week = usage.is_none() || weekly.is_some();
        entries.push(Entry {
            account_id: &account.id,
            deadline: has_week.then(|| week_deadline(weekly, now)),
            room: if has_week { 100.0 - week } else { 100.0 - five },
            order: k + 1,
            reset: None,
        });
    }
    let main = readings.accounts.get(MAIN_ID);
    let usage = main.and_then(|r| r.usage.as_ref());
    let known = matches!(
        readings.main_login.sign_in.state,
        SignInState::SignedIn | SignInState::Expired
    ) && main.is_some_and(|r| {
        r.identity_change.is_none()
            && !matches!(
                r.error,
                Some(UsageError::SignedOut | UsageError::IdentityChanged)
            )
    });
    let weekly = usage.and_then(|u| u.weekly.as_ref());
    let week = known.then(|| used_now(weekly, now)).flatten();
    let five = usage.and_then(|u| used_now(u.five_hour.as_ref(), now));
    let remaining = reserve_now(
        settings.reserve,
        settings.work_hours.as_ref(),
        weekly.and_then(|l| l.resets_at),
        now,
    );
    let hand_off_at = hand_off(remaining);
    let five_full = five.is_some_and(|five| five >= FIVE_HOUR_FULL);
    let main_reset = week.and_then(|week| reset_to_use(main, weekly, week, now));
    if let Some(reset) = main_reset {
        entries.push(Entry {
            account_id: MAIN_ID,
            deadline: reset.expires_at,
            room: 100.0,
            order: 0,
            reset: Some(reset),
        });
    } else if let Some(week) = week.filter(|w| *w < f64::from(hand_off_at) && !five_full) {
        // Main counts only up to its hand-off point: above what it keeps for the user.
        entries.push(Entry {
            account_id: MAIN_ID,
            deadline: Some(week_deadline(weekly, now)),
            room: f64::from(hand_off_at) - week,
            order: 0,
            reset: None,
        });
    }
    let main_ranked = entries
        .iter()
        .any(|e| e.account_id == MAIN_ID && e.reset.is_none());
    rank(&mut entries);
    // A reset ranked first is suggested; the work goes to the first account ranked by its own allowance meanwhile.
    let suggested_reset = entries.first().and_then(|e| {
        let reset = e.reset?;
        Some(Suggested {
            account_id: e.account_id.to_owned(),
            reset_id: reset.id.clone()?,
            title: reset.title.clone()?,
            expires_at: reset.expires_at?,
        })
    });
    let pick = entries
        .iter()
        .find(|e| e.reset.is_none())
        .map(|e| e.account_id);
    let delegated = entries
        .iter()
        .find(|e| e.reset.is_none() && e.account_id != MAIN_ID)
        .map(|e| Target {
            account_id: e.account_id.to_owned(),
        });
    let main_works_itself = pick.is_none_or(|p| p == MAIN_ID);
    let why = if delegated.is_none() {
        Why::NoOther
    } else if main_works_itself {
        Why::MainSoonest
    } else if week.is_none() {
        Why::MainUnknown
    } else if five_full {
        Why::MainFiveHourFull
    } else if main_ranked {
        Why::Soonest
    } else {
        Why::MainKeeps
    };
    let order = entries
        .iter()
        .map(|e| Ranked {
            account_id: e.account_id.to_owned(),
            deadline: e.deadline,
            reset: e.reset.is_some(),
        })
        .collect();
    Decision {
        main_works_itself,
        delegated,
        why,
        hand_off_at,
        order,
        suggested_reset,
        accounts: listed,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guidance {
    pub text: String,
    pub key: String,
}

/// Exact official command. Both variables apply only to this delegated run, and are restored even on failure.
pub fn command(account: &AccountConfig, shell: Shell, parent: Option<&str>) -> String {
    let parent = parent.filter(|id| crate::pipe::valid_session_id(id));
    let var = account.provider.folder_var();
    let folder = shell.literal(&account.folder.to_string_lossy());
    let run = match account.provider {
        // Each CLI's own unattended mode, so a delegated run can edit inside the project within its limits.
        Provider::Claude => "claude -p --permission-mode auto --name \"<name>\" \"<task>\"",
        Provider::Codex => "codex exec --approve-for-me \"<task>\"",
    };
    match shell {
        Shell::Bash => format!(
            "{}{}={folder} {run}",
            parent.map_or_else(String::new, |id| format!(
                "WARDEN_PARENT_SESSION={} ",
                shell.literal(id)
            )),
            var
        ),
        Shell::PowerShell => {
            let save = parent.map_or_else(String::new, |_| {
                " $parentWas = $env:WARDEN_PARENT_SESSION;".into()
            });
            let set = parent.map_or_else(String::new, |id| {
                format!(" $env:WARDEN_PARENT_SESSION = {};", shell.literal(id))
            });
            let restore = parent.map_or_else(String::new, |_| {
                " $env:WARDEN_PARENT_SESSION = $parentWas;".into()
            });
            format!(
                "& {{ $was = $env:{var};{save} try {{ $env:{var} = {folder};{set} {run} }} finally {{ $env:{var} = $was;{restore} }} }}"
            )
        }
    }
}

/// Told to main whenever its advice lists a command.
pub const BACKGROUND_RUNS: &str = "Run a delegated command in the background and read its result when it finishes; a long run in the foreground is cut off.";

/// Told with it: a tool that starts a CLI (a review script) runs on a Warden account too, never on a CLI's default folder.
pub const STARTED_FOR_YOU: &str = "Anything you start that runs Claude or Codex for you (a review script, say) is delegated work too: run it with the same variables as that account's command above.";

/// Told with them when a Claude command is listed: what to put in its placeholders.
pub const NAME_AND_TASK: &str =
    "Replace `<name>` with a few words saying what the run does, and `<task>` with the task.";

/// Told with them when only Codex commands are listed (Codex's command has no name).
pub const TASK: &str = "Replace `<task>` with the task.";

pub fn guidance(
    settings: &Settings,
    readings: &Readings,
    session_account: Option<&str>,
    claude_shell: ClaudeShell,
    parent: Option<&str>,
    now: i64,
) -> Guidance {
    if session_account != Some(MAIN_ID) || readings.identity_changed(MAIN_ID) {
        return Guidance {
            text: String::new(),
            key: String::new(),
        };
    }
    let decision = decide(settings, readings, now);
    let shell = Shell::of(Provider::Claude, claude_shell);
    let pick = decision
        .delegated
        .as_ref()
        .and_then(|target| settings.account(&target.account_id));
    let mut text = match pick {
        None => "Warden: no other account can take delegated work now, so work in this session. If the user names an account, run it there.".to_owned(),
        Some(account) if decision.main_works_itself => format!(
            "Warden: work in this session: main's week resets soonest of the accounts with room. If the user asks to delegate, Warden's pick is {}. The user can always choose: work here, or run on whichever account they name.",
            account.label
        ),
        Some(account) => {
            let main_why = match decision.why {
                Why::MainFiveHourFull => "main's 5-hour window is full",
                Why::MainUnknown => "main's usage is unknown",
                Why::MainKeeps => "main keeps the rest of its week for the user",
                _ => "another account's week resets sooner than main's",
            };
            let pick_why = if decision
                .order
                .iter()
                .any(|r| r.account_id == account.id && r.deadline.is_some())
            {
                "its week resets soonest of the accounts with room"
            } else {
                "no account with a weekly limit has room, and it has none"
            };
            format!(
                "Warden: main is handing work off now ({main_why}), so run each task on {} (Warden's pick: {pick_why}); here, only talk to the user and check results. Whenever the user asks, work in this session instead, or run on the account they name.",
                account.label
            )
        }
    };
    // A reset to use: only the user uses it, in Warden, so the session tells them. No expiry, so the advice does not
    // change with the clock.
    if let Some(s) = &decision.suggested_reset {
        let label = if s.account_id == MAIN_ID {
            Some(settings.main.label.as_str())
        } else {
            settings.account(&s.account_id).map(|a| a.label.as_str())
        };
        if let Some(label) = label {
            text.push_str(&format!(
                "\nTell the user: Warden suggests using {label}'s {} on Warden's Overview, as it expires before {label}'s week resets. Once it is used, {label} takes the work; until then, work goes as above.",
                s.title
            ));
        }
    }
    // Every switched-on account, so the user can choose any of them: the pick, the others, and the skipped ones with
    // why. An account holding someone else's login (or none) gets no command, so no work ever runs on it.
    let mut lines = String::new();
    let mut any_command = false;
    let mut any_claude_command = false;
    for listed in &decision.accounts {
        let Some(account) = settings.account(&listed.account_id) else {
            continue;
        };
        let label = &account.label;
        let line = match listed.skip {
            None if pick.is_some_and(|p| p.id == account.id) => {
                format!(
                    "- {label} (Warden's pick): {}",
                    command(account, shell, parent)
                )
            }
            None => format!("- {label}: {}", command(account, shell, parent)),
            Some(skip) if skip.can_run() => format!(
                "- {label} (skipped: {}): {}",
                skip.words(),
                command(account, shell, parent)
            ),
            Some(skip) => format!("- {label} (skipped: {})", skip.words()),
        };
        let can_run = listed.skip.is_none_or(Skip::can_run);
        any_command |= can_run;
        any_claude_command |= can_run && account.provider == Provider::Claude;
        lines.push('\n');
        lines.push_str(&line);
    }
    // Claude Code stops waiting for a foreground command after a few minutes, which would cut a delegated task off.
    if any_command {
        text.push('\n');
        text.push_str(BACKGROUND_RUNS);
        text.push('\n');
        text.push_str(STARTED_FOR_YOU);
        text.push('\n');
        // Codex's command has no name, so only the task is explained when no Claude command is listed.
        text.push_str(if any_claude_command {
            NAME_AND_TASK
        } else {
            TASK
        });
    }
    text.push_str(&lines);
    // Percentages, reading age and reset time never cause repeated advice while the actual decision stays put: the
    // text holds none of them, so its key changes only with the pick, the mode, a skip, the accounts or a command.
    let key = format!("{shell:?}:{text}");
    Guidance {
        text: text + "\n",
        key,
    }
}

pub fn readings_are_stale(settings: &Settings, readings: &Readings, now: i64) -> bool {
    let limit = (i64::from(settings.usage_interval_min) + 1) * 60_000;
    readings
        .checked_at
        .is_none_or(|t| now - t > limit || t > now)
}

/// The shell the session's AI runs commands in: Claude Code runs them in Git Bash when it finds one and in PowerShell
/// otherwise (the same answer as for its hooks), Codex on Windows in PowerShell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    /// PowerShell, the folder variable set only while the CLI runs and put back afterwards, so the session's later
    /// commands keep its own account (Claude Code without Git Bash, and Codex).
    PowerShell,
}

impl Shell {
    pub fn of(provider: Provider, claude_shell: ClaudeShell) -> Shell {
        match (provider, claude_shell) {
            (Provider::Claude, ClaudeShell::Bash) => Shell::Bash,
            (Provider::Claude, ClaudeShell::PowerShell) | (Provider::Codex, _) => Shell::PowerShell,
        }
    }

    /// A literal string in this shell: single quotes, which keep backslashes and spaces as they are. PowerShell takes
    /// its typographic single quotes as quotes too, so there a quote of any kind inside is doubled.
    fn literal(self, s: &str) -> String {
        match self {
            Shell::Bash => format!("'{}'", s.replace('\'', "'\\''")),
            Shell::PowerShell => {
                let mut out = String::from("'");
                for c in s.chars() {
                    if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
                        out.push(c);
                    }
                    out.push(c);
                }
                out.push('\'');
                out
            }
        }
    }
}
#[cfg(test)]
mod tests;
