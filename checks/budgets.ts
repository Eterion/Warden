// Performance budget tests (warden-plan.md, Performance, Budgets). Builds the release binaries, measures, and
// compares each number with checks/budgets.json. Budgets still null are placeholders: reported, never failed.
// Results are also written to artifacts/budgets.json for the step's approval summary.

import { type ChildProcess, execFile, execFileSync, spawn, spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
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
/** The demo server (crates/warden-server/examples/demo.rs), built in release: the core with its loopback listener. */
const DEMO = join(RELEASE, 'examples', 'demo.exe');
const IDLE_SECONDS = 5;
/**
 * Idle CPU is measured over this long. Windows charges CPU time in whole clock ticks (15.6 ms) to whichever thread is
 * running when the tick comes, so an idle process's brief wake-ups (about 0.1 ms a second for the core) read as 0 or
 * a whole tick: over 5 s one tick is 0.31% of a core and two are 0.62%, over budget. Over 20 s a tick is 0.08%.
 */
const IDLE_CPU_SECONDS = 20;
const ROUTE_RUNS = 41;
/**
 * The pause after each round of `warden route` runs, so the rounds spread over about 12 s. Windows Defender looks at
 * every program Windows starts, and while it falls behind (a build or another check writing many new files can keep
 * it busy for seconds) any program, `warden route` or Windows' own hostname.exe alike, takes 20 to 70 ms to start
 * instead of 7 to 9. Back to back, 21 runs took 0.15 s, so one such burst could hold the whole median; spread out, a
 * burst of a few seconds holds a few runs, and the median is Warden's own time.
 */
const ROUTE_PAUSE_MS = 250;
const ROUTE_ATTEMPTS = 8;
const ROUTE_SETTLE_MS = 30_000;
const ROUTE_RETRY_MARGIN_MS = 3_000;
const ROUTE_LAUNCH_TIMEOUT_MS = 3_000;
const ROUTE_DEADLINE_MS = 300_000;
// Leave time for rm's five retries, whose Windows sharing-violation backoff can total 7.5 s.
const ROUTE_CLEANUP_MS = 10_000;
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
  return median(times);
}

/**
 * Starts the demo server on a free port with this data folder, the scratch folder as its home, and empty folders for
 * its stand-in usage answers and its UI: the core with its loopback listener, stand-in endpoints and no logins.
 */
function startDemo(scratch: string, data: string, env: NodeJS.ProcessEnv): ChildProcess & { pid: number } {
  const usage = mkdtempSync(join(scratch, 'usage-'));
  const ui = mkdtempSync(join(scratch, 'ui-'));
  const args = ['--port', '0', '--data-dir', data, '--home', scratch, '--usage-dir', usage, '--ui-dir', ui];
  const child = spawn(DEMO, args, { stdio: ['ignore', 'ignore', 'pipe'], env });
  if (child.pid === undefined) throw new Error('the demo server did not start');
  return child as ChildProcess & { pid: number };
}

/**
 * Where the demo server listens and the cookie its token trades for, from the address it prints
 * ("demo: open http://127.0.0.1:<port>/?token=<token>").
 */
async function listening(child: ChildProcess): Promise<{ port: number; cookie: string }> {
  return new Promise((resolve, reject) => {
    // The demo writes its line in pieces, so only whole lines are matched (as checks/demo.ts does): a token cut at a
    // chunk's end would make every request after it unauthenticated. The unfinished piece waits for the next chunk.
    let pending = '';
    const timer = setTimeout(() => reject(new Error('the demo server did not say where it listens')), 10_000);
    child.stderr?.on('data', (chunk: Buffer) => {
      const lines = (pending + chunk.toString()).split('\n');
      pending = lines.pop() ?? '';
      for (const line of lines) {
        const m = /^demo: open http:\/\/127\.0\.0\.1:(\d+)\/\?token=([0-9a-f]+)$/.exec(line.trim());
        if (m) {
          clearTimeout(timer);
          resolve({ port: Number(m[1]), cookie: `warden-token-${m[1]}=${m[2]}` });
        }
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
  fullMemoryDay(data);
  const child = startDemo(scratch, data, env);
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

/** Today's local date, as the core names its memory files. */
function localDate(): string {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

/** The memory series a full day holds in the budgets (crates/warden-core/src/memory.rs keeps up to 200). */
const MEMORY_SERIES = 20;

/**
 * Today's memory file, full: 1,440 minutes with 20 series each, so the core keeps today's whole buffer in memory
 * (it reads today's file back at start).
 */
function fullMemoryDay(data: string): void {
  const lines: string[] = [];
  for (let n = 1; n <= MEMORY_SERIES; n++) {
    lines.push(
      JSON.stringify({
        series: n,
        kind: 'session',
        provider: 'codex',
        accountId: 'a1',
        account: 'Budget',
        sessionId: `budget-${n}`,
        parentSessionId: null,
        name: `Budget session ${n}`,
        project: 'Budget',
      }),
    );
  }
  for (let m = 0; m < 1440; m++) {
    const s = Array.from({ length: MEMORY_SERIES }, (_, i) => [i + 1, 100_000 + ((m * 37 + i * 101) % 50_000)]);
    const sum = s.reduce((a, [, kib]) => a + (kib ?? 0), 0);
    lines.push(JSON.stringify({ m, total: 33_554_432, used: 18_000_000 + m, sum, s }));
  }
  mkdirSync(join(data, 'memory'), { recursive: true });
  writeFileSync(join(data, 'memory', `memory-${localDate()}.jsonl`), lines.join('\n') + '\n');
}

/** Processes in the stand-in session's tree under its CLI (the CLI, ping processes and the hook). */
const TREE_PROCESSES = 20;
const SAMPLING_SECONDS = 20;

/**
 * The core's cost of sampling memory on this computer while a session runs: the demo server sampling this computer
 * (`--real-memory-every-ms`), with one Claude session whose CLI is a stand-in (a copy of Node, which Warden takes for
 * Claude's CLI) running a tree of ping processes, its SessionStart sent by the real status hook (`warden event`) from
 * inside that tree, so Warden finds the CLI as it would a real one. `every` is the sampling interval; returns the
 * core's CPU time over SAMPLING_SECONDS and the number of samples in it.
 */
async function measureSampling(everyMs: number): Promise<{ cpuMs: number; samples: number }> {
  const scratch = mkdtempSync(join(tmpdir(), 'warden-budget-memory-'));
  const env = scratchEnv(scratch);
  const data = join(scratch, 'data');
  const folder = join(scratch, 'claude-budget');
  mkdirSync(data, { recursive: true });
  mkdirSync(folder, { recursive: true });
  const account = { id: 'a1', label: 'Budget', provider: 'claude', folder, enabled: true, identity: null };
  writeFileSync(
    join(data, 'settings.json'),
    JSON.stringify({ version: 2, setupDone: true, accounts: [account], nextId: 2 }),
  );
  const usage = mkdtempSync(join(scratch, 'usage-'));
  const ui = mkdtempSync(join(scratch, 'ui-'));
  const args = ['--port', '0', '--data-dir', data, '--home', scratch, '--usage-dir', usage, '--ui-dir', ui];
  const child = spawn(DEMO, [...args, '--real-memory-every-ms', String(everyMs)], {
    stdio: ['ignore', 'ignore', 'pipe'],
    env,
  }) as ChildProcess & { pid: number };
  if (child.pid === undefined) throw new Error('the demo server did not start');
  // The stand-in CLI: pings that idle under it, then the status hook's SessionStart from inside its tree.
  const event = JSON.stringify({ session_id: 'budget-memory', hook_event_name: 'SessionStart', cwd: scratch });
  const script = [
    "const { spawn } = require('node:child_process');",
    `for (let i = 0; i < ${TREE_PROCESSES - 2}; i++) spawn('ping', ['-n', '600', '127.0.0.1'], { stdio: 'ignore', windowsHide: true });`,
    `const hook = spawn(${JSON.stringify(join(RELEASE, 'warden.exe'))}, ${JSON.stringify(hookArgs(data, 'a1'))}, { stdio: ['pipe', 'ignore', 'ignore'], windowsHide: true });`,
    `hook.stdin.end(${JSON.stringify(event)});`,
    'setInterval(() => {}, 1000);',
  ].join('\n');
  let cli: ChildProcess | undefined;
  try {
    const { port, cookie } = await listening(child);
    await pipeReady(data);
    cli = spawn(process.execPath, ['-e', script], { stdio: 'ignore', windowsHide: true, env });
    const date = localDate();
    const measured = async () => {
      const r = await fetch(`http://127.0.0.1:${port}/api/memory/day?date=${date}`, { headers: { cookie } });
      const day = (await r.json()) as { series: { sessionId: string | null }[] };
      return day.series.some((s) => s.sessionId === 'budget-memory');
    };
    const deadline = performance.now() + 30_000;
    while (!(await measured())) {
      if (performance.now() > deadline) throw new Error("the stand-in session's memory was never sampled");
      await sleep(250);
    }
    const tree = cli.pid === undefined ? 0 : sample(cli.pid, true).processes;
    if (tree < TREE_PROCESSES - 1)
      throw new Error(`the stand-in tree has ${tree} processes, ${TREE_PROCESSES - 1} expected`);
    const first = sample(child.pid, false);
    await sleep(SAMPLING_SECONDS * 1000);
    const last = sample(child.pid, false);
    return { cpuMs: last.cpuMs - first.cpuMs, samples: (last.atMs - first.atMs) / everyMs };
  } finally {
    if (cli?.pid !== undefined) spawnSync('taskkill', ['/PID', String(cli.pid), '/T', '/F'], { stdio: 'ignore' });
    child.kill();
    await sleep(500);
    rmSync(scratch, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
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
  const child = startDemo(scratch, data, env);
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

/** Runs `measure` once the demo server has exited. */
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

/**
 * Runs `steps` (PowerShell) on the window's Windows UI Automation tree (the WebView2's accessibility tree: `$root`, `$T`
 * for its descendants, `$named` for a name condition); they end by printing `done`. No debugging port is opened.
 */
function inWindow(pid: number, steps: string[], done: string): void {
  const script = [
    'Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes',
    'Add-Type -Namespace W -Name U -MemberDefinition \'public delegate bool P(System.IntPtr h, System.IntPtr l); [DllImport("user32.dll")] public static extern bool EnumWindows(P f, System.IntPtr l); [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr h, out uint p); [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(System.IntPtr h, System.Text.StringBuilder s, int n);\'',
    `$target = ${pid}; $script:win = [IntPtr]::Zero`,
    '[W.U]::EnumWindows({ param($h, $l) $p = 0; [void][W.U]::GetWindowThreadProcessId($h, [ref]$p); $t = New-Object System.Text.StringBuilder 64; [void][W.U]::GetWindowText($h, $t, 64); if ($p -eq $target -and $t.ToString() -eq "Warden") { $script:win = $h }; $true }, [IntPtr]::Zero) | Out-Null',
    'if ($script:win -eq [IntPtr]::Zero) { throw "the desktop window was not found" }',
    '$A = [System.Windows.Automation.AutomationElement]; $T = [System.Windows.Automation.TreeScope]::Descendants',
    '$root = $A::FromHandle($script:win)',
    '$named = { param($n) New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, $n) }',
    ...steps,
  ].join('; ');
  const out = execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], {
    encoding: 'utf8',
  }).trim();
  if (!out.endsWith(done)) throw new Error(`could not reach ${done}: ${out}`);
}

/**
 * Waits for the Overview's Memory card to draw today: its chart, a link named by what it shows ("AI sessions' memory
 * today: …").
 */
function waitForMemoryCard(pid: number): void {
  inWindow(
    pid,
    [
      `$card = $null; for ($i = 0; $i -lt 60 -and -not $card; $i++) { $card = $root.FindAll($T, (New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::Hyperlink))) | Where-Object { $_.Current.Name -like "AI sessions' memory today*" } | Select-Object -First 1; if (-not $card) { Start-Sleep -Milliseconds 500 } }`,
      'if (-not $card) { throw "the Overview did not show its Memory card\'s chart" }',
      '"the Memory card"',
    ],
    'the Memory card',
  );
}

/** Opens the window's Memory page as a user would: its sidebar link, invoked; then waits for the page's chart. */
function openMemoryPage(pid: number): void {
  inWindow(
    pid,
    [
      '$link = $null; for ($i = 0; $i -lt 60 -and -not $link; $i++) { $link = $root.FindFirst($T, (New-Object System.Windows.Automation.AndCondition((& $named "Memory"), (New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::Hyperlink))))); if (-not $link) { Start-Sleep -Milliseconds 500 } }',
      'if (-not $link) { throw "the sidebar link to the Memory page was not found" }',
      '$link.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()',
      '$chart = $null; for ($i = 0; $i -lt 60 -and -not $chart; $i++) { $chart = $root.FindFirst($T, (& $named "Memory over the day")); if (-not $chart) { Start-Sleep -Milliseconds 500 } }',
      'if (-not $chart) { throw "the Memory page did not show its chart" }',
      '"the Memory page"',
    ],
    'the Memory page',
  );
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
 * - Keep running in the tray off: the window open on the Overview, then on the Memory page, with a full day (the app
 *   and its WebView2 processes, after each page loaded and sat idle; the higher counts) is measured; closing the
 *   window then quits Warden.
 * - On: closing the window leaves the app running alone (no WebView2 process: the window's memory is freed); that
 *   idle app is measured. Started as Windows would at sign-in, it opens no window.
 * Each window must load its page through the app's own scheme: the page, its styles and its first API answer, which
 * a Content-Security-Policy that blocked the scripts, styles or API would stop.
 */
async function measureDesktop(): Promise<{ window: number; windowCpu: number; tray: number; trayCpu: number }> {
  const exe = join(RELEASE, 'warden-desktop.exe');
  async function launch(
    settings: object,
    args: string[],
    measure: (pid: number, log: () => string, data: string) => Promise<void>,
    prepare?: (data: string) => void,
  ): Promise<void> {
    const scratch = mkdtempSync(join(tmpdir(), 'warden-budget-desktop-'));
    const data = join(scratch, 'data');
    mkdirSync(data, { recursive: true });
    writeFileSync(join(data, 'settings.json'), JSON.stringify({ ...settings, version: 2 }));
    prepare?.(data);
    const child = spawn(exe, ['--data-dir', data, ...args], {
      stdio: ['ignore', 'ignore', 'pipe'],
      env: scratchEnv(scratch),
    });
    if (child.pid === undefined) throw new Error('warden-desktop did not start');
    let log = '';
    child.stderr?.on('data', (chunk: Buffer) => (log += chunk.toString()));
    try {
      await measure(child.pid, () => log, data);
    } finally {
      spawnSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
      await sleep(1000);
      rmSync(scratch, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
    }
  }
  let window = NaN;
  let windowCpu = NaN;
  let tray = NaN;
  let trayCpu = NaN;
  // The window opens (a WebView2 process) within this, even on a machine busy with a build.
  const opened = async (pid: number, log: () => string) => {
    await waitFor('the desktop window to open', OPEN_LIMIT_SECONDS, () => sample(pid, true).processes >= 2);
    await waitFor('the window to load its page, its styles and its first API answer', OPEN_LIMIT_SECONDS, () =>
      PAGE_LOADED.every((line) => log().includes(line)),
    );
  };
  // With the Overview open on a full day (1,440 minutes, 20 series; its Memory card draws it small and follows it), then
  // the Memory page on that day, the heaviest page: the higher of the two counts.
  await launch(
    { setupDone: true },
    [],
    async (pid, log) => {
      await opened(pid, log);
      waitForMemoryCard(pid);
      await sleep((WINDOW_LOAD_SECONDS + IDLE_SECONDS) * 1000);
      const overview = sample(pid, true).privateMB;
      openMemoryPage(pid);
      await sleep((WINDOW_LOAD_SECONDS + IDLE_SECONDS) * 1000);
      const memory = sample(pid, true).privateMB;
      window = Math.max(overview, memory);
      console.log(`window: Overview ${overview.toFixed(1)} MB, Memory page ${memory.toFixed(1)} MB`);
      closeWindow(pid);
      await sleep(CLOSE_SECONDS * 1000);
      if (alive(pid)) throw new Error('with Keep running in the tray off, closing the window should quit Warden');
    },
    fullMemoryDay,
  );
  // Use the same made-up day in a fresh desktop window and keep a demo session working throughout the CPU sample.
  await launch(
    { setupDone: true },
    [],
    async (pid, log, data) => {
      await opened(pid, log);
      await pipeReady(data);
      const session_id = 'budget-window';
      await pipeSend(data, 'main', { session_id, hook_event_name: 'SessionStart', cwd: data });
      await pipeSend(data, 'main', { session_id, hook_event_name: 'PreToolUse', tool_name: 'Bash' });
      // Give Overview time to render the acknowledged events. UI Automation would turn on accessibility in Chromium.
      await sleep(WINDOW_LOAD_SECONDS * 1000);
      const first = sample(pid, true);
      await sleep(IDLE_CPU_SECONDS * 1000);
      const last = sample(pid, true);
      if (first.processes < 2 || last.processes < 2 || last.cpuMs < first.cpuMs)
        throw new Error('the desktop window or its WebView2 processes exited during the CPU sample');
      windowCpu = cpuPercent(first, last);
    },
    fullMemoryDay,
  );
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
  return { window, windowCpu, tray, trayCpu };
}

/** The middle value (the upper one of an even count). */
function median(values: number[]): number {
  return [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)] ?? NaN;
}

interface RouteFixture {
  data: string;
  env: NodeJS.ProcessEnv;
}

interface RouteMeasurement {
  deadline: number;
  fixtures: Set<string>;
  attemptError?: unknown;
}

/**
 * A throwaway data and home folder for `warden route`: three fixture accounts (no logins, so nothing is ever
 * requested) and fresh readings.
 */
async function routeFixture(measurement: RouteMeasurement): Promise<RouteFixture> {
  routeTimeLeft(measurement.deadline);
  const data = await mkdtemp(join(tmpdir(), 'warden-budget-route-'));
  measurement.fixtures.add(data);
  routeTimeLeft(measurement.deadline);
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
  for (const a of accounts) await mkdir(a.folder);
  await writeFile(
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
  await writeFile(
    join(data, 'readings.json'),
    JSON.stringify({
      mainLogin: { signIn: { state: 'signedIn', email: 'main@example.com' } },
      accounts: { main: reading(61, 11), a1: reading(61, 11), a2: reading(83, 38), a3: reading(null, 31) },
      checkedAt: now,
    }),
  );
  const home = join(data, 'home');
  await mkdir(join(home, '.claude'), { recursive: true });
  const env: NodeJS.ProcessEnv = { ...process.env, USERPROFILE: home, HOME: home };
  delete env.CLAUDE_CONFIG_DIR;
  delete env.CODEX_HOME;
  return { data, env };
}

function routeMeasuringTimeLeft(deadline: number): number {
  return Math.floor(deadline - ROUTE_CLEANUP_MS - performance.now());
}

function routeTimeLeft(deadline: number): number {
  const remaining = routeMeasuringTimeLeft(deadline);
  if (remaining < 1) throw new Error('no route measuring time is left before the time reserved for fixture cleanup');
  return remaining;
}

/** Time each launch with the same piped stdin, stdout and stderr. */
function timedRouteLaunch(
  exe: string,
  args: string[],
  input: string,
  env: NodeJS.ProcessEnv,
  deadline: number,
): Promise<{ took: number; printed: boolean }> {
  const timeout = Math.min(ROUTE_LAUNCH_TIMEOUT_MS, routeTimeLeft(deadline));
  const start = performance.now();
  return new Promise((resolve, reject) => {
    const child = execFile(exe, args, { encoding: 'utf8', env, timeout, killSignal: 'SIGKILL' }, (error, stdout) => {
      const took = performance.now() - start;
      if (error?.killed || (!error && took >= timeout))
        reject(new Error(`${basename(exe)} timed out after ${timeout} ms`));
      else if (error) reject(error);
      else resolve({ took, printed: stdout.length > 0 });
    });
    child.stdin?.on('error', reject);
    child.stdin?.end(input);
  });
}

type RouteMode = 'silent' | 'print';
const ROUTE_MODES: readonly RouteMode[] = ['silent', 'print'];

interface RouteTimes {
  silent: number;
  print: number;
  control: number;
  seconds: number;
  attempts: number;
}

export interface RouteRound {
  silent: number;
  print: number;
  controls: [number, number, number];
}

export interface RouteAttempt {
  rounds: RouteRound[];
  times: RouteTimes;
}

interface ExcusedRouteAttempt {
  times: RouteTimes;
  baselineMs: number;
  sharedRounds: Partial<Record<RouteMode, number>>;
}

interface RouteDecision {
  times: RouteTimes;
  excusedAttempts: ExcusedRouteAttempt[];
}

/**
 * One attempt: median times of `warden route` in ROUTE_RUNS rounds spread over time (ROUTE_PAUSE_MS), each round one
 * run of each kind: `silent` the same session each time, so after its first run it prints nothing (every message while
 * nothing changes); `print` a new session each time, so it prints the guidance (a session's first message, or a
 * change). Each kind has its own fixture, new for each attempt, so a later attempt times the same work as the first
 * (the printing runs leave a session file each, which every printing run lists to prune). `control` is hostname.exe
 * started immediately before and after each measured launch, sharing the middle control to keep the rounds short.
 */
async function routeRounds(attempts: number, measurement: RouteMeasurement): Promise<RouteAttempt> {
  const { deadline, fixtures } = measurement;
  const systemRoot = process.env.SystemRoot;
  if (!systemRoot) throw new Error('SystemRoot is not set, so hostname.exe cannot be found');
  const controlExe = join(systemRoot, 'System32', 'hostname.exe');
  const runRoute = (fixture: RouteFixture, session: string) =>
    timedRouteLaunch(
      join(RELEASE, 'warden.exe'),
      ['route', '--account', 'main', '--data-dir', fixture.data],
      JSON.stringify({ session_id: session, hook_event_name: 'UserPromptSubmit', prompt: 'hi' }),
      fixture.env,
      deadline,
    );
  try {
    const silentFixture = await routeFixture(measurement);
    const printFixture = await routeFixture(measurement);
    const controlRun = async () => (await timedRouteLaunch(controlExe, [], '', silentFixture.env, deadline)).took;
    // The first run of each warms the file cache and tells the session; it is not timed.
    if (!(await runRoute(silentFixture, 'budget')).printed || !(await runRoute(printFixture, 'budget-0')).printed)
      throw new Error('warden route printed nothing for a new session');
    const rounds: RouteRound[] = [];
    const start = performance.now();
    for (let i = 1; i <= ROUTE_RUNS; i++) {
      const before = await controlRun();
      const quiet = await runRoute(silentFixture, 'budget');
      if (quiet.printed) throw new Error('warden route printed while nothing changed');
      const middle = await controlRun();
      const guided = await runRoute(printFixture, `budget-${i}`);
      if (!guided.printed) throw new Error('warden route printed nothing for a new session');
      const after = await controlRun();
      rounds.push({ silent: quiet.took, print: guided.took, controls: [before, middle, after] });
      if (i < ROUTE_RUNS) await sleep(Math.min(ROUTE_PAUSE_MS, routeTimeLeft(deadline)));
    }
    routeTimeLeft(deadline);
    return {
      rounds,
      times: {
        silent: median(rounds.map((round) => round.silent)),
        print: median(rounds.map((round) => round.print)),
        control: median(rounds.flatMap((round) => round.controls)),
        seconds: (performance.now() - start) / 1000,
        attempts,
      },
    };
  } catch (error) {
    measurement.attemptError = error;
    throw error;
  } finally {
    // Start every deletion before awaiting any of them so each gets the same time to retry Windows file locks.
    const removals = await Promise.allSettled(
      [...fixtures].map(async (data) => {
        try {
          await rm(data, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
          fixtures.delete(data);
        } catch (error) {
          throw new Error(`could not remove route fixture ${data}`, { cause: error });
        }
      }),
    );
    const failures = removals.flatMap((result) => (result.status === 'rejected' ? [result.reason] : []));
    if (failures.length) {
      if (measurement.attemptError !== undefined) failures.unshift(measurement.attemptError);
      throw new AggregateError(failures, 'route fixture cleanup failed');
    }
  }
}

/**
 * Each failing mode needs a majority of rounds where the route exceeded its budget and both neighboring controls
 * slowed enough to explain the excess. Use the passing attempt's upper quartile as the normal control time, and
 * require a rise of at least half that time so ordinary control variation cannot excuse a breach. The remaining
 * rounds must also pass their own median when any remain, so they cannot show a separate, sustained route slowdown.
 */
function sharedRouteSlowdown(
  rounds: RouteRound[],
  mode: RouteMode,
  budget: number,
  baseline: number,
): { shared: number; remainingMedian: number | undefined } {
  const remaining = rounds.filter((round) => {
    const [before, middle, after] = round.controls;
    const control = Math.min(middle, mode === 'silent' ? before : after);
    const delay = control - baseline;
    const shared = round[mode] > budget && delay >= baseline / 2 && delay >= round[mode] - budget;
    return !shared;
  });
  return {
    shared: rounds.length - remaining.length,
    remainingMedian: remaining.length ? median(remaining.map((round) => round[mode])) : undefined,
  };
}

function routeWithinBudget(times: RouteTimes, budget: Record<RouteMode, number>): boolean {
  return ROUTE_MODES.every((mode) => times[mode] <= budget[mode]);
}

/** A passing attempt replaces earlier breaches only if each failing mode has evidence of shared slowdown. */
export function selectRouteAttempt(
  breachedAttempts: readonly RouteAttempt[],
  latest: RouteAttempt,
  budget: Record<RouteMode, number>,
): RouteDecision {
  if (!breachedAttempts.length || !routeWithinBudget(latest.times, budget))
    return { times: latest.times, excusedAttempts: [] };

  // A later attempt can establish the baseline even when the first one was slow from start to finish.
  const controls = latest.rounds.flatMap((round) => round.controls).sort((a, b) => a - b);
  const baselineMs = controls[Math.floor(0.75 * controls.length)] ?? NaN;
  const excusedAttempts: ExcusedRouteAttempt[] = [];
  for (const earlier of breachedAttempts) {
    const sharedRounds: ExcusedRouteAttempt['sharedRounds'] = {};
    for (const mode of ROUTE_MODES) {
      if (earlier.times[mode] <= budget[mode]) continue;
      const { shared, remainingMedian } = sharedRouteSlowdown(earlier.rounds, mode, budget[mode], baselineMs);
      if (shared <= earlier.rounds.length / 2 || (remainingMedian ?? 0) > budget[mode])
        return { times: earlier.times, excusedAttempts: [] };
      sharedRounds[mode] = shared;
    }
    excusedAttempts.push({ times: earlier.times, baselineMs, sharedRounds });
  }
  return { times: latest.times, excusedAttempts };
}

async function routeAttempts(budget: Record<RouteMode, number>, measurement: RouteMeasurement): Promise<RouteDecision> {
  const failed: RouteAttempt[] = [];
  for (let attempt = 1; ; attempt++) {
    const started = performance.now();
    const result = await routeRounds(attempt, measurement);
    // routeRounds finishes after fixture cleanup, so this includes setup, launches, pauses and removal.
    const retryMs = performance.now() - started + ROUTE_RETRY_MARGIN_MS;
    const { times } = result;
    if (routeWithinBudget(times, budget)) {
      const decision = selectRouteAttempt(failed, result, budget);
      if (decision.times !== times)
        console.log(
          `route: attempt ${attempt} passed. Attempt ${decision.times.attempts} still has an unexplained breach.`,
        );
      return decision;
    }

    const said = `route: ${times.silent.toFixed(2)} and ${times.print.toFixed(2)} ms while hostname.exe took ${times.control.toFixed(2)} ms (median)`;
    if (attempt < ROUTE_ATTEMPTS && routeMeasuringTimeLeft(measurement.deadline) >= ROUTE_SETTLE_MS + retryMs) {
      console.log(`${said}. Keeping the breach and measuring again in ${ROUTE_SETTLE_MS / 1000} s.`);
      await sleep(ROUTE_SETTLE_MS);
      if (routeMeasuringTimeLeft(measurement.deadline) >= retryMs) {
        failed.push(result);
        continue;
      }
    }
    console.log(`${said}. No time or attempts left for another measurement.`);
    return selectRouteAttempt(failed, result, budget);
  }
}

async function measureRoute(budget: Record<RouteMode, number>): Promise<RouteDecision> {
  const measurement: RouteMeasurement = { deadline: performance.now() + ROUTE_DEADLINE_MS, fixtures: new Set() };
  const expired = Promise.withResolvers<never>();
  const timeoutError = () => {
    const message = `route measurement exceeded ${ROUTE_DEADLINE_MS / 1000} s including retries and cleanup`;
    const errors: unknown[] = [...measurement.fixtures].map(
      (data) => new Error(`route fixture cleanup did not finish for ${data}`),
    );
    if (measurement.attemptError !== undefined) errors.unshift(measurement.attemptError);
    return errors.length ? new AggregateError(errors, message) : new Error(message);
  };
  const timer = setTimeout(() => expired.reject(timeoutError()), ROUTE_DEADLINE_MS);
  try {
    const result = await Promise.race([routeAttempts(budget, measurement), expired.promise]);
    if (performance.now() >= measurement.deadline) throw timeoutError();
    return result;
  } finally {
    clearTimeout(timer);
  }
}

/** Release parsers must reject development switches before starting a core or a window. */
function releaseRefusesDevelopmentOptions(): void {
  const scratch = mkdtempSync(join(tmpdir(), 'warden-release-options-'));
  try {
    for (const binary of ['warden-desktop.exe', 'warden.exe']) {
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
    console.log('release: desktop and CLI refuse --no-hooks and --home');
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

async function main(): Promise<void> {
  const budgets = JSON.parse(readFileSync(join(ROOT, 'checks', 'budgets.json'), 'utf8')) as Record<string, Budget>;
  // The desktop window shows the built UI (ui/dist), found beside target/release.
  if (
    !run('cargo', ['build', '--release', '--workspace', '--bins', '--quiet']).ok ||
    !run('cargo', ['build', '--release', '-p', 'warden-server', '--example', 'demo', '--quiet']).ok ||
    !run('npm', ['run', '--silent', 'build', '-w', 'ui']).ok
  ) {
    process.exitCode = 1;
    return;
  }
  releaseRefusesDevelopmentOptions();
  const core = await measureCore();
  const events = await measureEvents();
  // Every second, so a sample's CPU time is measured over many samples; then at the real interval.
  const perSample = await measureSampling(1000);
  const sampling = await measureSampling(10_000);
  const desktop = await measureDesktop();
  // A placeholder budget (null) never fails, so it never needs measuring again.
  const { times: route, excusedAttempts: routeExcusedAttempts } = await measureRoute({
    silent: budgets.routeHookMs?.budget ?? Infinity,
    print: budgets.routeHookPrintMs?.budget ?? Infinity,
  });
  for (const { times, baselineMs, sharedRounds } of routeExcusedAttempts) {
    for (const [mode, shared] of Object.entries(sharedRounds)) {
      console.log(
        `route: excused attempt ${times.attempts} ${mode} with shared slowdown in ${shared}/${ROUTE_RUNS} rounds (hostname.exe baseline ${baselineMs.toFixed(2)} ms)`,
      );
    }
  }
  console.log(
    `route: ${ROUTE_RUNS} runs of each over ${route.seconds.toFixed(0)} s (attempt ${route.attempts}); Windows' hostname.exe started in ${route.control.toFixed(2)} ms (median) in the same rounds`,
  );
  const measured: Record<string, Measured> = {
    coreIdleMemoryMB: core.memory,
    coreIdleCpuPercent: core.cpu,
    windowMemoryMB: desktop.window,
    windowIdleCpuPercent: desktop.windowCpu,
    desktopTrayMemoryMB: desktop.tray,
    desktopTrayCpuPercent: desktop.trayCpu,
    routeHookMs: route.silent,
    routeHookPrintMs: route.print,
    statusHookMs: core.hook,
    statusHookDownMs: core.hookDown,
    statusEventCpuMs: events,
    memorySampleCpuMs: perSample.cpuMs / perSample.samples,
    coreSamplingCpuPercent: (sampling.cpuMs / (SAMPLING_SECONDS * 1000)) * 100,
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
  writeFileSync(
    join(ROOT, 'artifacts', 'budgets.json'),
    JSON.stringify({ measured, budgets, routeExcusedAttempts }, null, 2),
  );
  console.log(`budgets: ${over} over budget; placeholders are reported only`);
  if (over) process.exitCode = 1;
}

if (import.meta.main) await main();
