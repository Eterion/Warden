// Made-up states for the rendered accessibility check and its screenshots: the UI is served by `vite preview`
// with this standing in for the local server's API (docs/api.md). No real account is read.

import type { Account, Bin, Candidate, Session, State } from '../../ui/src/api.ts';

const MIN = 60_000;
const HOUR = 60 * MIN;
const HOME = '%USERPROFILE%';

function account(a: Partial<Account> & Pick<Account, 'id' | 'label' | 'provider' | 'folder'>): Account {
  return {
    enabled: true,
    sameAsMain: false,
    identity: null,
    identityChange: null,
    usage: null,
    usageError: null,
    resets: [],
    hooks: 'installed',
    ...a,
  };
}

export function normalState(now: number): State {
  const accounts: Account[] = [
    account({
      id: 'main',
      label: 'Main',
      provider: 'claude',
      folder: `${HOME}\\.claude`,
      identity: { email: 'sam@example.com', organization: 'Example Ltd' },
      usage: {
        readAt: now - 1 * MIN,
        fiveHour: { usedPercent: 61, resetsAt: now + 2 * HOUR + 10 * MIN },
        weekly: { usedPercent: 34, resetsAt: now + 3 * 24 * HOUR },
        perModelWeekly: [],
        extra: { enabled: true, usedCredits: 1240, monthlyLimit: 5000, usedPercent: 24.8 },
      },
    }),
    account({
      id: 'a2',
      label: 'Second',
      provider: 'claude',
      folder: `${HOME}\\.warden\\claude-second`,
      identity: { email: 'sam.second@example.com', organization: 'Example Ltd' },
      usage: {
        readAt: now - 3 * HOUR,
        fiveHour: { usedPercent: 72, resetsAt: now + 40 * MIN },
        weekly: { usedPercent: 83, resetsAt: now + 26 * HOUR },
        perModelWeekly: [],
        extra: { enabled: false, usedCredits: null, monthlyLimit: null, usedPercent: null },
      },
      usageError: { kind: 'loginExpired' },
    }),
    account({
      id: 'a3',
      label: 'Codex',
      provider: 'codex',
      folder: `${HOME}\\.warden\\codex-work`,
      identity: { email: 'sam@example.com', organization: null },
      identityChange: { now: { email: 'sam@home.example', organization: null }, knownAccountId: 'a4' },
      usageError: { kind: 'identityChanged' },
      hooks: 'notTrusted',
      usage: {
        readAt: now - 20 * MIN,
        fiveHour: null,
        weekly: { usedPercent: 12, resetsAt: now + 5 * 24 * HOUR },
        perModelWeekly: [],
        extra: null,
      },
    }),
    account({
      id: 'a4',
      label: 'Codex personal',
      provider: 'codex',
      folder: `${HOME}\\.warden\\codex-personal`,
      enabled: false,
      identity: { email: 'sam@home.example', organization: 'Plus' },
      hooks: 'notInstalled',
    }),
  ];
  return {
    hooksOff: false,
    version: '0.1.0',
    setupDone: true,
    dataDir: `${HOME}\\AppData\\Roaming\\Warden`,
    logDir: `${HOME}\\AppData\\Roaming\\Warden\\logs`,
    settings: {
      reserve: 25,
      usageIntervalMin: 5,
      theme: 'system',
      keepInTray: false,
      workHours: null,
      resetWarningDays: 3,
      showQuietSessions: true,
    },
    accounts: accounts.slice(1),
    main: { ...accounts[0]!, signIn: { state: 'signedIn', email: accounts[0]!.identity?.email ?? null } },
    problems: [],
    lastRead: now - 1 * MIN,
    revision: 1,
    homeDir: HOME,
    delegation: {
      mainWorksItself: true,
      delegated: { accountId: 'a2' },
      mainReason: 'Main has used 34% of its week; it keeps 25% for you.',
      delegatedReason: 'Second has the most allowance left.',
      handOffAt: 84,
    },
    routeTool: `${HOME}\\AppData\\Local\\Warden\\warden.exe`,
    sessions: sessions(now),
    activity: activity(now),
    desktop: null,
    update: null,
  };
}

function session(s: Partial<Session> & Pick<Session, 'id' | 'accountId' | 'state' | 'doing'>): Session {
  return {
    parentId: null,
    provider: 'claude',
    project: null,
    name: null,
    startedAt: 0,
    lastEventAt: 0,
    endedAt: null,
    subagents: [],
    signedInAs: null,
    source: 'hook',
    ...s,
  };
}

/**
 * Sessions on the accounts in use: one waits for you, three work, one is idle, one has just ended, and one is seen only
 * in a CLI's list (its state not known).
 */
function sessions(now: number): Session[] {
  return [
    session({
      id: 's1',
      accountId: 'main',
      state: 'working',
      doing: 'Editing files',
      project: 'warden',
      name: 'Overview design',
      startedAt: now - 38 * MIN,
    }),
    session({
      id: 's2',
      accountId: 'main',
      state: 'idle',
      doing: 'Waiting for your next message',
      project: 'notes',
      name: 'Status skill',
      startedAt: now - 62 * MIN,
    }),
    session({
      id: 's3',
      accountId: 'a2',
      state: 'working',
      doing: 'Running commands',
      project: 'shop',
      name: '55402 performance admin',
      startedAt: now - 134 * MIN,
      subagents: [{ type: 'Explore' }, { type: 'general-purpose' }],
    }),
    session({
      id: 's4',
      accountId: 'a2',
      state: 'working',
      doing: 'Reading files',
      project: 'shop',
      name: 'Admin operations',
      startedAt: now - 51 * MIN,
      subagents: [{ type: 'Explore' }],
    }),
    session({
      id: 's5',
      accountId: 'a3',
      provider: 'codex',
      state: 'waiting',
      doing: 'Asks to run a command',
      project: 'design-kit',
      name: 'Fix review comments',
      startedAt: now - 12 * MIN,
      signedInAs: 'sam@home.example',
    }),
    session({
      id: 's6',
      accountId: 'a3',
      provider: 'codex',
      state: 'done',
      doing: 'Ended',
      project: 'design-kit',
      name: 'Lint fixes',
      startedAt: now - 30 * MIN,
      signedInAs: 'sam@home.example',
      endedAt: now - 4 * MIN,
    }),
    session({
      id: 's7',
      accountId: 'main',
      state: 'idle',
      doing: 'Seen in the session list, no status hook',
      project: 'scratch',
      name: 'Release notes',
      startedAt: now - 9 * MIN,
      lastEventAt: now - 9 * MIN,
      source: 'list',
    }),
  ];
}

/** Two hours of activity, oldest first: sessions working per 5-minute bin, and the bins where one started waiting. */
function track(now: number, working: number[], waited: number[] = []): Bin[] {
  const bin = 5 * MIN;
  const end = now - (now % bin);
  return working.map((w, k) => ({
    at: end - (working.length - 1 - k) * bin,
    working: w,
    last: w,
    waited: waited.includes(k),
  }));
}

function activity(now: number): State['activity'] {
  return [
    { accountId: 'main', bins: track(now, [0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1]) },
    {
      accountId: 'a2',
      bins: track(now, [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2], [8]),
    },
    {
      accountId: 'a3',
      bins: track(now, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 0, 0, 1, 1, 0, 0], [22]),
    },
  ];
}

/**
 * The desktop app: Keep running in the tray on, Start with Windows off. No account has spent extra usage (Main's is
 * on, with nothing spent), so the Overview shows no Extra usage column.
 */
export function desktopState(now: number): State {
  const s = normalState(now);
  const accounts = s.accounts.map((a) =>
    a.usage?.extra ? { ...a, usage: { ...a.usage, extra: { ...a.usage.extra, usedCredits: 0, usedPercent: 0 } } } : a,
  );
  const main = s.main?.usage?.extra
    ? { ...s.main, usage: { ...s.main.usage, extra: { ...s.main.usage.extra, usedCredits: 0, usedPercent: 0 } } }
    : s.main;
  return { ...s, main, accounts, settings: { ...s.settings, keepInTray: true }, desktop: { startWithWindows: false } };
}

/** First run: main is fixed, Warden is not signed in for its usage yet; only Warden accounts are chosen. */
export function setupState(now: number): State {
  const s = normalState(now);
  const main = s.main && { ...s.main, usage: null, signIn: { state: 'signedOut' as const, email: null } };
  return { ...s, main, setupDone: false, accounts: [], lastRead: null, sessions: [], activity: [] };
}

export const candidates: Candidate[] = [
  {
    provider: 'claude',
    folder: `${HOME}\\.warden\\claude-second`,
    label: 'Second',
    identity: { email: 'sam.second@example.com', organization: 'Example Ltd' },
    signedIn: true,
    known: false,
  },
  {
    provider: 'codex',
    folder: `${HOME}\\.warden\\codex-work`,
    label: 'Codex',
    identity: { email: 'sam@example.com', organization: 'Plus' },
    signedIn: true,
    known: false,
  },
  {
    provider: 'codex',
    folder: `${HOME}\\.codex-old`,
    label: 'Codex old',
    identity: null,
    signedIn: false,
    known: false,
  },
];

/** Five folders found at first run, for setup at the window's smallest size. */
export const fiveCandidates: Candidate[] = [
  ...candidates,
  {
    provider: 'claude',
    folder: `${HOME}\\.warden\\claude-review`,
    label: 'Review',
    identity: { email: 'sam.review@example.com', organization: 'Example Ltd' },
    signedIn: true,
    known: false,
  },
  {
    provider: 'claude',
    folder: `${HOME}\\.warden\\claude-spare`,
    label: 'Spare',
    identity: { email: 'sam.spare@example.com', organization: 'Example Ltd' },
    signedIn: true,
    known: false,
  },
];

/**
 * Settings › Accounts with one account in each Hooks state: main installed, Second missing, Codex not trusted, Codex
 * personal switched off with none, and a switched-off account whose hook is still there.
 */
export function hooksState(state: State): void {
  const second = state.accounts.find((a) => a.id === 'a2')!;
  second.hooks = 'notInstalled';
  state.accounts.push(
    account({
      id: 'a5',
      label: 'Claude old',
      provider: 'claude',
      folder: `${HOME}\\.claude-old`,
      enabled: false,
      identity: { email: 'sam.old@example.com', organization: 'Example Ltd' },
      hooks: 'installed',
    }),
  );
}

/**
 * Settings › Accounts with one of each problem: on main, the Claude app missing, main's usage unknown, its folder
 * signed in as someone else, a hook missing and the Claude CLI missing; on the others, an account signed out with its
 * hook missing, a changed sign-in with its hook not trusted, one signed in as main's account, and an old Codex CLI.
 */
export function problemsState(state: State): void {
  state.problems = [
    { kind: 'claudeAppMissing' },
    { kind: 'cliMissing', provider: 'claude', name: 'claude' },
    { kind: 'cliTooOld', provider: 'codex', name: 'codex', version: '0.1.0', needed: '1.0.0' },
  ];
  if (state.main) {
    state.main.signIn = { state: 'signedOut', email: null };
    state.main.identityChange = { now: { email: 'sam@home.example', organization: null }, knownAccountId: null };
    state.main.hooks = 'notInstalled';
  }
  const second = state.accounts.find((a) => a.id === 'a2')!;
  second.usageError = { kind: 'signedOut' };
  second.hooks = 'notInstalled';
  state.accounts.push(
    account({
      id: 'a5',
      label: 'Claude spare',
      provider: 'claude',
      folder: `${HOME}\\.warden\\claude-spare`,
      identity: { email: 'sam@example.com', organization: 'Example Ltd' },
      sameAsMain: true,
    }),
  );
}

/**
 * The sessions as a tree: two main sessions, the first with a Claude child on Second and a Codex child waiting for
 * you, and a session on Second whose parent is gone, at the top level.
 */
export function treeState(now: number): State {
  const s = normalState(now);
  const byId = new Map(s.sessions.map((x) => [x.id, x]));
  byId.get('s3')!.parentId = 's1';
  byId.get('s5')!.parentId = 's1';
  byId.get('s4')!.parentId = 'a-session-that-has-ended';
  s.sessions = s.sessions.filter((x) => x.id !== 's6');
  return s;
}

/**
 * The sessions tree with ended and not-known sessions hidden: Codex's ended session started a session on Second, so
 * it stays as its faded parent; the not-known one is left out.
 */
export function quietHiddenState(now: number): State {
  const s = normalState(now);
  s.settings.showQuietSessions = false;
  const byId = new Map(s.sessions.map((x) => [x.id, x]));
  byId.get('s3')!.parentId = 's1';
  byId.get('s5')!.parentId = 's1';
  byId.get('s4')!.parentId = 's6';
  return s;
}

/**
 * The Overview with limit resets: main's Claude resets (two full resets, one of them inside the warning days, and a
 * 5-hour reset), Codex has two, one of them next year; a switched-off account's are not shown.
 */
export function resetsState(now: number): State {
  const s = healthyState(now);
  const day = 24 * HOUR;
  const nextYear = new Date(new Date(now).getFullYear() + 1, 2, 9, 23, 0).getTime();
  if (s.main) {
    s.main = {
      ...s.main,
      resets: [
        { id: 'grant-full', title: 'Full reset', expiresAt: now + 2 * day },
        { id: 'grant-session', title: '5-hour reset', expiresAt: now + 6 * day },
        { id: 'grant-later', title: 'Full reset', expiresAt: now + 12 * day },
      ],
    };
  }
  s.accounts = s.accounts.map((a) =>
    a.id === 'a3'
      ? {
          ...a,
          resets: [
            { id: 'credit-1', title: 'Full reset (Weekly + 5 hr)', expiresAt: now + 9 * day },
            { id: 'credit-2', title: null, expiresAt: nextYear },
          ],
        }
      : a.id === 'a4'
        ? { ...a, resets: [{ id: 'credit-3', title: 'Full reset (Weekly + 5 hr)', expiresAt: now + 2 * day }] }
        : a,
  );
  return s;
}

/** Settings › Accounts with all well: no changed sign-in, every wanted hook in place, every reading current. */
export function healthyState(now: number): State {
  const s = normalState(now);
  s.accounts = s.accounts.map((a) =>
    a.id === 'a3'
      ? { ...a, identityChange: null, usageError: null, hooks: 'installed' }
      : a.id === 'a2'
        ? { ...a, usageError: null }
        : a,
  );
  s.sessions = s.sessions.map((x) => ({ ...x, signedInAs: null }));
  return s;
}
