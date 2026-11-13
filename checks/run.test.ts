import assert from 'node:assert/strict';
import { describe, it, mock } from 'node:test';
import { lastLines, plan, runCheck, runStep, runningTests } from './run.ts';
import childProcess from 'node:child_process';
import { EventEmitter } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { PassThrough } from 'node:stream';

const names = (scope: 'all' | 'changed', level: 'quick' | 'full', changed: string[]): string[] =>
  plan(scope, level, changed).map((s) => s.name);

describe('run only what changed', () => {
  it('skips Rust checks for a UI-only change', () => {
    const steps = names('changed', 'full', ['ui/src/pages/OverviewPage.vue']);
    assert.ok(steps.includes('ui: lint') && steps.includes('ui: a11y + screenshots'));
    assert.ok(steps.includes('rust: budgets'));
    assert.ok(!steps.includes('rust: clippy') && !steps.includes('rust: test'));
  });

  it('skips UI checks for a Rust-only change', () => {
    const steps = names('changed', 'full', ['crates/warden-core/src/lib.rs']);
    assert.ok(steps.includes('rust: clippy') && steps.includes('rust: budgets'));
    assert.ok(!steps.some((s) => s.startsWith('ui:')));
  });

  it('runs every area when the shared check helpers change', () => {
    const steps = names('changed', 'full', ['checks/lib/repo.ts']);
    assert.ok(
      steps.includes('rust: budgets') && steps.includes('rust: test') && steps.includes('ui: a11y + screenshots'),
    );
  });

  it('runs the rendered UI checks when their made-up API states change', () => {
    const steps = names('changed', 'full', ['checks/fixtures/ui-state.ts']);
    assert.ok(steps.includes('ui: a11y + screenshots'));
  });

  it('runs the rendered UI check when its CSP helper changes', () => {
    assert.ok(names('changed', 'full', ['checks/lib/csp.ts']).includes('ui: a11y + screenshots'));
  });

  it('always runs the rule scripts, the API doc check and Prettier, even for docs', () => {
    assert.deepEqual(names('changed', 'quick', ['AGENTS.md']), ['rules', 'api: doc', 'format: prettier']);
  });

  it('leaves the slow checks out of the quick level', () => {
    const steps = names('all', 'quick', []);
    assert.ok(!steps.includes('rust: budgets') && !steps.includes('ui: a11y + screenshots'));
    assert.ok(steps.includes('rust: test') && steps.includes('ui: test'));
  });

  it('runs everything for the full set', () => {
    assert.equal(names('all', 'full', []).length, 15);
  });

  it('skips budgets for test-only changes, including UI tests and Rust test modules', () => {
    for (const path of [
      'ui/src/settings/GeneralSection.test.ts',
      'crates/warden-core/tests/routing.rs',
      'crates/warden-core/src/tests/routing.rs',
    ])
      assert.ok(!names('changed', 'full', [path]).includes('rust: budgets'), path);
    assert.ok(!names('changed', 'full', ['ui/src/settings/GeneralSection.test.ts']).includes('ui: a11y + screenshots'));
  });

  it('keeps budgets for every production build input', () => {
    for (const path of [
      'ui/src/styles/tokens.css',
      'ui/vite.config.ts',
      'ui/public/licenses/font.txt',
      'Cargo.toml',
      'Cargo.lock',
      'crates/warden-core/Cargo.toml',
      'crates/warden-desktop/tauri.conf.json',
      'crates/warden-desktop/build.rs',
      '.cargo/config.toml',
      'package-lock.json',
      'checks/budgets.ts',
      'checks/budgets.json',
      'checks/lib/pipe.ts',
      'checks/lib/repo.ts',
    ])
      assert.ok(names('changed', 'full', [path]).includes('rust: budgets'), path);
  });
});

describe('a failed check keeps its own output', () => {
  it('returns what the check printed on stdout and stderr, and that it failed', async () => {
    const r = await runStep('node', ['-e', `"console.log('printed'); console.error('the cause'); process.exit(3)"`]);
    assert.equal(r.ok, false);
    assert.match(r.output, /printed/);
    assert.match(r.output, /the cause/);
  });

  it('passes a check that exits with 0', async () => {
    assert.equal((await runStep('node', ['-e', '"process.exit(0)"'])).ok, true);
  });

  it('keeps the last lines', () => {
    assert.equal(lastLines('a\nb\r\nc\n\n', 2), 'b\nc');
  });

  it('reports unfinished tests even after other workers fill the log', () => {
    assert.deepEqual(runningTests('RUNNING stalled\nRUNNING done\nFINISHED done\n' + 'other output\n'.repeat(100)), [
      'stalled',
    ]);
    assert.deepEqual(
      runningTests(
        '....test hangs has been running for over 60 seconds\ntest another has been running for over 60 seconds\n',
      ),
      ['Rust: hangs', 'Rust: another'],
    );
    assert.deepEqual(runningTests('test hangs has been running for over 60 seconds\ntest result: ok.\n'), []);
    assert.deepEqual(runningTests('test first ... ok\ntest hangs ... '), []);
  });

  it('compiles before the deadline, retaining parallel quiet test execution', async () => {
    const step = plan('all', 'full', []).find((step) => step.name === 'rust: test')!;
    assert.deepEqual(step.args, ['test', '--workspace', '--quiet']);
    assert.deepEqual(step.prepareArgs, ['test', '--workspace', '--quiet', '--no-run']);
    const result = await runCheck(
      {
        ...step,
        command: 'node',
        timeoutMs: 500,
        prepareArgs: ['-e', `"setTimeout(() => console.log('compiled'), 750)"`],
      },
      ['-e', `"console.log('ran')"`],
    );
    assert.equal(result.ok, true);
    assert.match(result.output, /compiled[\s\S]*ran/);
  });

  it('does not execute tests after compilation fails', async () => {
    const step = plan('all', 'full', []).find((step) => step.name === 'rust: test')!;
    const result = await runCheck({ ...step, command: 'node', prepareArgs: ['-e', `"process.exit(3)"`] }, [
      '-e',
      `"console.log('should not run')"`,
    ]);
    assert.equal(result.ok, false);
    assert.doesNotMatch(result.output, /should not run/);
  });

  it('fails a hung process and retains the timeout reason', async () => {
    const result = await runStep(
      'node',
      [
        '-e',
        `"const cp = require('node:child_process'); const worker = cp.spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'inherit' }); console.log('TREE', process.pid, worker.pid); console.log('RUNNING deliberately_stalled'); setInterval(() => {}, 1000)"`,
      ],
      500,
    );
    assert.equal(result.ok, false);
    assert.match(result.output, /TIMEOUT/);
    assert.match(result.output, /Running tests: deliberately_stalled/);
    assert.ok(result.ms < 15_000);
    const pids = /TREE (\d+) (\d+)/.exec(result.output);
    assert.ok(pids, 'the child and grandchild started');
    for (const pid of pids.slice(1).map(Number)) assert.throws(() => process.kill(pid, 0), `process ${pid} survived`);
  });

  for (const exited of [true, false]) {
    it(`returns despite orphan-held pipes when the child ${exited ? 'already exited' : 'never emits exit'}`, async () => {
      // Keep both pipes open and never emit close, as an escaped grandchild can do.
      // Model this explicitly: whether Windows inherits pipe handles depends on how the program starts children.
      const stdout = new PassThrough();
      const stderr = new PassThrough();
      const child = Object.assign(new EventEmitter(), {
        stdout,
        stderr,
        exitCode: exited ? 0 : null,
        signalCode: null,
        pid: undefined,
        unref: () => undefined,
      });
      const spawn = mock.method(childProcess, 'spawn', () => child);
      syncBuiltinESMExports();
      let deadline: NodeJS.Timeout | undefined;
      try {
        const result = await Promise.race([
          runStep('orphan-fixture', [], 20),
          new Promise<never>((_, reject) => {
            deadline = setTimeout(() => reject(new Error('orphan kept the runner waiting')), 5000);
          }),
        ]);
        assert.equal(result.ok, false);
        assert.match(result.output, /Running tests: unknown/);
        assert.ok(result.ms < 3000);
        assert.ok(stdout.destroyed && stderr.destroyed, 'both output pipes must be destroyed');
      } finally {
        clearTimeout(deadline);
        spawn.mock.restore();
        syncBuiltinESMExports();
        stdout.destroy();
        stderr.destroy();
      }
    });
  }
});
