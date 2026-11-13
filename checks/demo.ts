// `npm run demo`: Warden working on made-up accounts, to look at before any live check.
//
// Builds the UI and the demo server (crates/warden-server/examples/demo.rs: the real server with a stand-in for the
// usage endpoint), makes a throwaway home folder with four made-up accounts (two Claude, two Codex, one of them
// switched off) and a throwaway data folder, and feeds the server believable activity: usage readings, Codex session
// logs and status-hook events for sessions that work, wait for you, go idle, run subagents and finish. Made-up apps
// and tools sit in the throwaway home (placeholder versions and folders, CLIs that are empty stand-ins), so Settings,
// General finds them all. Ctrl+C stops it and removes the throwaway folders. Nothing real is read, nothing is
// started and nothing leaves this computer.
//
//   npm run demo [-- --port 4787]

import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { appendFileSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { build } from 'vite';
import { pipeSend } from './lib/pipe.ts';
import { ROOT } from './lib/repo.ts';

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;

interface DemoAccount {
  key: string;
  provider: 'claude' | 'codex';
  folder: string; // under the throwaway home
  label: string;
  email: string;
  organization: string | null;
  enabled: boolean;
  /** Usage at the start and how fast it grows, in percentage points per minute of demo. */
  fiveHour: [number, number];
  weekly: [number, number];
  /** Hours until each limit resets. */
  fiveHourResetH: number;
  weeklyResetH: number;
  /** Minutes until its login expires: an idle login keeps its last reading, shown in grey. */
  loginExpiresMin?: number;
}

const ACCOUNTS: DemoAccount[] = [
  {
    key: 'claude-main',
    provider: 'claude',
    folder: '.claude',
    label: 'Work',
    email: 'ada@example.com',
    organization: 'Example Co',
    enabled: true,
    fiveHour: [38, 0.6],
    weekly: [46, 0.05],
    fiveHourResetH: 2.5,
    weeklyResetH: 80,
  },
  {
    key: 'claude-second',
    provider: 'claude',
    folder: '.warden/claude-second',
    label: 'Second',
    email: 'ada.second@example.com',
    organization: 'Example Co',
    enabled: true,
    fiveHour: [12, 0.4],
    weekly: [71, 0.04],
    fiveHourResetH: 4,
    weeklyResetH: 20,
    loginExpiresMin: 3,
  },
  {
    key: 'codex-main',
    provider: 'codex',
    folder: '.warden/codex-work',
    label: 'Codex',
    email: 'ada@example.com',
    organization: 'Example Co',
    enabled: true,
    fiveHour: [24, 0.5],
    weekly: [33, 0.04],
    fiveHourResetH: 3,
    weeklyResetH: 120,
  },
  {
    key: 'codex-spare',
    provider: 'codex',
    folder: '.warden/codex-spare',
    label: 'Codex spare',
    email: 'spare@example.com',
    organization: null,
    enabled: false,
    fiveHour: [0, 0],
    weekly: [8, 0],
    fiveHourResetH: 5,
    weeklyResetH: 140,
  },
];

/** Made-up projects the sessions work in: a folder name and the branch checked out there. */
const PROJECTS = [
  { folder: 'billing', branch: 'feature/48213-invoice-export' },
  { folder: 'payroll-api', branch: 'bugfix/51077-overtime-rounding' },
  { folder: 'web-portal', branch: 'main' },
  { folder: 'docs-site', branch: 'chore/release-notes' },
] as const;

function arg(name: string, fallback: string): string {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? (process.argv[i + 1] ?? fallback) : fallback;
}

const PORT = Number(arg('port', '4787'));
const BASE = `http://127.0.0.1:${PORT}`;
const started = Date.now();
const work = mkdtempSync(join(tmpdir(), 'warden-demo-'));
const home = join(work, 'home');
const dataDir = join(work, 'data');
const usageDir = join(work, 'usage');
const uiDir = join(work, 'ui');

let server: ChildProcess | undefined;
/** The address the demo server printed, with its token, and the cookie the token trades for (docs/api.md). */
let openUrl: string | undefined;
let cookie: string | undefined;
const timers = new Set<NodeJS.Timeout>();
let stopping = false;

function log(line: string): void {
  console.log(`demo: ${line}`);
}

function later(ms: number, fn: () => void): void {
  const t = setTimeout(() => {
    timers.delete(t);
    if (!stopping) fn();
  }, ms);
  timers.add(t);
}

function every(ms: number, fn: () => void): void {
  const t = setInterval(() => {
    if (!stopping) fn();
  }, ms);
  timers.add(t);
}

function write(path: string, text: string): void {
  mkdirSync(join(path, '..'), { recursive: true });
  writeFileSync(path, text);
}

// ---- The made-up apps and tools ----

/** Version of the made-up Claude desktop app (its install folder is named after it). */
const DEMO_APP_VERSION = '0.0.0-demo';

/**
 * Where Warden looks for the desktop apps and CLIs, inside the throwaway home: Claude's per-user install folder,
 * Codex's Store package folder, and `claude.cmd` and `codex.cmd` on the demo server's PATH (its `bin`). The CLIs
 * are stand-ins that do nothing; the demo server's sign-in only prints, so none of them is ever run.
 */
function makeTools(): void {
  mkdirSync(join(home, 'AppData', 'Local', 'AnthropicClaude', `app-${DEMO_APP_VERSION}`), { recursive: true });
  mkdirSync(join(home, 'AppData', 'Local', 'Packages', 'OpenAI.Codex_demo'), { recursive: true });
  for (const cli of ['claude', 'codex'])
    write(join(home, 'bin', `${cli}.cmd`), '@echo off\r\necho The Warden demo stand-in does nothing.\r\n');
}

// ---- The throwaway account folders ----

function b64url(text: string): string {
  return Buffer.from(text).toString('base64url');
}

function token(a: DemoAccount): string {
  return `demo-${a.key}`;
}

function makeFolder(a: DemoAccount): void {
  const folder = join(home, a.folder);
  mkdirSync(folder, { recursive: true });
  if (a.provider === 'claude') {
    write(
      join(folder, '.credentials.json'),
      JSON.stringify({
        claudeAiOauth: {
          accessToken: a.folder === '.claude' ? 'demo-main-app-private' : token(a),
          refreshToken: `${token(a)}-refresh`,
          expiresAt: started + (a.loginExpiresMin !== undefined ? a.loginExpiresMin * 60_000 : 24 * HOUR),
          scopes: ['user:inference'],
          subscriptionType: 'team',
        },
      }),
    );
    write(
      join(folder, '.claude.json'),
      JSON.stringify({
        numStartups: 12,
        oauthAccount: {
          accountUuid: `acc-${a.key}`,
          emailAddress: a.email,
          organizationUuid: a.organization ? 'org-example' : null,
          organizationName: a.organization,
          displayName: 'Ada',
        },
      }),
    );
  } else {
    const claims = {
      email: a.email,
      'https://api.openai.com/auth': {
        chatgpt_account_id: `ws-${a.key}`,
        organizations: a.organization ? [{ id: 'org-1', title: a.organization, is_default: true }] : [],
      },
    };
    const jwt = `${b64url('{"alg":"RS256"}')}.${b64url(JSON.stringify(claims))}.signature`;
    write(
      join(folder, 'auth.json'),
      JSON.stringify({
        OPENAI_API_KEY: null,
        tokens: { id_token: jwt, access_token: token(a), refresh_token: `${token(a)}-refresh`, account_id: 'ws-1' },
        last_refresh: new Date(started).toISOString(),
      }),
    );
    mkdirSync(join(folder, 'sessions'), { recursive: true });
  }
}

function makeProjects(): void {
  for (const p of PROJECTS) {
    write(join(home, 'src', p.folder, '.git', 'HEAD'), `ref: refs/heads/${p.branch}\n`);
  }
}

// ---- Usage: Claude answers through the stand-in endpoint, Codex through its session logs ----

/** A limit's use after `minutes` of demo. */
function used(start: number, perMinute: number, minutes: number): number {
  return Math.min(100, Math.round((start + perMinute * minutes) * 10) / 10);
}

function resetAt(hours: number): number {
  return started + hours * HOUR;
}

function writeClaudeUsage(a: DemoAccount, minutes: number): void {
  const five = used(a.fiveHour[0], a.fiveHour[1], minutes);
  const week = used(a.weekly[0], a.weekly[1], minutes);
  const body = {
    five_hour: { utilization: five, resets_at: new Date(resetAt(a.fiveHourResetH)).toISOString() },
    seven_day: { utilization: week, resets_at: new Date(resetAt(a.weeklyResetH)).toISOString() },
    seven_day_opus: { utilization: Math.round(week * 0.6), resets_at: new Date(resetAt(a.weeklyResetH)).toISOString() },
    extra_usage: { is_enabled: false, monthly_limit: null, used_credits: null, utilization: null },
  };
  write(join(usageDir, `${token(a)}.json`), JSON.stringify(body));
}

function codexLog(a: DemoAccount): string {
  const d = new Date(started);
  const day = [
    String(d.getFullYear()),
    String(d.getMonth() + 1).padStart(2, '0'),
    String(d.getDate()).padStart(2, '0'),
  ];
  return join(home, a.folder, 'sessions', ...day, `rollout-demo-${a.key}.jsonl`);
}

function appendCodexUsage(a: DemoAccount, minutes: number): void {
  const line = {
    timestamp: new Date().toISOString(),
    type: 'event_msg',
    payload: {
      type: 'token_count',
      info: null,
      rate_limits: {
        primary: {
          used_percent: used(a.fiveHour[0], a.fiveHour[1], minutes),
          window_minutes: 300,
          resets_at: Math.round(resetAt(a.fiveHourResetH) / 1000),
        },
        secondary: {
          used_percent: used(a.weekly[0], a.weekly[1], minutes),
          window_minutes: 10080,
          resets_at: Math.round(resetAt(a.weeklyResetH) / 1000),
        },
      },
    },
  };
  const path = codexLog(a);
  mkdirSync(join(path, '..'), { recursive: true });
  appendFileSync(path, `${JSON.stringify(line)}\n`);
}

/** Writes the next readings. A switched-off Codex account gets one, at the start, as if it was last used then. */
function updateUsage(first = false): void {
  const minutes = (Date.now() - started) / MINUTE;
  for (const a of ACCOUNTS) {
    if (a.provider === 'claude') writeClaudeUsage(a, minutes);
    else if (a.enabled || first) appendCodexUsage(a, minutes);
  }
}

// ---- The API ----

async function api(method: string, path: string, body?: unknown): Promise<unknown> {
  const res = await fetch(`${BASE}/api${path}`, {
    method,
    headers: {
      ...(cookie ? { Cookie: cookie } : {}),
      ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${await res.text()}`);
  return res.status === 204 ? null : res.json();
}

/**
 * Waits until the demo server answers. Only its own answer counts: another Warden already on the port (one with real
 * accounts) has another data folder and is never sent anything that changes it.
 */
async function waitForServer(): Promise<void> {
  for (let i = 0; i < 300; i++) {
    if (server?.exitCode !== null) throw new Error('the demo server stopped while starting');
    if (!cookie) {
      await new Promise((r) => setTimeout(r, 100));
      continue;
    }
    let state: { dataDir?: string } | undefined;
    try {
      state = (await api('GET', '/state')) as { dataDir?: string };
    } catch {
      await new Promise((r) => setTimeout(r, 100));
      continue;
    }
    if (state.dataDir?.toLowerCase() === dataDir.toLowerCase()) return;
    throw new Error(`another Warden answers on port ${PORT}; pick a free one with --port`);
  }
  throw new Error('the demo server did not answer within 30 seconds');
}

interface StateAccount {
  id: string;
  folder: string;
}

const ids = new Map<string, string>(); // account key → Warden's id

async function setUp(): Promise<void> {
  await api('POST', '/setup', {
    // Main is implied; the Codex desktop folder is never added.
    accounts: ACCOUNTS.filter((a) => a.folder !== '.claude').map((a) => ({
      provider: a.provider,
      folder: join(home, a.folder),
      label: a.label,
    })),
  });
  const state = (await api('GET', '/state')) as { main: StateAccount | null; accounts: StateAccount[] };
  for (const a of ACCOUNTS) {
    const found = [state.main, ...state.accounts]
      .filter((s): s is StateAccount => s !== null)
      .find((s) => s.folder.toLowerCase() === join(home, a.folder).toLowerCase());
    if (!found) throw new Error(`account ${a.label} was not added`);
    ids.set(a.key, found.id);
  }
  // Work hours, so main's hand-off point (the status bar's 'until N%' and the mark) moves as in real use.
  await api('PUT', '/settings', { usageIntervalMin: 2, workHours: { days: [1, 2, 3, 4, 5], from: 540, to: 1080 } });
  for (const a of ACCOUNTS) {
    const id = ids.get(a.key) ?? '';
    if (!a.enabled) await api('PATCH', `/accounts/${id}`, { enabled: false });
  }
}

// ---- Sessions: status-hook events as the CLIs send them ----

type Step = { event: Record<string, unknown>; wait: number };

/** One turn: a prompt, some tools, maybe a subagent or a question for you, then the turn ends and it sits idle. */
function turn(provider: 'claude' | 'codex', round: number): Step[] {
  const tool = (name: string, wait: number): Step[] => [
    { event: { hook_event_name: 'PreToolUse', tool_name: name }, wait },
    { event: { hook_event_name: 'PostToolUse', tool_name: name }, wait: 2 },
  ];
  const [read, edit, run] = provider === 'claude' ? ['Read', 'Edit', 'Bash'] : ['read_file', 'apply_patch', 'shell'];
  const steps: Step[] = [{ event: { hook_event_name: 'UserPromptSubmit' }, wait: 3 }, ...tool(read, 4)];
  if (provider === 'claude' && round % 2 === 0) {
    steps.push(
      { event: { hook_event_name: 'SubagentStart', agent_id: `a${round}`, agent_type: 'Explore' }, wait: 2 },
      { event: { hook_event_name: 'SubagentStart', agent_id: `b${round}`, agent_type: 'code-reviewer' }, wait: 14 },
      { event: { hook_event_name: 'SubagentStop', agent_id: `a${round}`, agent_type: 'Explore' }, wait: 8 },
      { event: { hook_event_name: 'SubagentStop', agent_id: `b${round}`, agent_type: 'code-reviewer' }, wait: 2 },
    );
  }
  steps.push(...tool(edit, 6));
  if (round % 2 === 1) {
    // Asks to run a command and waits for you.
    steps.push(
      { event: { hook_event_name: 'PreToolUse', tool_name: run }, wait: 1 },
      { event: { hook_event_name: 'PermissionRequest', tool_name: run }, wait: 20 },
    );
  }
  steps.push(...tool(run, 8), { event: { hook_event_name: 'Stop' }, wait: 25 });
  return steps;
}

interface SessionPlan {
  account: string;
  project: number;
  rounds: number; // turns before the session ends (Infinity: it never ends)
  delay: number; // seconds before it starts
  /** The session plan (by index) whose session started this one: a delegated run from main's advice. */
  parent?: number;
}

const SESSIONS: SessionPlan[] = [
  { account: 'claude-main', project: 0, rounds: Infinity, delay: 1 },
  { account: 'claude-main', project: 2, rounds: 3, delay: 6 },
  { account: 'claude-second', project: 1, rounds: Infinity, delay: 3, parent: 0 },
  { account: 'codex-main', project: 3, rounds: 2, delay: 9 },
  { account: 'codex-main', project: 1, rounds: Infinity, delay: 15, parent: 0 },
];

let sessionCount = 0;
/** Each plan's current session id, so a delegated run can name the main session that started it. */
const currentSession = new Map<number, string>();

function runSession(plan: SessionPlan, index: number): void {
  const account = ACCOUNTS.find((a) => a.key === plan.account);
  const id = ids.get(plan.account);
  if (!account || !id) return;
  sessionCount += 1;
  const sessionId = `demo-${String(sessionCount).padStart(4, '0')}-${plan.account}`;
  currentSession.set(index, sessionId);
  // As `warden event` sends it when the delegated run was started with WARDEN_PARENT_SESSION set.
  const parent = plan.parent !== undefined ? currentSession.get(plan.parent) : undefined;
  const cwd = join(home, 'src', PROJECTS[plan.project % PROJECTS.length]?.folder ?? 'billing');
  const steps: Step[] = [{ event: { hook_event_name: 'SessionStart' }, wait: 2 }];
  const rounds = Number.isFinite(plan.rounds) ? plan.rounds : 1;
  for (let r = 0; r < rounds; r++) steps.push(...turn(account.provider, sessionCount + r));
  if (Number.isFinite(plan.rounds)) steps.push({ event: { hook_event_name: 'SessionEnd' }, wait: 40 });
  let i = 0;
  const next = (): void => {
    const step = steps[i];
    if (!step) {
      // An endless session carries on with another turn; a finished one is followed by a new session.
      if (Number.isFinite(plan.rounds)) runSession({ ...plan, project: plan.project + 1, delay: 0 }, index);
      else {
        steps.push(...turn(account.provider, sessionCount + i));
        next();
      }
      return;
    }
    i += 1;
    const event = { session_id: sessionId, cwd, ...(parent ? { parent_session_id: parent } : {}), ...step.event };
    // Over the pipe, as the status hook's `warden event` sends it.
    pipeSend(dataDir, id, event).catch((e: unknown) => log(`event not taken: ${String(e)}`));
    later(step.wait * 1000 * (0.7 + Math.random() * 0.6), next);
  };
  later(plan.delay * 1000, next);
}

// ---- Start and stop ----

function stop(code: number): void {
  if (stopping) return;
  stopping = true;
  for (const t of timers) clearTimeout(t);
  const done = async (): Promise<void> => {
    // The server's files may stay locked for a moment after it exits.
    for (let i = 0; i < 20 && existsSync(work); i++) {
      try {
        rmSync(work, { recursive: true, force: true });
      } catch {
        await new Promise((r) => setTimeout(r, 250));
      }
    }
    log(existsSync(work) ? `could not remove ${work}; remove it by hand` : 'stopped; throwaway folders removed');
    process.exit(code);
  };
  if (server && server.exitCode === null && server.signalCode === null) {
    server.once('exit', () => void done());
    server.kill();
  } else void done();
}

async function main(): Promise<void> {
  process.on('SIGINT', () => stop(0));
  process.on('SIGTERM', () => stop(0));
  process.on('SIGBREAK', () => stop(0));

  log('building the demo server');
  const cargo = spawnSync('cargo', ['build', '-p', 'warden-server', '--example', 'demo', '-p', 'warden-cli'], {
    cwd: ROOT,
    stdio: 'inherit',
  });
  if (cargo.status !== 0) throw new Error('cargo build failed');
  log('building the UI');
  await build({ root: join(ROOT, 'ui'), logLevel: 'warn', build: { outDir: uiDir, emptyOutDir: true } });

  for (const a of ACCOUNTS) makeFolder(a);
  const ownSignIn = join(dataDir, 'main-sign-in');
  mkdirSync(ownSignIn, { recursive: true });
  copyFileSync(join(home, '.claude', '.claude.json'), join(ownSignIn, '.claude.json'));
  const main = ACCOUNTS.find((a) => a.folder === '.claude')!;
  write(
    join(ownSignIn, '.credentials.json'),
    JSON.stringify({
      claudeAiOauth: {
        accessToken: token(main),
        refreshToken: token(main) + '-refresh',
        expiresAt: started + 24 * HOUR,
        scopes: ['user:profile'],
      },
    }),
  );
  mkdirSync(join(home, '.codex'), { recursive: true });
  write(
    join(home, '.codex', 'auth.json'),
    JSON.stringify({ fixture: 'The Codex app folder is private and never added.' }),
  );
  makeTools();
  makeProjects();
  updateUsage(true);

  const exe = (name: string) => join(ROOT, 'target', 'debug', `${name}${process.platform === 'win32' ? '.exe' : ''}`);
  // The server's log goes to the console only when something is wrong, so the URL stays visible.
  server = spawn(
    join(ROOT, 'target', 'debug', 'examples', `demo${process.platform === 'win32' ? '.exe' : ''}`),
    [
      ...['--port', String(PORT), '--data-dir', dataDir, '--home', home],
      ...['--usage-dir', usageDir, '--ui-dir', uiDir, '--route-exe', exe('warden')],
    ],
    { cwd: work, stdio: ['ignore', 'ignore', 'pipe'] },
  );
  // The server writes a line in several pieces (`eprintln!` sends "demo: open " and the address separately), so whole
  // lines are collected before matching; the last, unfinished piece waits for the next chunk.
  let pending = '';
  server.stderr?.on('data', (chunk: Buffer) => {
    const lines = (pending + chunk.toString()).split('\n');
    pending = lines.pop() ?? '';
    for (const line of lines) {
      // The server's address with this run's token: its cookie goes with every request the demo sends.
      const open = /^demo: open (http:\S+\?token=([0-9a-f]+))/.exec(line.trim());
      if (open?.[1] && open[2]) {
        openUrl = open[1];
        cookie = `warden-token-${PORT}=${open[2]}`;
      } else if (/fail|error|could not/i.test(line)) log(line.trim());
    }
  });
  server.on('exit', (code) => {
    // Ctrl+C reaches the server too; this script's own handler goes first.
    setTimeout(() => {
      if (!stopping) {
        log(`the server stopped (${code})`);
        stop(1);
      }
    }, 500);
  });
  await waitForServer();
  await setUp();

  // Claude's readings: the stand-in endpoint's answers change as the demo runs; Warden reads them on its interval,
  // and a refresh every minute stands in for pressing Refresh. Codex's come from its logs as they are written.
  every(20_000, updateUsage);
  every(MINUTE, () => void api('POST', '/usage/refresh').catch(() => {}));
  SESSIONS.forEach((s, k) => runSession(s, k));

  log(`Warden is running on made-up accounts: open ${openUrl ?? BASE}`);
  log('press Ctrl+C to stop; the throwaway folders are removed');
}

main().catch((e: unknown) => {
  log(String(e instanceof Error ? e.message : e));
  stop(1);
});
