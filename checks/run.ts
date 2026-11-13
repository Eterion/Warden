// The one command for every check (warden-plan.md, Review, layer 1).
//
//   npm run check           full set on everything: once per step, before review
//   npm run check:changed   full set, only for the areas that changed
//   npm run check:quick     quick set, only for the areas that changed: while working
//
// Flags: --scope all|changed, --level quick|full, --base <git ref> (default: master), --dry-run (print the plan only).
// "Changed" means changed since the branch left the base, including uncommitted and untracked files.
// Quick leaves out the slow checks: rendered accessibility with screenshots, and the performance budgets.

import { spawn } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { ROOT, changedFiles } from './lib/repo.ts';
import { affectsBudgets, isTest } from './lib/selection.ts';

type Area = 'rust' | 'ui' | 'checks';

interface Step {
  name: string;
  area: Area | 'always';
  level: 'quick' | 'full';
  command: string;
  /** Compile Rust tests without a deadline before timing their execution. */
  prepareArgs?: string[];
  timeoutMs?: number;
  /** Fixed, or from the run's scope and base (for checks that look only at what changed themselves). */
  args: string[] | ((run: { scope: 'all' | 'changed'; base: string }) => string[]);
}

/** In the changed scope, a check that narrows itself gets the base. */
const sinceBase = (script: string) => (r: { scope: 'all' | 'changed'; base: string }) =>
  r.scope === 'changed' ? [script, '--base', r.base] : [script];

const STEPS: Step[] = [
  { name: 'rules', area: 'always', level: 'quick', command: 'node', args: ['checks/rules/run.ts'] },
  { name: 'api: doc', area: 'always', level: 'quick', command: 'node', args: ['checks/api-doc.ts'] },
  { name: 'format: prettier', area: 'always', level: 'quick', command: 'node', args: sinceBase('checks/format.ts') },
  { name: 'commits: messages', area: 'always', level: 'full', command: 'node', args: sinceBase('checks/commits.ts') },
  { name: 'checks: self-tests', area: 'checks', level: 'quick', command: 'node', args: ['--test', 'checks/'] },
  { name: 'checks: types', area: 'checks', level: 'quick', command: 'npx', args: ['tsc', '-p', 'checks'] },
  { name: 'rust: fmt', area: 'rust', level: 'quick', command: 'cargo', args: ['fmt', '--all', '--check'] },
  {
    name: 'rust: clippy',
    area: 'rust',
    level: 'quick',
    command: 'cargo',
    args: ['clippy', '--workspace', '--all-targets', '--quiet', '--', '-D', 'warnings'],
  },
  {
    name: 'rust: test',
    area: 'rust',
    level: 'quick',
    command: 'cargo',
    prepareArgs: ['test', '--workspace', '--quiet', '--no-run'],
    args: ['test', '--workspace', '--quiet'],
    timeoutMs: 300_000,
  },
  { name: 'rust: budgets', area: 'rust', level: 'full', command: 'node', args: ['checks/budgets.ts'] },
  { name: 'ui: types', area: 'ui', level: 'quick', command: 'npm', args: ['run', '--silent', 'typecheck', '-w', 'ui'] },
  { name: 'ui: lint', area: 'ui', level: 'quick', command: 'npm', args: ['run', '--silent', 'lint', '-w', 'ui'] },
  {
    name: 'ui: test',
    area: 'ui',
    level: 'quick',
    command: 'npm',
    args: [
      'run',
      '--silent',
      'test',
      '-w',
      'ui',
      '--',
      '--reporter=verbose',
      '--reporter=../checks/vitest-progress.ts',
    ],
    timeoutMs: 300_000,
  },
  { name: 'ui: contrast', area: 'ui', level: 'quick', command: 'node', args: ['checks/contrast.ts'] },
  { name: 'ui: a11y + screenshots', area: 'ui', level: 'full', command: 'node', args: sinceBase('checks/a11y.ts') },
];

/** Which areas a changed path belongs to. Root files shared by both sides count for both. */
export function areasOf(path: string): Area[] {
  if (
    path.startsWith('crates/') ||
    path.startsWith('.cargo/') ||
    /^(Cargo\.(toml|lock)|rustfmt\.toml|clippy\.toml|rust-toolchain(\.toml)?)$/.test(path)
  )
    return ['rust'];
  // Shared by every check script.
  if (path === 'checks/lib/repo.ts') return ['checks', 'rust', 'ui'];
  if (path.startsWith('ui/')) return ['ui'];
  if (path === 'package.json' || path === 'package-lock.json') return ['ui', 'checks'];
  if (path === 'checks/budgets.ts' || path === 'checks/budgets.json') return ['checks', 'rust'];
  // The rendered check's inputs: its pages, the made-up API states they show, and the colour helpers.
  if (
    path === 'checks/a11y.ts' ||
    path === 'checks/lib/pages.ts' ||
    path === 'checks/lib/csp.ts' ||
    path === 'checks/contrast.ts' ||
    path.startsWith('checks/fixtures/') ||
    path.startsWith('checks/lib/colour')
  )
    return ['checks', 'ui'];
  if (path.startsWith('checks/')) return ['checks'];
  return [];
}

export function plan(scope: 'all' | 'changed', level: 'quick' | 'full', changed: string[]): Step[] {
  const areas = new Set<Area>(scope === 'all' ? ['rust', 'ui', 'checks'] : changed.flatMap(areasOf));
  return STEPS.filter((s) => {
    if (level !== 'full' && s.level !== 'quick') return false;
    if (s.name === 'rust: budgets') return scope === 'all' || changed.some(affectsBudgets);
    if (
      s.name === 'ui: a11y + screenshots' &&
      scope === 'changed' &&
      changed.filter((path) => areasOf(path).includes('ui')).every(isTest)
    )
      return false;
    return s.area === 'always' || areas.has(s.area);
  });
}

/** How many of a failed check's last output lines the summary repeats. */
const FAILURE_LINES = 60;

export interface StepRun {
  ok: boolean;
  ms: number;
  /** Everything the check printed, stdout and stderr in the order they came. */
  output: string;
}

/** Compilation is unbounded; only the subsequent test execution gets the test deadline. */
export async function runCheck(step: Step, args: string[]): Promise<StepRun> {
  const prepared = step.prepareArgs ? await runStep(step.command, step.prepareArgs) : null;
  if (prepared && !prepared.ok) return prepared;
  const tested = await runStep(step.command, args, step.timeoutMs);
  return { ok: tested.ok, ms: (prepared?.ms ?? 0) + tested.ms, output: (prepared?.output ?? '') + tested.output };
}

/**
 * Runs one check in the repo, streaming its output as it comes and keeping it, so a failure can be shown again in
 * the summary (a long log piped through `tail` or a filter would otherwise lose it).
 */
export function runStep(command: string, args: string[], timeoutMs?: number): Promise<StepRun> {
  const start = performance.now();
  const chunks: Buffer[] = [];
  // Tools print colour only to a terminal; their output now goes through a pipe, so ask for it when there is one.
  const colour = process.stdout.isTTY ? { FORCE_COLOR: '1', CARGO_TERM_COLOR: 'always' } : {};
  // shell: npm and npx are .cmd files on Windows.
  const child = spawn([command, ...args].join(' '), {
    cwd: ROOT,
    shell: true,
    detached: process.platform !== 'win32',
    stdio: ['inherit', 'pipe', 'pipe'],
    env: { ...process.env, ...colour },
  });
  child.stdout.on('data', (c: Buffer) => {
    chunks.push(c);
    process.stdout.write(c);
  });
  child.stderr.on('data', (c: Buffer) => {
    chunks.push(c);
    process.stderr.write(c);
  });
  return new Promise((resolve) => {
    let timedOut = false;
    let timer: NodeJS.Timeout | undefined;
    let grace: NodeJS.Timeout | undefined;
    let settled = false;
    const done = (ok: boolean, extra = '') => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      clearTimeout(grace);
      // An escaped descendant can keep inherited pipe handles open after our child exits.
      if (timedOut) {
        child.stdout.destroy();
        child.stderr.destroy();
        child.unref();
      }
      resolve({
        ok: ok && !timedOut,
        ms: performance.now() - start,
        output: Buffer.concat(chunks).toString('utf8') + extra,
      });
    };
    if (timeoutMs !== undefined)
      timer = setTimeout(() => {
        timedOut = true;
        const output = Buffer.concat(chunks).toString('utf8');
        const message = `\nTIMEOUT after ${timeoutMs / 1000} s: ${command} ${args.join(' ')}\nRunning tests: ${runningTests(output).join(', ') || 'unknown (no running-test names reported)'}\nLast test output:\n${lastLines(output, 30)}\n`;
        chunks.push(Buffer.from(message));
        process.stderr.write(message);
        // Kill only the tree rooted at the shell we started, including Cargo/Vitest workers.
        // Neither taskkill nor an orphan holding the pipes may extend the deadline indefinitely.
        grace = setTimeout(() => done(false), 1000);
        if (child.exitCode !== null || child.signalCode !== null) {
          done(false);
          return;
        }
        if (process.platform === 'win32' && child.pid !== undefined) {
          const killer = spawn('taskkill', ['/PID', String(child.pid), '/T', '/F'], {
            windowsHide: true,
            stdio: 'ignore',
            timeout: 1000,
          });
          killer.once('error', () => child.kill());
          killer.once('exit', (code) => {
            if (code !== 0) child.kill();
          });
        } else {
          if (child.pid !== undefined) {
            try {
              process.kill(-child.pid, 'SIGKILL');
            } catch {
              child.kill('SIGKILL');
            }
          }
        }
      }, timeoutMs);
    child.on('error', (e) => done(false, `\n${e.message}\n`));
    child.on('exit', () => {
      if (timedOut) done(false);
    });
    child.on('close', (code) => done(code === 0));
  });
}

/** The last `n` lines of a check's output, without trailing blank lines. */
export function lastLines(output: string, n: number): string {
  return output.trimEnd().split(/\r?\n/).slice(-n).join('\n');
}

/** Keep Vitest starts and libtest's slow-test notices even when they have scrolled out of the log tail. */
export function runningTests(output: string): string[] {
  const active = new Set<string>();
  for (const line of output.replace(/\u001b\[[0-9;]*m/g, '').split(/\r?\n/)) {
    if (line.startsWith('RUNNING ')) active.add(line.slice(8));
    else if (line.startsWith('FINISHED ')) active.delete(line.slice(9));
    else {
      const rust = /\btest (.+?) has been running for over \d+ seconds\b/.exec(line);
      if (rust) active.add(`Rust: ${rust[1]}`);
      // Quiet libtest does not print individual successful names. A summary ends that test binary.
      else if (/^test result:/.test(line)) {
        for (const name of active) if (name.startsWith('Rust: ')) active.delete(name);
      }
    }
  }
  return [...active];
}

function arg(name: string, fallback: string): string {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? (process.argv[i + 1] ?? fallback) : fallback;
}

async function main(): Promise<void> {
  const scope = arg('scope', 'all') as 'all' | 'changed';
  const level = arg('level', 'full') as 'quick' | 'full';
  const base = arg('base', 'master');
  if (!['all', 'changed'].includes(scope) || !['quick', 'full'].includes(level)) {
    console.error('usage: node checks/run.ts --scope all|changed --level quick|full [--base <ref>] [--dry-run]');
    process.exitCode = 2;
    return;
  }
  let changed: string[] = [];
  let against = '';
  if (scope === 'changed') {
    try {
      ({ files: changed, against } = changedFiles(base, process.argv.includes('--base')));
    } catch (e) {
      console.error(`checks: ${(e as Error).message}`);
      process.exitCode = 2;
      return;
    }
  }
  const steps = plan(scope, level, changed);
  const skipped = STEPS.filter((s) => !steps.includes(s)).map((s) => s.name);
  console.log(
    `checks: scope ${scope}${scope === 'changed' ? ` (${changed.length} files vs ${against})` : ''}, level ${level}`,
  );
  console.log(`  run:  ${steps.map((s) => s.name).join(', ')}`);
  if (skipped.length) console.log(`  skip: ${skipped.join(', ')}`);
  if (process.argv.includes('--dry-run')) {
    if (steps.some((step) => step.name === 'ui: a11y + screenshots')) {
      const { PAGES } = await import('./a11y.ts');
      const { affectedPages } = await import('./lib/pages.ts');
      const pages = scope === 'all' ? PAGES : affectedPages(PAGES, changed);
      console.log(`  pages: ${pages.map((page) => page.name).join(', ') || '(none)'}`);
    }
    return;
  }

  const results: ({ step: Step } & StepRun)[] = [];
  for (const step of steps) {
    console.log(`\n== ${step.name}`);
    const args = typeof step.args === 'function' ? step.args({ scope, base }) : step.args;
    results.push({ step, ...(await runCheck(step, args)) });
  }

  console.log('\n== summary');
  for (const r of results)
    console.log(`${r.ok ? 'pass' : 'FAIL'}  ${(r.ms / 1000).toFixed(1).padStart(5)} s  ${r.step.name}`);
  const failed = results.filter((r) => !r.ok);
  // Each failure's own output again, last, where a filtered or shortened log still shows it; all of it in a file.
  const logs = join(ROOT, 'artifacts', 'checks');
  for (const r of failed) {
    mkdirSync(logs, { recursive: true });
    const file = join(logs, `${r.step.name.replace(/[^a-z0-9]+/gi, '-')}.log`);
    writeFileSync(file, r.output);
    console.log(`\n== FAILED: ${r.step.name} (the last ${FAILURE_LINES} lines; all of it in ${file})`);
    console.log(lastLines(r.output, FAILURE_LINES) || '(it printed nothing)');
  }
  const total = results.reduce((t, r) => t + r.ms, 0) / 1000;
  console.log(
    failed.length
      ? `\n${failed.length} check(s) failed (${total.toFixed(1)} s)`
      : `\nall ${results.length} checks pass (${total.toFixed(1)} s)`,
  );
  if (failed.length) process.exitCode = 1;
}

if (import.meta.main) await main();
