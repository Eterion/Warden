// Writes the step's change to artifacts/review/ so the read-only reviewer agents can read it without running git:
// commits.txt, files.txt and diff.patch (lock files left out of the patch, untracked files included).
//
//   npm run review:prep [-- --base <ref>]     default base: main

import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { MAX_OUTPUT, ROOT, git } from './lib/repo.ts';
import { reviewKinds } from './lib/review.ts';

const LOCK_FILES = ['package-lock.json', 'Cargo.lock'];
export const REVIEW_DIR = join(ROOT, 'artifacts', 'review');

function untrackedDiff(path: string): string {
  try {
    execFileSync('git', ['diff', '--no-index', '--', '/dev/null', path], {
      cwd: ROOT,
      encoding: 'utf8',
      stdio: 'pipe',
      maxBuffer: MAX_OUTPUT,
    });
    return '';
  } catch (e) {
    // git diff --no-index exits 1 when the files differ, which they always do here.
    return (e as { stdout?: string }).stdout ?? '';
  }
}

export function prepare(base: string): { from: string; files: number } {
  let from: string;
  try {
    from = git(['merge-base', base, 'HEAD']).trim();
  } catch {
    throw new Error(`"${base}" has no commit to compare with; pass --base <ref> (the commit the step started from)`);
  }
  const lines = (s: string): string[] => s.split('\n').filter(Boolean);
  const untracked = lines(git(['ls-files', '--others', '--exclude-standard']));
  const changed = lines(git(['diff', '--name-status', from]));
  const exclude = LOCK_FILES.map((f) => `:(exclude)${f}`);
  const patch =
    git(['diff', '--stat', '--patch', from, '--', '.', ...exclude]) +
    untracked
      .filter((f) => !LOCK_FILES.includes(f))
      .map(untrackedDiff)
      .join('');

  mkdirSync(REVIEW_DIR, { recursive: true });
  writeFileSync(join(REVIEW_DIR, 'commits.txt'), git(['log', '--reverse', '--format=%h %s%n%n%b', `${from}..HEAD`]));
  writeFileSync(
    join(REVIEW_DIR, 'files.txt'),
    [`base ${base} (${from.slice(0, 8)})`, '', ...changed, ...untracked.map((f) => `?\t${f} (uncommitted)`)].join(
      '\n',
    ) + '\n',
  );
  writeFileSync(join(REVIEW_DIR, 'diff.patch'), patch);
  const files = [...lines(git(['diff', '--name-only', '--no-renames', from])), ...untracked];
  writeFileSync(join(REVIEW_DIR, 'plan.json'), JSON.stringify({ reviews: reviewKinds(files) }, null, 2) + '\n');
  return { from, files: changed.length + untracked.length };
}

function main(): void {
  const i = process.argv.indexOf('--base');
  const base = i >= 0 ? (process.argv[i + 1] ?? 'main') : 'main';
  try {
    const { from, files } = prepare(base);
    console.log(`review: ${files} changed files since ${from.slice(0, 8)} written to artifacts/review/`);
  } catch (e) {
    console.error(`review: ${(e as Error).message}`);
    process.exitCode = 1;
  }
}

if (import.meta.main) main();
