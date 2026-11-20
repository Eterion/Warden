// How readings are put into words. Pure functions, so they are unit tested.

import type { Account, Limit, Main, Problem, Provider, State, Usage, UsageError, WorkHours } from './api';

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** "just now", "1 min ago", "3 h ago", "2 days ago". */
export function ago(at: number, now: number): string {
  const d = Math.max(0, now - at);
  if (d < MINUTE) return 'just now';
  if (d < HOUR) return `${Math.floor(d / MINUTE)} min ago`;
  if (d < DAY) return `${Math.floor(d / HOUR)} h ago`;
  const days = Math.floor(d / DAY);
  return days === 1 ? '1 day ago' : `${days} days ago`;
}

// The UI stays in English whatever the Windows region is: 'en', with a 24-hour clock.
const HOUR_MINUTE = new Intl.DateTimeFormat('en', { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
const WEEKDAY_TIME = new Intl.DateTimeFormat('en', {
  weekday: 'short',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
});
const DAY_MONTH = new Intl.DateTimeFormat('en', { day: 'numeric', month: 'short' });
const DAY_MONTH_YEAR = new Intl.DateTimeFormat('en', { day: 'numeric', month: 'short', year: 'numeric' });

/** "12:10" when it is today, "Fri 05:00" within the week, "14 Nov" further away. Local time, 24-hour. */
export function resetTime(at: number, now: number): string {
  const t = new Date(at);
  const n = new Date(now);
  if (t.toDateString() === n.toDateString()) return HOUR_MINUTE.format(t);
  if (at - now < 6 * DAY && at > now) return WEEKDAY_TIME.format(t);
  // 'en' writes "Nov 14"; the day comes first here.
  const parts = DAY_MONTH.formatToParts(t);
  const part = (type: Intl.DateTimeFormatPartTypes) => parts.find((p) => p.type === type)?.value ?? '';
  return `${part('day')} ${part('month')}`;
}

/** Whether a reset expires within the warning window: at most `warnDays` days (of 24 hours) from now. */
export function expiresSoon(at: number | null, now: number, warnDays: number): boolean {
  return at !== null && at > now && at - now <= warnDays * DAY;
}

/** Calendar days from `now`'s local date to `at`'s: 0 today, 1 tomorrow. */
function calendarDays(at: number, now: number): number {
  const day = (ms: number) => {
    const d = new Date(ms);
    return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  };
  // Rounded: a day across a change of summer time is 23 or 25 hours.
  return Math.round((day(at) - day(now)) / DAY);
}

/**
 * "Expires 10 Dec, 23:00", with the year when it is not this year's. Within the warning window (`warnDays`), how soon
 * too: "Expires in 2 days, 10 Dec, 23:00", "Expires tomorrow, 23:00", "Expires today, 23:00". Local time, 24-hour.
 */
export function expiryText(at: number | null, now: number, warnDays = 0): string {
  if (at === null) return 'Does not expire';
  const t = new Date(at);
  const clock = HOUR_MINUTE.format(t);
  const parts = DAY_MONTH_YEAR.formatToParts(t);
  const part = (type: Intl.DateTimeFormatPartTypes) => parts.find((p) => p.type === type)?.value ?? '';
  const year = t.getFullYear() === new Date(now).getFullYear() ? '' : ` ${part('year')}`;
  const date = `${part('day')} ${part('month')}${year}`;
  if (expiresSoon(at, now, warnDays)) {
    const days = calendarDays(at, now);
    if (days <= 0) return `Expires today, ${clock}`;
    if (days === 1) return `Expires tomorrow, ${clock}`;
    return `Expires in ${days} days, ${date}, ${clock}`;
  }
  return `Expires ${date}, ${clock}`;
}

export interface ResetRow {
  /**
   * Stays with the row while the list changes: the account, the reset's id and which of the resets with that id it is
   * (a Claude grant lists one per reset left), so using one takes away the row of the last of them.
   */
  key: string;
  account: Account;
  /** What using it needs; `null`: it cannot be used. */
  id: string | null;
  title: string;
  expiresAt: number | null;
  /** Expires within the warning window. */
  soon: boolean;
}

/**
 * Every reset the accounts can use, one row each, soonest to expire first (those that never expire last); one past its
 * expiry is gone. `soon`: it expires within `warnDays` days.
 */
export function resetRows(accounts: Account[], now: number, warnDays: number): ResetRow[] {
  return accounts
    .flatMap((a) => {
      const seen = new Map<string, number>();
      return a.resets.map((r, i) => {
        const n = r.id === null ? i : (seen.get(r.id) ?? 0);
        if (r.id !== null) seen.set(r.id, n + 1);
        return {
          key: r.id === null ? `${a.id}:#${n}` : `${a.id}:${r.id}:${n}`,
          account: a,
          id: r.id,
          title: r.title ?? 'Reset',
          expiresAt: r.expiresAt,
          soon: expiresSoon(r.expiresAt, now, warnDays),
        };
      });
    })
    .filter((r) => r.expiresAt === null || r.expiresAt > now)
    .sort((x, y) => (x.expiresAt ?? Infinity) - (y.expiresAt ?? Infinity));
}

/**
 * The reset rows the Overview's filter lets through: none of an account in `hidden`, and with `soonestOnly` only each
 * account's first (the rows come soonest first, so that is its soonest).
 */
export function filterResets(rows: ResetRow[], hidden: string[], soonestOnly: boolean): ResetRow[] {
  const seen = new Set<string>();
  return rows.filter((r) => {
    if (hidden.includes(r.account.id)) return false;
    if (!soonestOnly) return true;
    if (seen.has(r.account.id)) return false;
    seen.add(r.account.id);
    return true;
  });
}

/**
 * What using a reset does, as the confirmation says it, from its short name: a full reset (Codex's "Full reset (Weekly
 * + 5 hr)" too) refills both limits; a 5-hour or weekly reset one of them; a Fable reset gives back what Fable used.
 */
export function resetEffect(title: string): string {
  if (title.startsWith('Full reset')) return 'It refills its 5-hour and weekly limits now.';
  if (title === '5-hour reset') return 'It refills its 5-hour limit now.';
  if (title === 'Weekly reset') return 'It refills its weekly limit now.';
  if (title === 'Fable reset') return 'It gives what Fable used this week back to its weekly limit.';
  return 'It refills its limits now.';
}

export type Level = 'low' | 'mid' | 'high';

/** Usage bars: green below 50%, orange from 50%, red from 80%. */
export function level(percent: number): Level {
  return percent >= 80 ? 'high' : percent >= 50 ? 'mid' : 'low';
}

export interface LimitView {
  percent: number;
  /** "61% used" */
  used: string;
  /** "resets 12:10", "reset at 12:10" once past, or "" */
  reset: string;
  level: Level;
}

/** A limit as shown: once its reset time has passed, the old percentage no longer holds, so it shows as reset. */
export function limitView(limit: Limit, now: number): LimitView {
  if (limit.resetsAt !== null && limit.resetsAt <= now) {
    return { percent: 0, used: '0% used', reset: `reset at ${resetTime(limit.resetsAt, now)}`, level: 'low' };
  }
  const percent = Math.round(Math.min(100, Math.max(0, limit.usedPercent)));
  return {
    percent,
    used: `${percent}% used`,
    reset: limit.resetsAt === null ? '' : `resets ${resetTime(limit.resetsAt, now)}`,
    level: level(percent),
  };
}

// Whole dollars without decimals ("$50"), anything else with cents ("$12.40"). The API gives no currency, so USD.
const COUNT = new Intl.NumberFormat('en');

/** A count as the UI shows it: English digits and grouping, whatever the Windows region is ("1,250"). */
export function count(n: number): string {
  return COUNT.format(n);
}

const WHOLE_DOLLARS = new Intl.NumberFormat('en', { style: 'currency', currency: 'USD', maximumFractionDigits: 0 });
const DOLLARS_AND_CENTS = new Intl.NumberFormat('en', { style: 'currency', currency: 'USD', minimumFractionDigits: 2 });

function dollars(cents: number): string {
  return (cents % 100 === 0 ? WHOLE_DOLLARS : DOLLARS_AND_CENTS).format(cents / 100);
}

/**
 * Claude's extra usage, in dollars ("$12.40 of $50"; the endpoint gives cents), only once some of it is spent:
 * `null` while nothing is, even with extra usage switched on.
 */
export function extraText(provider: Provider, usage: Usage | null): string | null {
  const extra = usage?.extra;
  const used = extra?.usedCredits ?? 0;
  if (provider !== 'claude' || !extra || used <= 0) return null;
  return extra.monthlyLimit !== null ? `${dollars(used)} of ${dollars(extra.monthlyLimit)}` : `${dollars(used)} used`;
}

/** Why there is no fresh reading, in a few quiet words; `null` when the reading is current. */
export function usageProblem(error: UsageError | null): string | null {
  if (!error) return null;
  switch (error.kind) {
    case 'signedOut':
      return 'Signed out';
    case 'loginExpired':
      // A Warden account's idle login is shown as grey bars and its age (`readingNote`); main's has its own words.
      return null;
    case 'identityChanged':
      return 'Kept apart until you confirm the new sign-in';
    case 'noLogs':
      return 'No Codex sessions yet';
    case 'failed':
      return `Could not read: ${error.message}`;
  }
}

/** Claude's readings count as old after this many usage intervals. */
export const STALE_INTERVALS = 3;

/** Which of a Codex reading's limits have reset since it was taken: "5-hour limit has reset", or null. */
function resetSinceReading(usage: Usage, now: number): string | null {
  const passed = (l: Limit | null) => l?.resetsAt != null && l.resetsAt > usage.readAt && l.resetsAt <= now;
  const five = passed(usage.fiveHour);
  const week = passed(usage.weekly);
  if (five && week) return 'Both limits have reset';
  if (five) return '5-hour limit has reset';
  if (week) return 'Weekly limit has reset';
  return null;
}

/**
 * The note under an account's limits, only when it matters: a problem reading usage (amber), and in grey either
 * "Read 3 h ago" once a Claude reading is older than a few usage intervals, or, for Codex (read from its logs, so
 * an idle account's reading is naturally old), that a limit has reset since its last reading. No Codex sessions yet
 * is plain grey news.
 */
export function readingNote(
  account: Pick<Account, 'provider' | 'usage' | 'usageError'> & { id?: string },
  intervalMin: number,
  now: number,
): { problem: string | null; age: string | null; idle?: true } | null {
  const err = account.usageError;
  // A Warden account whose login has expired: nothing to fix, so no amber, only how old its (still valid) reading is.
  if (account.id !== 'main' && err?.kind === 'loginExpired') {
    return account.usage ? { problem: null, age: `Read ${ago(account.usage.readAt, now)}`, idle: true } : null;
  }
  let problem = err && err.kind !== 'noLogs' ? usageProblem(err) : null;
  if (account.id === 'main' && err) {
    if (err.kind === 'loginExpired') problem = "Sign in again for main's usage";
    else if (err.kind === 'signedOut') problem = "Sign in for main's usage";
    else if (err.kind === 'identityChanged') problem = "Restore main's sign-in to read usage";
  }
  const usage = account.usage;
  let age: string | null = null;
  if (usage && account.provider === 'codex') age = resetSinceReading(usage, now);
  else if (usage && now - usage.readAt > STALE_INTERVALS * intervalMin * MINUTE) age = `Read ${ago(usage.readAt, now)}`;
  if (!age && err?.kind === 'noLogs') age = usageProblem(err);
  return problem || age ? { problem, age } : null;
}

/** The tooltip on an idle login's age. */
export const IDLE_LOGIN =
  'Its login is idle. The last reading still counts, and Claude renews the login the next time this account works.';

/** A Warden account whose login has expired: its bars turn grey (main's own expired sign-in stays amber). */
export function loginIdle(account: Pick<Account, 'usageError'> & { id?: string }): boolean {
  return account.id !== 'main' && account.usageError?.kind === 'loginExpired';
}

/** "Claude app" for main; "Claude" or "Codex" for Warden's accounts. */
export function whereText(account: Pick<Account, 'provider'> & { id?: string }): string {
  if (account.id === 'main') return 'Claude app';
  return providerName(account.provider);
}

function providerName(p: Provider): string {
  return p === 'claude' ? 'Claude' : 'Codex';
}

/** A fix offered beside a problem in Settings › Accounts. */
export type Fix = 'signInMain' | 'signInMainAgain' | 'signIn' | 'accept' | 'undo' | 'scan';

export const FIX_LABEL: Record<Fix, string> = {
  signInMain: 'Sign in',
  signInMainAgain: 'Sign in again',
  signIn: 'Sign in',
  accept: 'Accept',
  undo: 'Undo',
  scan: 'Scan again',
};

/**
 * Something wrong with an account, in a few words, with its fixes. `tooltip` explains a changed sign-in; `identity`
 * marks what stands in place of the account's email line (a changed sign-in, and for main where to fix it).
 */
export interface RowProblem {
  text: string;
  tooltip?: string;
  identity?: true;
  fixes: Fix[];
}

/** What the Hooks column says about an account's hooks: always something, amber with Repair when it needs it. */
export interface HookCell {
  text: string;
  tone: 'ok' | 'attn' | 'quiet';
  repair: boolean;
}

export function hooksCell(account: Pick<Account, 'enabled' | 'hooks'>, hooksOff: boolean): HookCell {
  if (hooksOff) return { text: 'Off for this run', tone: 'quiet', repair: false };
  if (!account.enabled)
    return account.hooks === 'notInstalled'
      ? { text: 'Not installed', tone: 'quiet', repair: false }
      : { text: 'Still installed', tone: 'attn', repair: true };
  if (account.hooks === 'installed') return { text: 'Installed', tone: 'ok', repair: false };
  if (account.hooks === 'notTrusted') return { text: 'Not trusted', tone: 'attn', repair: true };
  return { text: 'Missing', tone: 'attn', repair: true };
}

function cliText(p: Problem): string | null {
  if (p.kind === 'cliMissing') return `The ${providerName(p.provider)} command line (${p.name}) was not found on PATH.`;
  if (p.kind === 'cliTooOld')
    return `The ${providerName(p.provider)} command line (${p.name} ${p.version}) is too old; update to ${p.needed} or later.`;
  return null;
}

/**
 * What Settings › Accounts says about an account, only when something is wrong: each problem in amber with its fix.
 * Main also carries the Claude app's own problems and Warden's sign-in for its usage; a missing or old CLI shows on
 * every row of its provider. An empty list while all is well.
 */
export function accountProblems(
  account: Account & { signIn?: Main['signIn'] },
  ctx: { all: Account[]; problems: Problem[] },
): RowProblem[] {
  const out: RowProblem[] = [];
  const isMain = account.id === 'main';
  if (isMain) {
    for (const p of ctx.problems) {
      if (p.kind === 'claudeAppMissing') out.push({ text: 'The Claude app was not found.', fixes: [] });
      if (p.kind === 'claudeAppSignedOut') out.push({ text: 'The Claude app is signed out.', fixes: [] });
    }
  }
  const change = account.identityChange;
  if (change) {
    const text = signedInAs(account, ctx.all) ?? '';
    const tooltip = expectedSignIn(account);
    if (isMain) {
      out.push({ text, tooltip, identity: true, fixes: [] });
      if (account.identity)
        out.push({ text: `Sign the Claude app back in as ${account.identity.email}`, identity: true, fixes: [] });
    } else out.push({ text, tooltip, identity: true, fixes: change.now ? ['accept', 'undo'] : ['signIn'] });
  } else if (!isMain && account.usageError?.kind === 'signedOut') out.push({ text: 'Signed out', fixes: ['signIn'] });
  if (isMain && account.signIn) {
    const s = account.signIn;
    if (s.state === 'signedOut') out.push({ text: "Main's usage unknown", fixes: ['signInMain'] });
    if (s.state === 'expired')
      out.push({ text: "Warden's sign-in for main's usage has expired.", fixes: ['signInMainAgain'] });
    if (s.state === 'otherAccount')
      out.push({
        text: `Warden is signed in as ${s.email ?? 'another account'} for main's usage.`,
        fixes: ['signInMainAgain'],
      });
  }
  if (account.sameAsMain) out.push({ text: 'Same account as main', fixes: [] });
  for (const p of ctx.problems) {
    const text = 'provider' in p && p.provider === account.provider ? cliText(p) : null;
    if (text) out.push({ text, fixes: ['scan'] });
  }
  return out;
}

/** "Signed in as me@home.example", naming the other account when the new identity is one Warden knows. */
export function signedInAs(account: Account, all: Account[]): string | null {
  const change = account.identityChange;
  if (!change) return null;
  if (!change.now) return 'Signed out';
  const other = change.knownAccountId ? all.find((a) => a.id === change.knownAccountId) : undefined;
  return `Signed in as ${change.now.email}${other ? `, which is ${other.label}` : ''}`;
}

/** The tooltip on a changed sign-in: the email the account expects. */
export function expectedSignIn(account: Pick<Account, 'identity'>): string {
  return account.identity ? `Expected ${account.identity.email}` : 'Expected the account it was added with';
}

/** Main's crown has a spoken name. */
export function badgeLabel(account: Pick<Account, 'id'>): string {
  return account.id === 'main' ? 'Main account' : '';
}

/** The folder a new account would get, as the server makes it (detect::new_account_folder). */
export function newFolderPreview(home: string | null, provider: Provider, label: string): string {
  if (!home) return '';
  const prefix = provider === 'claude' ? 'claude' : 'codex';
  let slug = label
    .toLowerCase()
    .replace(/[^a-z0-9]/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '');
  if (slug.startsWith(prefix)) slug = slug.slice(prefix.length).replace(/^-|-$/g, '');
  // Windows paths: a backslash unless the home is written with forward slashes only.
  const sep = home.includes('/') && !home.includes('\\') ? '/' : '\\';
  return `${home.replace(/[\\/]$/, '')}${sep}.warden${sep}${prefix}-${slug || 'account'}`;
}

/** A time of day from minutes after midnight: "09:00", "24:00" (24-hour, as the UI writes times). */
export function clock(minutes: number): string {
  const h = Math.floor(minutes / 60);
  return `${String(h).padStart(2, '0')}:${String(minutes % 60).padStart(2, '0')}`;
}

/** "09:00–18:00" */
export function workHoursText(w: Pick<WorkHours, 'from' | 'to'>): string {
  return `${clock(w.from)}–${clock(w.to)}`;
}

/** The weekdays' short names, Monday first (ISO 1 to 7), in English whatever the Windows region. */
export const WEEKDAYS: string[] = (() => {
  const short = new Intl.DateTimeFormat('en', { weekday: 'short' });
  // 2026-11-09 is a Monday.
  return Array.from({ length: 7 }, (_, k) => short.format(new Date(2026, 10, 9 + k)));
})();

const WEEK_MS = 7 * DAY;

/**
 * Main's hand-off point for a reserve, as the server works it out (`routing::reserve_now`): the reserve shrinks with
 * what is left of main's week before its weekly reset, the time left or, with work hours, the work hours left (in
 * local time); 100 less that, rounded down. Shown while the reserve slider is dragged, before the server answers.
 */
export function handOffPreview(
  reserve: number,
  workHours: WorkHours | null,
  resetsAt: number | null,
  now: number,
): number {
  let kept = reserve;
  if (resetsAt !== null) {
    const end = Math.min(resetsAt, now + WEEK_MS);
    if (!workHours) kept = (reserve * Math.min(Math.max(end - now, 0), WEEK_MS)) / WEEK_MS;
    else {
      const week = workHours.days.length * (workHours.to - workHours.from) * MINUTE;
      if (week > 0) kept = reserve * Math.min(Math.max(workMsBetween(workHours, now, end) / week, 0), 1);
    }
  }
  return Math.min(100, Math.max(0, Math.floor(100 - kept)));
}

/** How much of the work windows falls between `start` and `end`, in local time (`Date`, as `routing::work_ms_between`). */
export function workMsBetween(w: WorkHours, start: number, end: number): number {
  if (end <= start) return 0;
  let total = 0;
  const day = new Date(start);
  day.setHours(0, 0, 0, 0);
  for (; day.getTime() <= end; day.setDate(day.getDate() + 1)) {
    const iso = day.getDay() === 0 ? 7 : day.getDay();
    if (!w.days.includes(iso)) continue;
    const from = new Date(day.getFullYear(), day.getMonth(), day.getDate(), 0, w.from).getTime();
    const to = new Date(day.getFullYear(), day.getMonth(), day.getDate(), 0, w.to).getTime();
    total += Math.max(0, Math.min(to, end) - Math.max(from, start));
  }
  return total;
}

/** The status bar's one item: the account that gets the work now, "until N%" while it is main, and why. */
export function workGoesTo(state: Pick<State, 'delegation' | 'main' | 'accounts'>): {
  account: Account | null;
  until: string | null;
  reason: string;
} {
  const d = state.delegation;
  if (d.mainWorksItself)
    return { account: state.main, until: d.delegated ? `until ${d.handOffAt}%` : null, reason: d.mainReason };
  const other = d.delegated ? (state.accounts.find((a) => a.id === d.delegated!.accountId) ?? null) : null;
  return { account: other, until: null, reason: `${d.mainReason} ${d.delegatedReason}` };
}
