// Warden's API (docs/api.md). Inside the desktop app it is answered in-process through the app's own `warden` URL
// scheme (no network port); in a browser (development only) it is `/api` on the page's own server.

import { isTauri } from '@tauri-apps/api/core';

export type Provider = 'claude' | 'codex';

export interface Identity {
  email: string;
  organization: string | null;
}

export interface Limit {
  usedPercent: number;
  resetsAt: number | null;
}

export interface Usage {
  readAt: number;
  fiveHour: Limit | null;
  weekly: Limit | null;
  perModelWeekly: { model: string; limit: Limit }[];
  extra: {
    enabled: boolean;
    usedCredits: number | null;
    monthlyLimit: number | null;
    usedPercent: number | null;
  } | null;
}

export type UsageError =
  | { kind: 'signedOut' }
  | { kind: 'loginExpired'; status?: number }
  | { kind: 'identityChanged' }
  | { kind: 'noLogs' }
  | { kind: 'failed'; message: string };

export interface IdentityChange {
  now: Identity | null;
  knownAccountId: string | null;
}

export interface Account {
  id: string;
  label: string;
  provider: Provider;
  folder: string;
  enabled: boolean;
  sameAsMain: boolean;
  identity: Identity | null;
  identityChange: IdentityChange | null;
  usage: Usage | null;
  usageError: UsageError | null;
  hooks: HookSetup;
}

/** 'notTrusted': Codex only; the hook is in hooks.json but config.toml does not trust all of it yet. */
export type HookSetup = 'installed' | 'notTrusted' | 'notInstalled';

export interface Main extends Account {
  signIn: { state: 'signedIn' | 'signedOut' | 'expired' | 'otherAccount'; email: string | null };
}
export type Problem =
  | { kind: 'claudeAppMissing' }
  | { kind: 'claudeAppSignedOut' }
  | { kind: 'cliMissing'; provider: Provider; name: string }
  | { kind: 'cliTooOld'; provider: Provider; name: string; version: string; needed: string };
export interface Delegation {
  mainWorksItself: boolean;
  delegated: { accountId: string } | null;
  mainReason: string;
  delegatedReason: string;
  /** Main's hand-off point: the whole percent of its week from which it delegates; the same number decides. */
  handOffAt: number;
}

export type Theme = 'system' | 'light' | 'dark';

export interface Settings {
  reserve: number;
  usageIntervalMin: number;
  theme: Theme;
  /** Desktop app: closing the window leaves Warden in the tray. */
  keepInTray: boolean;
  /** The user's usual work hours, or null: every hour counts. */
  workHours: WorkHours | null;
}

/** One window on each work day, in local time: ISO weekdays (1 = Monday), minutes after midnight in steps of 30. */
export interface WorkHours {
  days: number[];
  from: number;
  to: number;
}

/** A change to the settings. `keepInTray` and `startWithWindows` only in the desktop app. */
export type SettingsPatch = Partial<Settings> & { startWithWindows?: boolean };

export interface State {
  hooksOff: boolean;
  version: string;
  setupDone: boolean;
  dataDir: string;
  /** The rolling log files' folder, inside the data folder. */
  logDir: string;
  settings: Settings;
  accounts: Account[];
  main: Main | null;
  problems: Problem[];
  lastRead: number | null;
  revision: number;
  homeDir: string | null;
  delegation: Delegation;
  /** The warden.exe the hooks run; null when not found beside the server. */
  routeTool: string | null;
  sessions: Session[];
  activity: { accountId: string; bins: Bin[] }[];
  /** Present only in the desktop app. */
  desktop: { startWithWindows: boolean } | null;
}

export interface Candidate {
  provider: Provider;
  folder: string;
  label: string;
  identity: Identity | null;
  signedIn: boolean;
  known: boolean;
}

/** An error the server answered, with its plain sentence. */
export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

/** Where the desktop app's window reaches its own `warden` scheme (WebView2 serves app schemes as `http://<scheme>.localhost`). */
export const DESKTOP_ORIGIN = 'http://warden.localhost';

/** The address of an API path (`/api/...`) where the page runs: the desktop app's own scheme, or the page's server. */
export function apiUrl(path: string): string {
  return isTauri() ? DESKTOP_ORIGIN + path : path;
}

async function request<T>(method: string, path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  const init: RequestInit = { method, signal };
  if (body !== undefined) {
    init.headers = { 'Content-Type': 'application/json' };
    init.body = JSON.stringify(body);
  }
  const res = await fetch(apiUrl(path), init);
  const data: unknown = await res.json().catch(() => null);
  if (!res.ok) {
    const message = (data as { error?: string } | null)?.error ?? `Warden answered ${res.status}.`;
    throw new ApiError(res.status, message);
  }
  return data as T;
}

export const api = {
  state: () => request<State>('GET', '/api/state'),
  stateAfter: (revision: number, signal: AbortSignal) =>
    request<State>('GET', `/api/state/wait?since=${revision}`, undefined, signal),
  refresh: () => request<State>('POST', '/api/usage/refresh'),
  settings: (patch: SettingsPatch) => request<State>('PUT', '/api/settings', patch),
  scanTools: () => request<State>('POST', '/api/tools/scan'),
  candidates: () => request<Candidate[]>('GET', '/api/setup/candidates'),
  finishSetup: (accounts: { provider: Provider; folder: string; label: string }[], skip = false) =>
    request<State>('POST', '/api/setup', { accounts, skip }),
  restartSetup: () => request<State>('POST', '/api/setup/restart'),
  addExisting: (provider: Provider, folder: string, label: string) =>
    request<State>('POST', '/api/accounts', { provider, folder, label }),
  addNew: (provider: Provider, label: string) => request<State>('POST', '/api/accounts/new', { provider, label }),
  update: (id: string, patch: { label?: string; enabled?: boolean }) =>
    request<State>('PATCH', `/api/accounts/${encodeURIComponent(id)}`, patch),
  acceptIdentity: (id: string) =>
    request<State>('POST', `/api/accounts/${encodeURIComponent(id)}/identity`, { action: 'accept' }),
  remove: (id: string) => request<State>('DELETE', `/api/accounts/${encodeURIComponent(id)}`),
  renameMain: (label: string) => request<State>('PATCH', '/api/main', { label }),
  signInMain: () => request<State>('POST', '/api/main/sign-in'),
  signIn: (id: string) => request<State>('POST', '/api/accounts/' + encodeURIComponent(id) + '/sign-in'),
  repairHooks: (id: string) => request<State>('POST', '/api/accounts/' + encodeURIComponent(id) + '/hooks/repair'),
  openLogFolder: () => request<State>('POST', '/api/logs/open'),
};

export type SessionState = 'working' | 'waiting' | 'idle' | 'done';

export interface Session {
  parentId: string | null;
  id: string;
  provider: Provider;
  /** The account whose folder it runs in; it counts for that account unless signedInAs is set. */
  accountId: string;
  /** The email the folder was signed in as instead of its account when the session started; it then counts for no account. */
  signedInAs: string | null;
  state: SessionState;
  doing: string;
  project: string | null;
  name: string | null;
  startedAt: number;
  lastEventAt: number;
  endedAt: number | null;
  subagents: { type: string }[];
  /** list: seen in the CLI's own session list, so its state is not known. */
  source: 'hook' | 'list';
}

/** A 5-minute bin that saw a change; bins without one carry on the `last` count before them. */
export interface Bin {
  at: number;
  working: number;
  last: number;
  waited: boolean;
}
