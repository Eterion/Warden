// Runs a Warden reviewer on Codex: a fresh, read-only `codex exec` session that follows the same reviewer
// instructions as the Claude agents (.agents/reviewers/). Codex has no project-level agent files, so this script is
// its equivalent of .claude/agents/.
//
//   npm run review:codex -- code --brief briefs/4.1-core-settings-accounts.md [--base <ref>]
//   npm run review:codex -- ux   --brief briefs/4.1-core-settings-accounts.md [--base <ref>]
//
// Uses whichever Codex account the caller's environment selects (CODEX_HOME); it changes no Codex settings.
// The report is written to artifacts/review/codex-<kind>-reviewer.md.

import { spawn } from 'node:child_process';
import { existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { ROOT, changedFiles } from './lib/repo.ts';
import { reviewKinds, type ReviewKind } from './lib/review.ts';
import { REVIEW_DIR, prepare } from './review-prep.ts';

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

/**
 * The environment without the folders of Windows app execution aliases (`...\WindowsApps`) on PATH. Codex's
 * read-only sandbox starts its shell with a restricted token, which cannot start an alias such as the Store's
 * pwsh.exe ("Access is denied"); without them on PATH, Codex falls back to a shell it can start.
 */
export function withoutAppAliases(env: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
  const out = { ...env };
  for (const key of Object.keys(out).filter((k) => k.toLowerCase() === 'path')) {
    out[key] = (out[key] ?? '')
      .split(';')
      .filter((dir) => !/[\\/]WindowsApps[\\/]?$/i.test(dir))
      .join(';');
  }
  return out;
}

async function main(): Promise<void> {
  const kind = process.argv[2];
  const brief = arg('brief');
  if (!['code', 'ux', 'auto'].includes(kind ?? '') || !brief || !existsSync(join(ROOT, brief))) {
    console.error('usage: node checks/review-codex.ts auto|code|ux --brief briefs/<step>.md [--base <ref>]');
    process.exitCode = 2;
    return;
  }
  const base = arg('base') ?? 'master';
  const kinds = kind === 'auto' ? reviewKinds(changedFiles(base, true).files) : [kind as ReviewKind];
  prepare(base);
  console.log(`review: ${kinds.join(' and ') || 'none (only tests, docs or checks changed)'}`);
  if (process.argv.includes('--dry-run')) return;
  const results = await Promise.all(kinds.map((kind) => runReviewer(kind, brief)));
  if (results.some((status) => status !== 0)) process.exitCode = 1;
}

async function runReviewer(kind: ReviewKind, brief: string): Promise<number> {
  const instructions = `.agents/reviewers/${kind}-reviewer.md`;
  const prompt = [
    `You are Warden's ${kind === 'code' ? 'code' : 'UX'} reviewer.`,
    `Read ${instructions} and follow it exactly. The step's brief is ${brief}.`,
    'The change is in artifacts/review/ (commits.txt, files.txt, diff.patch). The full checks have passed.',
    'You are read-only: do not modify any file. Your final message is the report.',
  ].join('\n');

  const shots = join(ROOT, 'artifacts', 'screenshots');
  const images =
    kind === 'ux' && existsSync(shots)
      ? readdirSync(shots)
          .filter((f) => f.endsWith('.png'))
          // Relative to --cd: cmd.exe takes at most 8191 characters, which 42 states in two themes with full paths pass.
          .flatMap((f) => ['--image', `artifacts/screenshots/${f}`])
      : [];
  const output = join(REVIEW_DIR, `codex-${kind}-reviewer.md`);
  const args = [
    'exec',
    '--sandbox',
    'read-only',
    '--ephemeral',
    '--cd',
    ROOT,
    '--output-last-message',
    output,
    ...images,
    '-',
  ];
  // Through the shell, so an npm-installed codex.cmd is found as well as codex.exe. Every argument is a fixed word
  // or a path inside the repo; each is quoted for cmd.exe.
  const child = spawn(['codex', ...args.map((a) => `"${a}"`)].join(' '), {
    cwd: ROOT,
    env: withoutAppAliases(process.env),
    shell: true,
    stdio: ['pipe', 'inherit', 'inherit'],
  });
  child.stdin.end(prompt);
  return new Promise((resolve) => {
    child.once('error', (error) => {
      console.error(error);
      resolve(1);
    });
    child.once('close', (status) => {
      console.log(`review: Codex ${kind} reviewer ${status === 0 ? 'report' : 'failed; output'} in ${output}`);
      resolve(status ?? 1);
    });
  });
}

if (import.meta.main) await main();
