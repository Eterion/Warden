import { describe, expect, it } from 'vitest';
import type { Account, Main, Problem, State } from './api';
import {
  IDLE_LOGIN,
  WEEKDAYS,
  accountProblems,
  ago,
  clock,
  extraText,
  handOffPreview,
  hooksCell,
  level,
  loginIdle,
  workMsBetween,
  limitView,
  newFolderPreview,
  readingNote,
  expiresSoon,
  count,
  expiryText,
  resetEffect,
  filterResets,
  resetRows,
  resetTime,
  signedInAs,
  usageProblem,
  whereText,
  workGoesTo,
  workHoursText,
} from './format';
import { NOW, account } from './test/state';

describe('Settings › Accounts wording', () => {
  const main = (extra: Partial<Account> = {}, signIn: Main['signIn']['state'] = 'signedIn'): Main => ({
    ...account({ id: 'main', label: 'Main', identity: { email: 'main@example.com', organization: null }, ...extra }),
    signIn: { state: signIn, email: signIn === 'otherAccount' ? 'other@example.com' : 'main@example.com' },
  });
  const ctx = (problems: Problem[] = [], all: Account[] = []) => ({ all, problems });
  const said = (a: Account, c = ctx()) => accountProblems(a, c).map((p) => [p.text, p.fixes.join(' ')]);

  it('names where an account lives: the Claude app for main, the provider for the others', () => {
    expect(whereText({ id: 'main', provider: 'claude' })).toBe('Claude app');
    expect(whereText({ provider: 'claude' })).toBe('Claude');
    expect(whereText({ provider: 'codex' })).toBe('Codex');
  });

  it('says nothing while all is well', () => {
    expect(said(main())).toEqual([]);
    expect(said(account({ id: 'a', label: 'A' }))).toEqual([]);
  });

  it("offers Warden's sign-in for main's usage", () => {
    expect(said(main({}, 'signedOut'))).toEqual([["Main's usage unknown", 'signInMain']]);
    expect(said(main({}, 'expired'))).toEqual([["Warden's sign-in for main's usage has expired.", 'signInMainAgain']]);
    expect(said(main({}, 'otherAccount'))).toEqual([
      ["Warden is signed in as other@example.com for main's usage.", 'signInMainAgain'],
    ]);
  });

  it("puts the Claude app's own problems on main only, with no fix", () => {
    const c = ctx([{ kind: 'claudeAppMissing' }, { kind: 'claudeAppSignedOut' }]);
    expect(said(main(), c)).toEqual([
      ['The Claude app was not found.', ''],
      ['The Claude app is signed out.', ''],
    ]);
    expect(said(account({ id: 'a', label: 'A' }), c)).toEqual([]);
  });

  it("sends main's changed sign-in back to the Claude app, and offers Accept and Undo on the others", () => {
    const change = { now: { email: 'x@example.com', organization: null }, knownAccountId: null };
    expect(accountProblems(main({ identityChange: change }), ctx())).toEqual([
      { text: 'Signed in as x@example.com', tooltip: 'Expected main@example.com', identity: true, fixes: [] },
      { text: 'Sign the Claude app back in as main@example.com', identity: true, fixes: [] },
    ]);
    const a = account({ id: 'a', label: 'A', identity: { email: 'a@example.com', organization: null } });
    expect(said({ ...a, identityChange: change })).toEqual([['Signed in as x@example.com', 'accept undo']]);
    expect(said({ ...a, identityChange: { now: null, knownAccountId: null } })).toEqual([['Signed out', 'signIn']]);
    expect(said({ ...a, usageError: { kind: 'signedOut' } })).toEqual([['Signed out', 'signIn']]);
  });

  it('says an account signed in as main takes no fix', () => {
    expect(said(account({ id: 'a', label: 'A', sameAsMain: true }))).toEqual([['Same account as main', '']]);
  });

  it("says how each account's hooks are in the Hooks column, and leaves them out of the row's problems", () => {
    const cell = (enabled: boolean, hooks: Account['hooks'], off = false) => {
      const c = hooksCell({ enabled, hooks }, off);
      return [c.text, c.tone, c.repair];
    };
    expect(cell(true, 'installed')).toEqual(['Installed', 'ok', false]);
    expect(cell(true, 'notTrusted')).toEqual(['Not trusted', 'attn', true]);
    expect(cell(true, 'notInstalled')).toEqual(['Missing', 'attn', true]);
    expect(cell(false, 'notInstalled')).toEqual(['Not installed', 'quiet', false]);
    expect(cell(false, 'installed')).toEqual(['Still installed', 'attn', true]);
    expect(cell(true, 'notInstalled', true)).toEqual(['Off for this run', 'quiet', false]);
    expect(said(account({ id: 'a', label: 'A', hooks: 'notInstalled' }))).toEqual([]);
  });

  it('shows a missing or old CLI on the rows of its provider, with Scan again', () => {
    const c = ctx([
      { kind: 'cliMissing', provider: 'claude', name: 'claude' },
      { kind: 'cliTooOld', provider: 'codex', name: 'codex', version: '0.1.0', needed: '1.0.0' },
    ]);
    expect(said(main(), c)).toEqual([['The Claude command line (claude) was not found on PATH.', 'scan']]);
    expect(said(account({ id: 'b', label: 'B', provider: 'codex' }), c)).toEqual([
      ['The Codex command line (codex 0.1.0) is too old; update to 1.0.0 or later.', 'scan'],
    ]);
  });
});

describe('work hours, the hand-off point and the status bar', () => {
  // Monday 2026-11-09 00:00 in this computer's local time.
  const monday = new Date(2026, 10, 9).getTime();
  const at = (day: number, h: number, m = 0) => new Date(2026, 10, 9 + day, h, m).getTime();
  const office = { days: [1, 2, 3, 4, 5], from: 540, to: 1080 };

  it('writes times of day and the weekdays as the UI does', () => {
    expect(clock(0)).toBe('00:00');
    expect(clock(570)).toBe('09:30');
    expect(clock(1440)).toBe('24:00');
    expect(workHoursText({ from: 540, to: 1080 })).toBe('09:00–18:00');
    expect(WEEKDAYS).toEqual(['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']);
  });

  it('works out the hand-off point as the server does, with and without work hours', () => {
    const reset = at(4, 5);
    // The plan's examples (25%, Mon to Fri 09:00 to 18:00, reset Friday 05:00).
    expect(handOffPreview(25, office, reset, at(0, 9))).toBe(80);
    expect(handOffPreview(25, office, reset, at(3, 16))).toBe(98);
    expect(handOffPreview(25, office, reset, at(0, 18))).toBe(handOffPreview(25, office, reset, at(0, 23)));
    expect(handOffPreview(25, office, reset, at(3, 18))).toBe(100);
    // No work hours: the time left; no reset known: the full reserve.
    expect(handOffPreview(25, null, monday + 7 * 86_400_000, monday)).toBe(75);
    expect(handOffPreview(25, null, monday + 3.5 * 86_400_000, monday)).toBe(87);
    expect(handOffPreview(25, office, null, monday)).toBe(75);
  });

  it('counts a repeated or missing hour as the server does (routing::tests)', () => {
    // Central Europe: the clocks go back on Sunday 2026-10-25 and forward on Sunday 2026-03-29.
    const zone = process.env.TZ;
    process.env.TZ = 'Europe/Prague';
    try {
      const sunday = { days: [7], from: 120, to: 240 };
      const back = Date.parse('2026-10-24T22:00:00Z');
      const forward = Date.parse('2026-03-28T23:00:00Z');
      expect(workMsBetween(sunday, back, back + 86_400_000)).toBe(3 * 3_600_000);
      expect(workMsBetween(sunday, forward, forward + 86_400_000)).toBe(3_600_000);
    } finally {
      if (zone === undefined) delete process.env.TZ;
      else process.env.TZ = zone;
    }
  });

  it('names the account that gets the work now, "until" while it is main, and why', () => {
    const main = { ...account({ id: 'main', label: 'Main' }), signIn: { state: 'signedIn' as const, email: null } };
    const second = account({ id: 's', label: 'Second' });
    const base = { main, accounts: [second] } as Pick<State, 'main' | 'accounts'>;
    const d = { mainReason: 'Main has room.', delegatedReason: 'Second has the most left.', handOffAt: 98 };
    expect(workGoesTo({ ...base, delegation: { ...d, mainWorksItself: true, delegated: { accountId: 's' } } })).toEqual(
      { account: main, until: 'until 98%', reason: 'Main has room.' },
    );
    expect(workGoesTo({ ...base, delegation: { ...d, mainWorksItself: true, delegated: null } }).until).toBeNull();
    expect(
      workGoesTo({ ...base, delegation: { ...d, mainWorksItself: false, delegated: { accountId: 's' } } }),
    ).toEqual({ account: second, until: null, reason: 'Main has room. Second has the most left.' });
  });

  it("shows an expired Warden login as idle, with its age and no amber, but not main's", () => {
    const usage = { readAt: NOW - 3 * 3_600_000, fiveHour: null, weekly: null, perModelWeekly: [], extra: null };
    const idle = { id: 'a', provider: 'claude' as const, usage, usageError: { kind: 'loginExpired' as const } };
    expect(loginIdle(idle)).toBe(true);
    expect(readingNote(idle, 5, NOW)).toEqual({ problem: null, age: 'Read 3 h ago', idle: true });
    expect(IDLE_LOGIN).toContain('The last reading still counts');
    expect(loginIdle({ ...idle, id: 'main' })).toBe(false);
    expect(readingNote({ ...idle, id: 'main' }, 5, NOW)?.problem).toBe("Sign in again for main's usage");
  });
});

const MIN = 60_000;
const HOUR = 60 * MIN;

describe('format', () => {
  it('says how old a reading is', () => {
    expect(ago(NOW - 10_000, NOW)).toBe('just now');
    expect(ago(NOW - 3 * MIN, NOW)).toBe('3 min ago');
    expect(ago(NOW - 5 * HOUR, NOW)).toBe('5 h ago');
    expect(ago(NOW - 25 * HOUR, NOW)).toBe('1 day ago');
    expect(ago(NOW - 72 * HOUR, NOW)).toBe('3 days ago');
    expect(ago(NOW + MIN, NOW)).toBe('just now');
  });

  it('writes reset times as a time today, a weekday this week, else a date', () => {
    expect(resetTime(NOW + 2 * HOUR + 10 * MIN, NOW)).toBe('12:10');
    expect(resetTime(NOW + 3 * 24 * HOUR, NOW)).toBe('Thu 10:00');
    expect(resetTime(NOW + 10 * 24 * HOUR, NOW)).toBe('19 Nov');
  });

  it('colours bars green below 50%, orange from 50%, red from 80%', () => {
    expect([level(49), level(50), level(79), level(80)]).toEqual(['low', 'mid', 'mid', 'high']);
  });

  it('shows a limit whose reset has passed as reset, not as its old percentage', () => {
    expect(limitView({ usedPercent: 61.4, resetsAt: NOW + HOUR }, NOW)).toMatchObject({
      percent: 61,
      used: '61% used',
      level: 'mid',
    });
    expect(limitView({ usedPercent: 95, resetsAt: NOW - MIN }, NOW)).toMatchObject({
      percent: 0,
      used: '0% used',
      reset: 'reset at 09:59',
    });
    expect(limitView({ usedPercent: 140, resetsAt: null }, NOW)).toMatchObject({ percent: 100, reset: '' });
  });

  it('writes extra usage in dollars from cents, only once some is spent', () => {
    const usage = { readAt: NOW, fiveHour: null, weekly: null, perModelWeekly: [] };
    expect(
      extraText('claude', {
        ...usage,
        extra: { enabled: true, usedCredits: 1240, monthlyLimit: 5000, usedPercent: 24.8 },
      }),
    ).toBe('$12.40 of $50');
    expect(
      extraText('claude', {
        ...usage,
        extra: { enabled: true, usedCredits: 700, monthlyLimit: null, usedPercent: null },
      }),
    ).toBe('$7 used');
    expect(
      extraText('claude', {
        ...usage,
        extra: { enabled: true, usedCredits: 123450, monthlyLimit: 200000, usedPercent: 61.7 },
      }),
    ).toBe('$1,234.50 of $2,000');
    expect(
      extraText('claude', {
        ...usage,
        extra: { enabled: false, usedCredits: null, monthlyLimit: null, usedPercent: null },
      }),
    ).toBeNull();
    expect(extraText('claude', null)).toBeNull();
    expect(extraText('codex', null)).toBeNull();
    expect(
      extraText('claude', {
        ...usage,
        extra: { enabled: true, usedCredits: 0, monthlyLimit: 5000, usedPercent: 0 },
      }),
    ).toBeNull();
  });

  it('puts each usage problem in a few quiet words', () => {
    expect(usageProblem(null)).toBeNull();
    // An idle login is no problem: grey bars and its age say it (readingNote).
    expect(usageProblem({ kind: 'loginExpired' })).toBeNull();
    expect(usageProblem({ kind: 'loginExpired', status: 401 })).toBeNull();
    expect(usageProblem({ kind: 'failed', message: 'the endpoint answered 500' })).toBe(
      'Could not read: the endpoint answered 500',
    );
    for (const message of [
      'HTTP 403: missing permission',
      'HTTP 429: rate limited',
      'network error: connection failed',
      'TLS error: secure connection failed',
      'HTTP 200: unexpected answer',
    ]) {
      expect(usageProblem({ kind: 'failed', message })).toBe(`Could not read: ${message}`);
    }
  });

  it('names the other account when a folder is signed in as one Warden knows', () => {
    const personal = account({ id: 'p', label: 'Codex personal' });
    const work = account({
      id: 'w',
      label: 'Codex',
      identityChange: { now: { email: 'me@home.example', organization: null }, knownAccountId: 'p' },
    });
    expect(signedInAs(work, [work, personal])).toBe('Signed in as me@home.example, which is Codex personal');
    expect(signedInAs({ ...work, identityChange: { now: null, knownAccountId: null } }, [work])).toBe('Signed out');
    expect(signedInAs(personal, [personal])).toBeNull();
  });

  it('previews the folder a new account gets, as the server makes it', () => {
    expect(newFolderPreview('%USERPROFILE%', 'claude', 'Second')).toBe('%USERPROFILE%\\.warden\\claude-second');
    expect(newFolderPreview('%USERPROFILE%\\', 'codex', 'Codex spare!')).toBe('%USERPROFILE%\\.warden\\codex-spare');
    expect(newFolderPreview('%USERPROFILE%', 'codex', 'Codexwork')).toBe('%USERPROFILE%\\.warden\\codex-work');
    expect(newFolderPreview('/srv/x', 'claude', '')).toBe('/srv/x/.warden/claude-account');
    expect(newFolderPreview(null, 'claude', 'A')).toBe('');
  });
});

describe('readingNote', () => {
  it('asks for main sign-in again rather than waiting for an account CLI to renew it', () => {
    expect(
      readingNote({ id: 'main', provider: 'claude', usage: null, usageError: { kind: 'loginExpired' } }, 5, NOW)
        ?.problem,
    ).toBe("Sign in again for main's usage");
    expect(
      readingNote({ id: 'main', provider: 'claude', usage: null, usageError: { kind: 'identityChanged' } }, 5, NOW)
        ?.problem,
    ).toBe("Restore main's sign-in to read usage");
  });
  const NOW = 1_800_000_000_000;
  const MIN = 60_000;
  const at = (readAt: number) => ({
    readAt,
    fiveHour: null,
    weekly: null,
    perModelWeekly: [],
    extra: null,
  });
  it('says nothing while the reading is fresh and fine', () => {
    expect(readingNote({ provider: 'claude', usage: at(NOW - 15 * MIN), usageError: null }, 5, NOW)).toBeNull();
    expect(readingNote({ provider: 'claude', usage: null, usageError: null }, 5, NOW)).toBeNull();
  });
  it('names an old reading after three usage intervals', () => {
    expect(readingNote({ provider: 'claude', usage: at(NOW - 16 * MIN), usageError: null }, 5, NOW)).toEqual({
      problem: null,
      age: 'Read 16 min ago',
    });
    expect(readingNote({ provider: 'claude', usage: at(NOW - 16 * MIN), usageError: null }, 30, NOW)).toBeNull();
  });
  it('gives an idle Codex account no age, only a note once a limit has reset since its reading', () => {
    const codex = (fiveHourReset: number | null, weeklyReset: number | null) => ({
      provider: 'codex' as const,
      usage: {
        ...at(NOW - 5 * 60 * MIN),
        fiveHour: { usedPercent: 40, resetsAt: fiveHourReset },
        weekly: { usedPercent: 20, resetsAt: weeklyReset },
      },
      usageError: null,
    });
    expect(readingNote(codex(NOW + MIN, NOW + 60 * MIN), 5, NOW)).toBeNull();
    expect(readingNote(codex(null, null), 5, NOW)).toBeNull();
    expect(readingNote(codex(NOW - MIN, NOW + 60 * MIN), 5, NOW)).toEqual({
      problem: null,
      age: '5-hour limit has reset',
    });
    expect(readingNote(codex(NOW + MIN, NOW - MIN), 5, NOW)?.age).toBe('Weekly limit has reset');
    expect(readingNote(codex(NOW - MIN, NOW - MIN), 5, NOW)?.age).toBe('Both limits have reset');
    // A reset already past when the reading was taken is not news.
    expect(readingNote(codex(NOW - 6 * 60 * MIN, NOW + MIN), 5, NOW)).toBeNull();
  });
  it('puts a problem first, and no Codex sessions yet in grey', () => {
    expect(readingNote({ provider: 'claude', usage: null, usageError: { kind: 'signedOut' } }, 5, NOW)).toEqual({
      problem: 'Signed out',
      age: null,
    });
    expect(readingNote({ provider: 'codex', usage: null, usageError: { kind: 'noLogs' } }, 5, NOW)).toEqual({
      problem: null,
      age: 'No Codex sessions yet',
    });
  });
});

describe('limit resets', () => {
  const DAY = 24 * 3_600_000;

  it('says when a reset expires, with the year only when it is not this one', () => {
    expect(expiryText(new Date(2026, 11, 10, 23, 0).getTime(), NOW)).toBe('Expires 10 Dec, 23:00');
    expect(expiryText(new Date(2027, 2, 9, 7, 5).getTime(), NOW)).toBe('Expires 9 Mar 2027, 07:05');
    expect(expiryText(null, NOW)).toBe('Does not expire');
  });

  it('writes counts in English, grouped, whatever the region', () => {
    expect([count(0), count(7), count(1250)]).toEqual(['0', '7', '1,250']);
  });

  it("lists every account's resets soonest first, never-expiring last, leaving out expired ones", () => {
    const codex = account({
      id: 'c',
      label: 'Codex',
      provider: 'codex',
      resets: [
        { id: null, title: 'Full reset (Weekly + 5 hr)', expiresAt: NOW + 9 * DAY },
        { id: null, title: null, expiresAt: null },
        { id: null, title: 'Gone', expiresAt: NOW - 1 },
      ],
    });
    const other = account({
      id: 'o',
      label: 'Other',
      provider: 'codex',
      resets: [{ id: null, title: 'Soon', expiresAt: NOW + DAY }],
    });
    expect(resetRows([codex, other], NOW, 3).map((r) => [r.account.label, r.title, r.expiresAt])).toEqual([
      ['Other', 'Soon', NOW + DAY],
      ['Codex', 'Full reset (Weekly + 5 hr)', NOW + 9 * DAY],
      ['Codex', 'Reset', null],
    ]);
    expect(resetRows([account({ id: 'm', label: 'Main' })], NOW, 3)).toEqual([]);
  });

  it("filters the rows: an account's left out, or only each account's soonest", () => {
    const codex = account({
      id: 'c',
      label: 'Codex',
      provider: 'codex',
      resets: [
        { id: 'x', title: 'Later', expiresAt: NOW + 9 * DAY },
        { id: 'y', title: 'Never', expiresAt: null },
        { id: 'z', title: 'Sooner', expiresAt: NOW + 2 * DAY },
      ],
    });
    const other = account({ id: 'o', label: 'Other', resets: [{ id: 'w', title: 'Soon', expiresAt: NOW + DAY }] });
    const rows = resetRows([codex, other], NOW, 3);
    const titles = (r: ReturnType<typeof filterResets>) => r.map((x) => x.title);
    expect(titles(filterResets(rows, [], false))).toEqual(['Soon', 'Sooner', 'Later', 'Never']);
    expect(titles(filterResets(rows, ['o'], false))).toEqual(['Sooner', 'Later', 'Never']);
    expect(titles(filterResets(rows, [], true))).toEqual(['Soon', 'Sooner']);
    expect(titles(filterResets(rows, ['c'], true))).toEqual(['Soon']);
    expect(filterResets(rows, ['c', 'o'], false)).toEqual([]);
  });

  it('keys each row by its reset, so using one of a grant takes away the row of the last of them', () => {
    const DAY = 24 * 3_600_000;
    const full = { id: 'grant-full', title: 'Full reset', expiresAt: NOW + DAY };
    const main = (resets: (typeof full | { id: null; title: string; expiresAt: null })[]) =>
      resetRows([account({ id: 'main', label: 'Main', resets })], NOW, 3).map((r) => [r.key, r.id]);
    const old = { id: null, title: 'Old', expiresAt: null };
    expect(main([full, full, old])).toEqual([
      ['main:grant-full:0', 'grant-full'],
      ['main:grant-full:1', 'grant-full'],
      ['main:#2', null],
    ]);
    expect(main([full, old])).toEqual([
      ['main:grant-full:0', 'grant-full'],
      ['main:#1', null],
    ]);
  });

  it('says what using each kind of reset does', () => {
    expect(resetEffect('Full reset')).toBe('It refills its 5-hour and weekly limits now.');
    expect(resetEffect('Full reset (Weekly + 5 hr)')).toBe('It refills its 5-hour and weekly limits now.');
    expect(resetEffect('5-hour reset')).toBe('It refills its 5-hour limit now.');
    expect(resetEffect('Weekly reset')).toBe('It refills its weekly limit now.');
    expect(resetEffect('Fable reset')).toBe('It gives what Fable used this week back to its weekly limit.');
    expect(resetEffect('Reset')).toBe('It refills its limits now.');
  });

  it('says how soon a reset expires within the warning days: today, tomorrow, in N days', () => {
    const now = new Date(2026, 11, 8, 9, 30).getTime();
    const at = (day: number, h: number, m = 0) => new Date(2026, 11, day, h, m).getTime();
    expect(expiryText(at(8, 23), now, 3)).toBe('Expires today, 23:00');
    expect(expiryText(at(9, 8), now, 3)).toBe('Expires tomorrow, 08:00');
    expect(expiryText(at(10, 23), now, 3)).toBe('Expires in 2 days, 10 Dec, 23:00');
    // The window is measured in days of 24 hours: three days from now is in, a minute later is out.
    expect(expiryText(at(11, 9, 30), now, 3)).toBe('Expires in 3 days, 11 Dec, 09:30');
    expect(expiryText(at(11, 9, 31), now, 3)).toBe('Expires 11 Dec, 09:31');
    expect(expiryText(at(10, 23), now, 1)).toBe('Expires 10 Dec, 23:00');
    // Without warning days (and for one that never expires) the plain line.
    expect(expiryText(at(8, 23), now)).toBe('Expires 8 Dec, 23:00');
    expect(expiryText(null, now, 14)).toBe('Does not expire');
    // Across the new year, the date keeps its year.
    const late = new Date(2026, 11, 30, 12, 0).getTime();
    expect(expiryText(new Date(2027, 0, 2, 7, 0).getTime(), late, 3)).toBe('Expires in 3 days, 2 Jan 2027, 07:00');

    expect(expiresSoon(at(11, 9, 30), now, 3)).toBe(true);
    expect(expiresSoon(at(11, 9, 31), now, 3)).toBe(false);
    expect(expiresSoon(null, now, 14)).toBe(false);
    expect(expiresSoon(now - 1, now, 3)).toBe(false);
  });

  it('marks the rows that expire within the warning days', () => {
    const DAY = 24 * 3_600_000;
    const claude = account({
      id: 'main',
      label: 'Main',
      resets: [
        { id: null, title: 'Full reset', expiresAt: NOW + 5 * DAY },
        { id: null, title: 'Full reset', expiresAt: NOW + 2 * DAY },
        { id: null, title: '5-hour reset', expiresAt: null },
      ],
    });
    const rows = (days: number) => resetRows([claude], NOW, days).map((r) => [r.title, r.soon]);
    expect(rows(3)).toEqual([
      ['Full reset', true],
      ['Full reset', false],
      ['5-hour reset', false],
    ]);
    expect(rows(1)).toEqual([
      ['Full reset', false],
      ['Full reset', false],
      ['5-hour reset', false],
    ]);
    expect(rows(14).map((r) => r[1])).toEqual([true, true, false]);
  });
});
