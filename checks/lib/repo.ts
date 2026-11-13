// Repository helpers shared by the check scripts: the repo root, the files to scan, and running commands.

import { execFileSync, spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');

export interface SourceFile {
  /** Repo-relative path with forward slashes. */
  path: string;
  text: string;
}

const BINARY = /\.(png|jpe?g|gif|ico|webp|woff2?|ttf|otf|exe|dll|pdb|lock)$/i;

/** The most output read from git: a whole-app diff for a review is several megabytes, past Node's 1 MB default. */
export const MAX_OUTPUT = 256 * 1024 * 1024;

export function git(args: string[]): string {
  return execFileSync('git', args, {
    cwd: ROOT,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    maxBuffer: MAX_OUTPUT,
  });
}

/** Tracked and untracked (not ignored) files, as repo-relative paths. */
export function listFiles(): string[] {
  const out = git(['ls-files', '--cached', '--others', '--exclude-standard', '-z']);
  return [...new Set(out.split('\0').filter(Boolean))].sort();
}

/** Reads the text files among `paths`; deleted files and binaries are skipped. */
export function readFiles(paths: string[]): SourceFile[] {
  const files: SourceFile[] = [];
  for (const path of paths) {
    if (BINARY.test(path)) continue;
    try {
      files.push({ path, text: readFileSync(join(ROOT, path), 'utf8') });
    } catch {
      // Deleted in the working tree but still in the index.
    }
  }
  return files;
}

/**
 * Files changed since the branch left `base`. When the default base has no commit yet (a new repo), everything
 * since the first commit counts, which runs more checks, never fewer. An explicit base that does not resolve is an
 * error, so a typo cannot quietly skip checks.
 */
export function changedFiles(base: string, explicit: boolean): { files: string[]; against: string } {
  const paths = (s: string): string[] => s.split('\0').filter(Boolean);
  let from: string;
  let against = base;
  try {
    from = git(['merge-base', base, 'HEAD']).trim();
  } catch {
    if (explicit) throw new Error(`--base ${base}: no common commit with HEAD`);
    from = git(['rev-list', '--max-parents=0', 'HEAD']).trim().split('\n')[0] ?? 'HEAD';
    against = `the first commit (${base} has no commit yet)`;
  }
  const files = [
    ...new Set([
      // Without renames, a moved file counts for the area it left as well as the one it joined.
      ...paths(git(['diff', '--name-only', '--no-renames', '-z', from])),
      ...paths(git(['ls-files', '--others', '--exclude-standard', '-z'])),
    ]),
  ];
  return { files, against };
}

export interface StepResult {
  ok: boolean;
  ms: number;
}

/** Runs a command in the repo (or `cwd`), streaming its output. */
export function run(command: string, args: string[], cwd = ROOT): StepResult {
  const start = performance.now();
  // shell: npm and npx are .cmd files on Windows.
  const result = spawnSync([command, ...args].join(' '), { cwd, stdio: 'inherit', shell: true });
  return { ok: result.status === 0, ms: performance.now() - start };
}
