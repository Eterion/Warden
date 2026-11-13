// Commit messages follow Conventional Commits: `type(scope): summary` (AGENTS.md, Commits).
//
//   node checks/commits.ts                every commit
//   node checks/commits.ts --base <ref>   only the commits since the branch left <ref>

import { git } from './lib/repo.ts';

export const TYPES = ['feat', 'fix', 'docs', 'style', 'refactor', 'perf', 'test', 'build', 'ci', 'chore', 'revert'];

const SUBJECT = new RegExp(`^(?:${TYPES.join('|')})(?:\\([a-z0-9][a-z0-9-]*\\))?!?: \\S`);

/** Why a commit's subject line does not follow Conventional Commits, or `null` when it does. */
export function problem(subject: string): string | null {
  if (SUBJECT.test(subject)) return null;
  const type = /^(\w+)/.exec(subject)?.[1];
  if (type && !TYPES.includes(type) && /^\w+(\([^)]*\))?!?:/.test(subject)) {
    return `unknown type "${type}" (one of ${TYPES.join(', ')})`;
  }
  return 'expected "type(scope): summary", such as "fix(ui): ..." or "docs: ..."';
}

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

function main(): void {
  const base = arg('base');
  const exclude: string[] = [];
  if (base) {
    try {
      exclude.push(`^${git(['merge-base', base, 'HEAD']).trim()}`);
    } catch {
      // A base with no commit yet: check everything since the adoption.
    }
  }
  const log = git(['log', '--no-merges', '--format=%h%x00%s', 'HEAD', ...exclude, '--']).trim();
  const commits = log ? log.split('\n').map((l) => l.split('\0') as [string, string]) : [];
  const bad = commits.flatMap(([hash, subject]) => {
    const why = problem(subject);
    return why ? [`${hash} "${subject}": ${why}`] : [];
  });
  for (const b of bad) console.error(`not a Conventional Commit: ${b}`);
  console.log(
    bad.length
      ? `commits: ${bad.length} of ${commits.length} messages need fixing (amend or reword before review)`
      : `commits: ${commits.length} message(s) follow Conventional Commits`,
  );
  if (bad.length) process.exitCode = 1;
}

if (import.meta.main) main();
