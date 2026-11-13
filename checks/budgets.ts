// Performance budget tests (warden-plan.md, Performance, Budgets). Builds the release binaries, measures, and
// compares each number with checks/budgets.json. Budgets still null are placeholders: reported, never failed.
// Results are also written to artifacts/budgets.json for the step's approval summary.

import { type ChildProcess, execFileSync, spawn, spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { pipeReady, pipeSend } from './lib/pipe.ts';
import { ROOT, run } from './lib/repo.ts';

interface Budget {
  budget: number | null;
  unit: string;
  what: string;
}

type Measured = number | { notYet: string };

const RELEASE = join(ROOT, 'target', 'release');
const IDLE_SECONDS = 5;
/**
 * Idle CPU is measured over this long. Windows charges CPU time in whole clock ticks (15.6 ms) to whichever thread is
 * running when the tick comes, so an idle process's brief wake-ups (about 0.1 ms a second for the core) read as 0 or
 * a whole tick: over 5 s one tick is 0.31% of a core and two are 0.62%, over budget. Over 20 s a tick is 0.08%.
 */
const IDLE_CPU_SECONDS = 20;
const ROUTE_RUNS = 21;
const HOOK_RUNS = 21;
/** The core's session cap (warden_core::sessions::MAX_SESSIONS). */
const SESSIONS = 200;

function sizeKB(file: string): number {
  return Math.round(statSync(join(RELEASE, file)).size / 1024);
}

interface Sample {
  privateMB: number;
  cpuMs: number;
  /** When it was taken, on a clock shared by every process (ms). */
  atMs: number;
  processes: number;
}

/**
 * Private memory (MB), CPU time (ms) and number of processes of a process, and with `tree` of everything it started
 * (the desktop app's WebView2 runs as a browser process with its own children). A process counts as started by
 * another only if it started after it: Windows keeps a process's parent id after the parent exits and reuses ids,
 * so the leftovers of a process that exited (another app's, or a WebView2 still closing) can look like children of a
 * new process that got the same id.
 */
function sample(pid: number, tree: boolean): Sample {
  const script = [
    '$all = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, CreationDate)',
    `$tree = [System.Collections.Generic.List[object]]::new(); $ids = [System.Collections.Generic.HashSet[int]]::new(); foreach ($c in $all) { if ($c.ProcessId -eq ${pid}) { $tree.Add($c); [void]$ids.Add(${pid}) } }`,
    `if ($tree.Count -eq 0) { throw 'process ${pid} is not running' }`,
    tree
      ? 'for ($i = 0; $i -lt $tree.Count; $i++) { $parent = $tree[$i]; foreach ($c in $all) { if ($c.ParentProcessId -eq $parent.ProcessId -and $c.CreationDate -ge $parent.CreationDate -and $ids.Add([int]$c.ProcessId)) { $tree.Add($c) } } }'
      : '',
    '$bytes = 0; $cpu = 0; $n = 0',
    // A process that exited since it was listed is left out.
    'foreach ($t in $tree) { $p = Get-Process -Id ([int]$t.ProcessId) -ErrorAction SilentlyContinue; if (-not $p) { continue }; $bytes += $p.PrivateMemorySize64; $cpu += $p.TotalProcessorTime.TotalMilliseconds; $n++ }',
    '"$bytes $cpu $([Diagnostics.Stopwatch]::GetTimestamp() * 1000.0 / [Diagnostics.Stopwatch]::Frequency) $n"',
  ].join('; ');
  const out = execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], {
    encoding: 'utf8',
  }).trim();
  const [bytes, cpu, at, n] = out.split(/\s+/).map(Number);
  if (![bytes, cpu, at, n].every((v) => v !== undefined && Number.isFinite(v)))
    throw new Error(`could not sample process ${pid}: "${out}"`);
  return {
    privateMB: (bytes ?? NaN) / 1024 / 1024,
    cpuMs: cpu ?? NaN,
    atMs: at ?? NaN,
    processes: n ?? NaN,
  };
}

/** CPU used between two samples, in % of one core. */
function cpuPercent(first: Sample, last: Sample): number {
  return ((last.cpuMs - first.cpuMs) / (last.atMs - first.atMs)) * 100;
}

/** Polls `check` until it holds; fails after `seconds`. */
async function waitFor(what: string, seconds: number, check: () => boolean): Promise<void> {
  const deadline = performance.now() + seconds * 1000;
  while (!check()) {
    if (performance.now() > deadline) throw new Error(`timed out after ${seconds} s waiting for ${what}`);
    await sleep(250);
  }
}

/** A throwaway data and home folder, so a measured app never reads the user's Warden data or accounts. */
function scratchEnv(scratch: string): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    USERPROFILE: scratch,
    HOME: scratch,
    APPDATA: join(scratch, 'AppData', 'Roaming'),
    LOCALAPPDATA: join(scratch, 'AppData', 'Local'),
  };
  delete env.CLAUDE_CONFIG_DIR;
  delete env.CODEX_HOME;
  delete env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS;
  delete env.WEBVIEW2_USER_DATA_FOLDER;
  return env;
}

/** A scratch folder, so the budgets never read or write the user's Warden data (%APPDATA%/Warden) or accounts. */
function tempDataDir(tag: string): string {
  return mkdtempSync(join(tmpdir(), `warden-budget-${tag}-`));
}

/** The status hook's arguments (warden_core::hooks::status_handler): `warden.exe event` for the account. */
function hookArgs(data: string, account: string): string[] {
  return ['event', '--account', account, '--data-dir', data];
}

/** A made-up PreToolUse event of the size Claude sends, with a tool input Warden must skip. */
const HOOK_EVENT = JSON.stringify({
  session_id: 'budget-1',
  transcript_path: 'transcript.jsonl',
  cwd: tmpdir(),
  hook_event_name: 'PreToolUse',
  tool_name: 'Edit',
  tool_input: { file_path: 'a.ts', old_string: 'x'.repeat(2000), new_string: 'y'.repeat(2000) },
});

/** The median time of one status hook event: `warden event` started, the event read, sent and answered. */
function measureHook(data: string, running: boolean): number {
  const times: number[] = [];
  const exe = join(RELEASE, 'warden.exe');
  for (let i = 0; i < HOOK_RUNS; i++) {
    const start = performance.now();
    const r = spawnSync(exe, hookArgs(data, 'a1'), { input: HOOK_EVENT, stdio: ['pipe', 'pipe', 'ignore'] });
    times.push(performance.now() - start);
    if (r.error) throw r.error;
    // Running or not, the hook exits 0 and prints nothing.
    if (r.status !== 0 || r.stdout.length > 0)
      throw new Error(`warden event exited with ${r.status} while Warden was ${running ? 'running' : 'stopped'}`);
  }
  times.sort((a, b) => a - b);
  return times[Math.floor(times.length / 2)] ?? NaN;
}

/**
 * Where the development server listens and the cookie its token trades for, from the address it prints
 * ("warden-server: open http://127.0.0.1:<port>/?token=<token>").
 */
async function listening(child: ChildProcess): Promise<{ port: number; cookie: string }> {
  return new Promise((resolve, reject) => {
    let text = '';
    const timer = setTimeout(() => reject(new Error('warden-server did not say where it listens')), 10_000);
    child.stderr?.on('data', (chunk: Buffer) => {
      text += chunk.toString();
      const m = /open http:\/\/127\.0\.0\.1:(\d+)\/\?token=([0-9a-f]+)/.exec(text);
      if (m) {
        clearTimeout(timer);
        resolve({ port: Number(m[1]), cookie: `warden-token-${m[1]}=${m[2]}` });
      }
    });
  });
}

/** Fills the core's session table to its cap, each session with a tool in use and a subagent. */
async function fillSessions(data: string): Promise<void> {
  // Each connection closed with its answer, so none is torn down later, inside the idle window.
  const send = (body: Record<string, unknown>) => pipeSend(data, 'a1', body);
  for (let i = 0; i < SESSIONS; i++) {
    const session_id = `fill-${i}`;
    await send({ session_id, hook_event_name: 'SessionStart', cwd: tmpdir() });
    await send({ session_id, hook_event_name: 'PreToolUse', tool_name: 'Bash' });
    await send({ session_id, hook_event_name: 'SubagentStart', agent_id: `x${i}`, agent_type: 'Explore' });
  }
}

/**
 * The idle core with its session table full, then the status hook's time against it and with Warden stopped. A
 * throwaway data and home folder, with one made-up Codex account (no network), so it never reads the user's Warden
 * data or accounts.
 */
async function measureCore(): Promise<{ memory: number; cpu: number; hook: number; hookDown: number }> {
  const scratch = mkdtempSync(join(tmpdir(), 'warden-budget-'));
  const env: NodeJS.ProcessEnv = { ...process.env, USERPROFILE: scratch, HOME: scratch };
  delete env.CLAUDE_CONFIG_DIR;
  delete env.CODEX_HOME;
  const data = join(scratch, 'data');
  const codex = join(scratch, 'codex-budget');
  mkdirSync(join(codex, 'sessions'), { recursive: true });
  mkdirSync(data, { recursive: true });
  const account = { id: 'a1', label: 'Budget', provider: 'codex', folder: codex, enabled: true, identity: null };
  writeFileSync(
    join(data, 'settings.json'),
    JSON.stringify({ version: 2, setupDone: true, accounts: [account], nextId: 2 }),
  );
  const args = ['--port', '0', '--data-dir', data];
  const child = spawn(join(RELEASE, 'warden-server.exe'), args, { stdio: ['ignore', 'ignore', 'pipe'], env });
  if (child.pid === undefined) throw new Error('warden-server did not start');
  try {
    await listening(child);
    await pipeReady(data);
    await fillSessions(data);
    await sleep(1000);
    const first = sample(child.pid, false);
    await sleep(IDLE_SECONDS * 1000);
    const memory = sample(child.pid, false).privateMB;
    await sleep((IDLE_CPU_SECONDS - IDLE_SECONDS) * 1000);
    const last = sample(child.pid, false);
    const hook = measureHook(data, true);
    return {
      memory,
      cpu: cpuPercent(first, last),
      hook,
      hookDown: await stopped(child, () => measureHook(data, false)),
    };
  } finally {
    child.kill();
    await sleep(200);
    rmSync(scratch, { recursive: true, force: true });
  }
}

const EVENT_RUNS = 1000;
const EVENT_WINDOWS = 4;

/**
 * The core's CPU time per status hook event while several windows are open: three made-up accounts (two Claude, one
 * Codex, no logins, so nothing is requested) with hook files of a usual size, four windows each holding the long poll
 * for changes and asking again as soon as it answers, and events that each change what a session is doing. Each event
 * waits until every window has had its answer, so every event costs what it costs with the windows open.
 */
async function measureEvents(): Promise<number> {
  const scratch = mkdtempSync(join(tmpdir(), 'warden-budget-events-'));
  const env = scratchEnv(scratch);
  const data = join(scratch, 'data');
  mkdirSync(data, { recursive: true });
  const userHook = (n: number) => ({ hooks: [{ type: 'command', command: `echo user hook ${n}`, timeout: 5 }] });
  const events = ['SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PostToolUse', 'Stop', 'SubagentStart'];
  const hooks = Object.fromEntries(events.map((e, i) => [e, [userHook(i)]]));
  const claudeSettings = JSON.stringify(
    {
      permissions: { allow: Array.from({ length: 40 }, (_, i) => `Bash(tool-${i} *)`), deny: [] },
      env: { EXAMPLE: '1' },
      hooks,
    },
    null,
    2,
  );
  const codexConfig = [
    'model = "example"',
    ...events.map((e, i) => `[hooks.state.'example:${e}:0:0']\nenabled = true\ntrusted_hash = "${i}"`),
  ].join('\n');
  const account = (id: string, provider: string, n: number) => {
    const folder = join(scratch, id);
    mkdirSync(folder);
    if (provider === 'claude') writeFileSync(join(folder, 'settings.json'), claudeSettings);
    else {
      writeFileSync(join(folder, 'hooks.json'), JSON.stringify({ hooks }, null, 2));
      writeFileSync(join(folder, 'config.toml'), codexConfig);
    }
    return { id, label: `Account ${n}`, provider, folder, enabled: true, identity: null };
  };
  const accounts = [account('a1', 'claude', 1), account('a2', 'claude', 2), account('a3', 'codex', 3)];
  writeFileSync(join(data, 'settings.json'), JSON.stringify({ version: 2, setupDone: true, accounts, nextId: 4 }));
  const args = ['--port', '0', '--data-dir', data];
  const child = spawn(join(RELEASE, 'warden-server.exe'), args, { stdio: ['ignore', 'ignore', 'pipe'], env });
  if (child.pid === undefined) throw new Error('warden-server did not start');
  let open = true;
  const answers = new Array<number>(EVENT_WINDOWS).fill(0);
  const windows: Promise<void>[] = [];
  try {
    const { port, cookie } = await listening(child);
    await pipeReady(data);
    const base = `http://127.0.0.1:${port}/api`;
    // Session sK belongs to account a(K % 3 + 1).
    const send = (k: number, body: Record<string, unknown>) =>
      pipeSend(data, `a${(k % 3) + 1}`, { session_id: `s${k}`, ...body });
    for (let i = 0; i < 10; i++) await send(i, { hook_event_name: 'SessionStart', cwd: tmpdir() });
    await sleep(500);
    for (let w = 0; w < EVENT_WINDOWS; w++) {
      windows.push(
        (async () => {
          let since = 0;
          while (open) {
            const r = await fetch(`${base}/state/wait?since=${since}`, { headers: { cookie } });
            since = ((await r.json()) as { revision: number }).revision;
            answers[w] = (answers[w] ?? 0) + 1;
          }
        })(),
      );
    }
    const settled = async (before: number[]) => {
      const deadline = performance.now() + 2000;
      while (answers.some((a, w) => a <= (before[w] ?? 0)) && performance.now() < deadline) await sleep(1);
    };
    await settled(answers.map(() => 0));
    const first = sample(child.pid, false);
    for (let i = 0; i < EVENT_RUNS; i++) {
      const before = [...answers];
      await send(i % 10, {
        hook_event_name: 'PreToolUse',
        tool_name: i % 20 < 10 ? 'Bash' : 'Edit',
      });
      await settled(before);
    }
    const last = sample(child.pid, false);
    if (answers.some((a) => a < EVENT_RUNS))
      throw new Error(`the windows had ${answers.join(', ')} answers for ${EVENT_RUNS} events`);
    return (last.cpuMs - first.cpuMs) / EVENT_RUNS;
  } finally {
    open = false;
    child.kill();
    await Promise.allSettled(windows);
    await sleep(200);
    rmSync(scratch, { recursive: true, force: true });
  }
}

/** Runs `measure` once the server has exited. */
async function stopped<T>(child: ChildProcess, measure: () => T): Promise<T> {
  const exited = new Promise((r) => child.once('exit', r));
  child.kill();
  await exited;
  return measure();
}

const WINDOW_LOAD_SECONDS = 8;
const CLOSE_SECONDS = 4;
/** The longest wait for the window's WebView2 processes to start or to exit. */
const OPEN_LIMIT_SECONDS = 30;

/** What the desktop app logs once its window's page has loaded under its Content-Security-Policy (shell.rs). */
const PAGE_LOADED = [
  'the window loaded its page',
  "the window's page loaded its styles",
  "the window's page reached the API",
];

/** Closes the app's window as the window's own Close does (WM_CLOSE to its top-level "Warden" window). */
function closeWindow(pid: number): void {
  const script = [
    'Add-Type -Namespace W -Name U -MemberDefinition \'[DllImport("user32.dll")] public static extern bool PostMessage(System.IntPtr h, uint m, System.IntPtr w, System.IntPtr l); public delegate bool P(System.IntPtr h, System.IntPtr l); [DllImport("user32.dll")] public static extern bool EnumWindows(P f, System.IntPtr l); [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr h, out uint p); [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(System.IntPtr h, System.Text.StringBuilder s, int n);\'',
    `$target = ${pid}; $found = 0`,
    '[W.U]::EnumWindows({ param($h, $l) $p = 0; [void][W.U]::GetWindowThreadProcessId($h, [ref]$p); $t = New-Object System.Text.StringBuilder 64; [void][W.U]::GetWindowText($h, $t, 64); if ($p -eq $target -and $t.ToString() -eq "Warden") { [void][W.U]::PostMessage($h, 0x10, [IntPtr]::Zero, [IntPtr]::Zero); $script:found++ }; $true }, [IntPtr]::Zero) | Out-Null',
    '$found',
  ].join('; ');
  const found = Number(
    execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], { encoding: 'utf8' }).trim(),
  );
  if (!found) throw new Error('the desktop window was not found to close');
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/**
 * The desktop app, in a throwaway data and home folder (WebView2's profile goes into the data folder too), with
 * setup already done and no accounts. Also checks the window's lifecycle, since the budgets depend on it:
 * - Keep running in the tray off: the window open (the app and its WebView2 processes, after the page loaded and sat
 *   idle) is measured; closing the window then quits Warden.
 * - On: closing the window leaves the app running alone (no WebView2 process: the window's memory is freed); that
 *   idle app is measured. Started as Windows would at sign-in, it opens no window.
 * Each window must load its page through the app's own scheme: the page, its styles and its first API answer, which
 * a Content-Security-Policy that blocked the scripts, styles or API would stop.
 */
async function measureDesktop(): Promise<{ window: number; tray: number; trayCpu: number }> {
  const exe = join(RELEASE, 'warden-desktop.exe');
  async function launch(
    settings: object,
    args: string[],
    measure: (pid: number, log: () => string) => Promise<void>,
  ): Promise<void> {
    const scratch = mkdtempSync(join(tmpdir(), 'warden-budget-desktop-'));
    const data = join(scratch, 'data');
    mkdirSync(data, { recursive: true });
    writeFileSync(join(data, 'settings.json'), JSON.stringify({ ...settings, version: 2 }));
    const child = spawn(exe, ['--data-dir', data, ...args], {
      stdio: ['ignore', 'ignore', 'pipe'],
      env: scratchEnv(scratch),
    });
    if (child.pid === undefined) throw new Error('warden-desktop did not start');
    let log = '';
    child.stderr?.on('data', (chunk: Buffer) => (log += chunk.toString()));
    try {
      await measure(child.pid, () => log);
    } finally {
      spawnSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
      await sleep(1000);
      rmSync(scratch, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
    }
  }
  let window = NaN;
  let tray = NaN;
  let trayCpu = NaN;
  // The window opens (a WebView2 process) within this, even on a machine busy with a build.
  const opened = async (pid: number, log: () => string) => {
    await waitFor('the desktop window to open', OPEN_LIMIT_SECONDS, () => sample(pid, true).processes >= 2);
    await waitFor('the window to load its page, its styles and its first API answer', OPEN_LIMIT_SECONDS, () =>
      PAGE_LOADED.every((line) => log().includes(line)),
    );
  };
  await launch({ setupDone: true }, [], async (pid, log) => {
    await opened(pid, log);
    await sleep((WINDOW_LOAD_SECONDS + IDLE_SECONDS) * 1000);
    window = sample(pid, true).privateMB;
    closeWindow(pid);
    await sleep(CLOSE_SECONDS * 1000);
    if (alive(pid)) throw new Error('with Keep running in the tray off, closing the window should quit Warden');
  });
  await launch({ setupDone: true, keepInTray: true }, [], async (pid, log) => {
    await opened(pid, log);
    await sleep(WINDOW_LOAD_SECONDS * 1000);
    closeWindow(pid);
    await sleep(CLOSE_SECONDS * 1000);
    // WebView2 closes on its own time; on a busy machine that can take longer than the pause above.
    await waitFor(
      "the window's WebView2 processes to exit",
      OPEN_LIMIT_SECONDS,
      () => sample(pid, true).processes === 1,
    );
    if (!alive(pid))
      throw new Error('with Keep running in the tray on, closing the window should leave Warden running');
    const first = sample(pid, true);
    await sleep(IDLE_SECONDS * 1000);
    tray = sample(pid, true).privateMB;
    await sleep((IDLE_CPU_SECONDS - IDLE_SECONDS) * 1000);
    const last = sample(pid, true);
    if (last.processes !== 1)
      throw new Error(`with the window closed the app should run alone, found ${last.processes} processes`);
    trayCpu = cpuPercent(first, last);
  });
  await launch({ setupDone: true, keepInTray: true }, ['--autostart'], async (pid) => {
    await sleep(CLOSE_SECONDS * 1000);
    const n = sample(pid, true).processes;
    if (n !== 1)
      throw new Error(`started at sign-in with the tray on, the app should open no window, found ${n} processes`);
  });
  return { window, tray, trayCpu };
}

/**
 * Median time of `warden route` as the hook runs it: the hook's JSON on stdin, three fixture accounts (no logins, so
 * nothing is ever requested) and fresh readings. `silent`: the same session each time, so after the first run it
 * prints nothing (every message while nothing changes). Otherwise a new session each time, so it prints the guidance
 * (a session's first message, or a change).
 */
function measureRoute(silent: boolean): number {
  const exe = join(RELEASE, 'warden.exe');
  const data = tempDataDir('route');
  try {
    const now = Date.now();
    const account = (id: string, provider: string, n: number) => ({
      id,
      label: `Account ${n}`,
      provider,
      folder: join(data, id),
      enabled: true,
      identity: { email: id + '@example.com', organization: null, organizationId: null },
    });
    const accounts = [account('a1', 'claude', 1), account('a2', 'claude', 2), account('a3', 'codex', 3)];
    for (const a of accounts) mkdirSync(a.folder);
    writeFileSync(
      join(data, 'settings.json'),
      JSON.stringify({
        version: 2,
        setupDone: true,
        main: { label: 'Main', identity: { email: 'main@example.com', organization: null, organizationId: null } },
        accounts,
        nextId: 4,
      }),
    );
    const reading = (five: number | null, week: number) => ({
      usage: {
        readAt: now,
        fiveHour: five === null ? null : { usedPercent: five, resetsAt: now + 3_600_000 },
        weekly: { usedPercent: week, resetsAt: now + 100 * 3_600_000 },
        perModelWeekly: [],
        extra: null,
      },
    });
    writeFileSync(
      join(data, 'readings.json'),
      JSON.stringify({
        mainLogin: { signIn: { state: 'signedIn', email: 'main@example.com' } },
        accounts: { main: reading(61, 11), a1: reading(61, 11), a2: reading(83, 38), a3: reading(null, 31) },
        checkedAt: now,
      }),
    );
    const home = join(data, 'home');
    mkdirSync(join(home, '.claude'), { recursive: true });
    const env: NodeJS.ProcessEnv = { ...process.env, USERPROFILE: home, HOME: home };
    delete env.CLAUDE_CONFIG_DIR;
    delete env.CODEX_HOME;
    const times: number[] = [];
    for (let i = 0; i <= ROUTE_RUNS; i++) {
      const input = JSON.stringify({
        session_id: silent ? 'budget' : `budget-${i}`,
        hook_event_name: 'UserPromptSubmit',
        prompt: 'hi',
      });
      const start = performance.now();
      const r = spawnSync(exe, ['route', '--account', 'main', '--data-dir', data], { input, encoding: 'utf8', env });
      const took = performance.now() - start;
      if (r.status !== 0) throw new Error(`warden route exited with ${r.status}`);
      const printed = r.stdout.length > 0;
      if (i === 0) {
        if (!printed) throw new Error('warden route printed nothing for a new session');
        continue; // the first run warms the file cache and tells the session
      }
      if (printed === silent)
        throw new Error(
          `warden route ${silent ? 'printed while nothing changed' : 'printed nothing for a new session'}`,
        );
      times.push(took);
    }
    times.sort((a, b) => a - b);
    return times[Math.floor(times.length / 2)] ?? NaN;
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
}

/** Release parsers must reject development switches before starting a core or a window. */
function releaseRefusesDevelopmentOptions(): void {
  const scratch = mkdtempSync(join(tmpdir(), 'warden-release-options-'));
  try {
    for (const binary of ['warden-server.exe', 'warden-desktop.exe', 'warden.exe']) {
      for (const args of [['--no-hooks'], ['--home', scratch]]) {
        const result = spawnSync(join(RELEASE, binary), ['--data-dir', join(scratch, 'data'), ...args], {
          env: scratchEnv(scratch),
          timeout: 3000,
          windowsHide: true,
          encoding: 'utf8',
        });
        if (result.error || result.status === null || result.status === 0) {
          throw new Error(`${binary} did not refuse ${args[0]} immediately`);
        }
      }
    }
    console.log('release: server, desktop and CLI refuse --no-hooks and --home');
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

async function main(): Promise<void> {
  const budgets = JSON.parse(readFileSync(join(ROOT, 'checks', 'budgets.json'), 'utf8')) as Record<string, Budget>;
  // The desktop window shows the built UI (ui/dist), found beside target/release.
  if (
    !run('cargo', ['build', '--release', '--workspace', '--bins', '--quiet']).ok ||
    !run('npm', ['run', '--silent', 'build', '-w', 'ui']).ok
  ) {
    process.exitCode = 1;
    return;
  }
  releaseRefusesDevelopmentOptions();
  const core = await measureCore();
  const events = await measureEvents();
  const desktop = await measureDesktop();
  const measured: Record<string, Measured> = {
    coreIdleMemoryMB: core.memory,
    coreIdleCpuPercent: core.cpu,
    windowMemoryMB: desktop.window,
    desktopTrayMemoryMB: desktop.tray,
    desktopTrayCpuPercent: desktop.trayCpu,
    routeHookMs: measureRoute(true),
    routeHookPrintMs: measureRoute(false),
    statusHookMs: core.hook,
    statusHookDownMs: core.hookDown,
    statusEventCpuMs: events,
    serverBinaryKB: sizeKB('warden-server.exe'),
    cliBinaryKB: sizeKB('warden.exe'),
    desktopBinaryKB: sizeKB('warden-desktop.exe'),
  };

  let over = 0;
  for (const [key, b] of Object.entries(budgets)) {
    if (key.startsWith('$')) continue;
    const m = measured[key];
    if (m === undefined) throw new Error(`budgets.json has "${key}" but budgets.ts does not measure it`);
    const value = typeof m === 'number' ? `${m.toFixed(m < 10 ? 2 : 0)} ${b.unit}` : `not measurable yet (${m.notYet})`;
    let verdict: string;
    if (typeof m !== 'number') verdict = 'n/a ';
    else if (b.budget === null) verdict = 'PLACEHOLDER';
    else if (m <= b.budget) verdict = 'ok  ';
    else {
      verdict = 'OVER';
      over++;
    }
    const limit = b.budget === null ? 'no budget yet' : `budget ${b.budget} ${b.unit}`;
    console.log(`${verdict.padEnd(11)} ${key.padEnd(22)} ${value.padEnd(24)} ${limit}  (${b.what})`);
  }
  mkdirSync(join(ROOT, 'artifacts'), { recursive: true });
  writeFileSync(join(ROOT, 'artifacts', 'budgets.json'), JSON.stringify({ measured, budgets }, null, 2));
  console.log(`budgets: ${over} over budget; placeholders are reported only`);
  if (over) process.exitCode = 1;
}

await main();
